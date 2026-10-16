//! A stored face value that `Money` cannot hold must stop the transfer read, never turn into 0.
//!
//! Written before the fix by claude/d7aa64 (ticket author). Do not edit, delete or weaken these tests; add your own in
//! another file.
//!
//! `denominations.face_value` is `numeric(20,4)` (`api/migrations/20261010100000_rate_units.sql:28`), while `Money`
//! holds at most two decimals and at most `Money::MAX_INPUT_UNITS` (100,000,000) units
//! (`api/crates/xc-kernel/src/money/money.rs:31-38`). `PgTransferRepository`'s line mapper reads the column as
//! `Money::parse(..).unwrap_or(Money::ZERO)` (`api/crates/xc-cash/src/infrastructure/transfer_repository.rs:63`), so
//! a face value of 100,000,000.01 or 1.0001 loads as a line worth 0, and every total built on it (the transfer
//! policy's value, the dispatch and receipt postings) is silently short.
//!
//! [required] by these tests:
//! - `PgTransferRepository::find_by_id` and `find_by_id_in` return `Err(AppError::Internal(_))` (a 500, detail in
//!   the log) when any line's stored face value is not a valid `Money`. They never return the transfer with a zero
//!   face value.
//! - A face value `Money` can hold still loads unchanged (`1000.0000` → `1000.00`).
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips. Each test gets its own
//! schema with every migration applied.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pos_error::AppError;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_cash::domain::transfer::cash_transfer::CashTransfer;
use xc_cash::domain::transfer::transfer_repository::TransferRepository;
use xc_cash::infrastructure::PgTransferRepository;
use xc_kernel::money::Money;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn api_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

async fn fresh_pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!("TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip.")
    });
    let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
    assert!(
        options.get_database().unwrap_or("").ends_with("_test"),
        "TEST_DATABASE_URL must name a database ending in `_test`"
    );
    let schema = unique("fvread");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .expect("connect");
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("schema");
    admin.close().await;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .expect("connect to the test schema");
    Migrator::new(api_dir().join("migrations").as_path())
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("every migration applies");
    pool
}

/// Stores one EXTERNAL_IN transfer with a single USD line of `qty` notes whose denomination has `face_value` (the
/// literal is written straight into `numeric(20,4)`, as a bad row would be). Returns the transfer id.
async fn stored_transfer(pool: &PgPool, face_value: &str, qty: i32) -> String {
    let branch = unique("b");
    sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
        .bind(&branch)
        .execute(pool)
        .await
        .expect("a branch");
    let user: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, 'teller', 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(ORG)
    .fetch_one(pool)
    .await
    .expect("a user");
    let denomination: i64 = sqlx::query_scalar(AssertSqlSafe(format!(
        "INSERT INTO denominations (currency_code, face_value, group_code) \
         VALUES ('USD', {face_value}::numeric, 'ALL') RETURNING id"
    )))
    .fetch_one(pool)
    .await
    .expect("a denomination with this face value");
    let id: String = sqlx::query_scalar(
        "INSERT INTO cash_transfers (organization_id, kind, from_branch_id, status, created_by, idempotency_key) \
         VALUES ($1::uuid, 'EXTERNAL_IN', $2, 'DRAFT', $3::uuid, $4) RETURNING id::text",
    )
    .bind(ORG)
    .bind(&branch)
    .bind(&user)
    .bind(unique("idem"))
    .fetch_one(pool)
    .await
    .expect("a transfer");
    sqlx::query(
        "INSERT INTO cash_transfer_lines (transfer_id, organization_id, currency, denomination_id, qty_sent) \
         VALUES ($1::uuid, $2::uuid, 'USD', $3, $4)",
    )
    .bind(&id)
    .bind(ORG)
    .bind(denomination)
    .bind(qty)
    .execute(pool)
    .await
    .expect("a transfer line");
    id
}

/// Both read paths (pool and caller's transaction) for the same id.
async fn read_both(
    pool: &PgPool,
    id: &str,
) -> (
    Result<Option<CashTransfer>, AppError>,
    Result<Option<CashTransfer>, AppError>,
) {
    let repo = PgTransferRepository::new(pool.clone());
    let on_pool = repo.find_by_id(id).await;
    let mut tx = pool.begin().await.expect("begin");
    let in_tx = repo.find_by_id_in(&mut tx, id).await;
    tx.rollback().await.expect("rollback");
    (on_pool, in_tx)
}

fn assert_refused(what: &str, got: Result<Option<CashTransfer>, AppError>) {
    match got {
        Err(AppError::Internal(_)) => {}
        Ok(Some(t)) => panic!(
            "{what}: the read must refuse a face value Money cannot hold, but it loaded the transfer with \
             face values {:?} (a bad stored value became a number)",
            t.lines
                .iter()
                .map(|l| l.face_value.to_string())
                .collect::<Vec<_>>()
        ),
        other => panic!("{what}: expected Err(AppError::Internal(_)), got {other:?}"),
    }
}

// ── row 1: past Money's range ───────────────────────────────────────────────

#[tokio::test]
async fn a_face_value_past_100_million_units_is_refused_not_read_as_zero() {
    let pool = fresh_pool().await;
    let id = stored_transfer(&pool, "100000000.01", 1).await;
    let (on_pool, in_tx) = read_both(&pool, &id).await;
    assert_refused("find_by_id", on_pool);
    assert_refused("find_by_id_in", in_tx);
}

// ── row 2: more decimals than Money holds ───────────────────────────────────

#[tokio::test]
async fn a_face_value_with_four_decimals_is_refused_not_read_as_zero() {
    let pool = fresh_pool().await;
    let id = stored_transfer(&pool, "1.0001", 3).await;
    let (on_pool, in_tx) = read_both(&pool, &id).await;
    assert_refused("find_by_id", on_pool);
    assert_refused("find_by_id_in", in_tx);
}

// ── row 3 (keep): a face value Money holds reads unchanged ──────────────────

#[tokio::test]
async fn a_normal_face_value_still_reads_unchanged() {
    let pool = fresh_pool().await;
    let id = stored_transfer(&pool, "1000.0000", 2).await;
    let (on_pool, in_tx) = read_both(&pool, &id).await;
    for (what, got) in [("find_by_id", on_pool), ("find_by_id_in", in_tx)] {
        let t = got
            .unwrap_or_else(|e| panic!("{what}: a valid face value must load, got {e:?}"))
            .unwrap_or_else(|| panic!("{what}: the stored transfer must be found"));
        assert_eq!(t.lines.len(), 1, "{what}: one line");
        assert_eq!(
            t.lines[0].face_value,
            Money::parse("1000.00").unwrap(),
            "{what}: face value"
        );
        assert_eq!(t.lines[0].qty_sent, 2, "{what}: qty");
    }
}

// ── row 3b (keep): the largest face value Money holds reads unchanged ───────

#[tokio::test]
async fn the_largest_face_value_money_holds_still_reads() {
    let pool = fresh_pool().await;
    let id = stored_transfer(&pool, "100000000.00", 1).await;
    let (on_pool, _) = read_both(&pool, &id).await;
    let t = on_pool
        .unwrap_or_else(|e| panic!("100,000,000.00 is within Money's range, got {e:?}"))
        .expect("found");
    assert_eq!(
        t.lines[0].face_value,
        Money::parse("100000000").unwrap(),
        "face value"
    );
}
