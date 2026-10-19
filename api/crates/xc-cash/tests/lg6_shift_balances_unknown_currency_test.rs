//! LG-6 setup S7 (#3214): nothing pins that an unknown currency code in the ledger's shift
//! balances is a fault — add the xc-cash test.
//!
//! Pinning the guard in `shift_balances.rs`:
//! A shift whose DRAWER account holds a code not in currencies (e.g. ZZZ, written with
//! `SET LOCAL session_replication_role = replica` so the FK is bypassed), with one ledger entry
//! on it, fails with `AppError::Internal("invalid currency code: ZZZ")`.
//! The same shift with only known codes succeeds.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pos_error::AppError;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xc_cash::application::count::count_service::post_opening_in;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger::shift_balances::PgLedgerShiftBalances;
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
use xc_shift::domain::shift::repositories::shift_balances::ShiftBalances;

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

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

async fn db() -> (PgPool, BranchId) {
    let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip. \
             Point it at an existing database whose name ends in `_test`."
        )
    });
    let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
    assert!(
        options.get_database().unwrap_or("").ends_with("_test"),
        "TEST_DATABASE_URL must name a database ending in `_test`"
    );
    let schema = unique("lg6s7");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .expect("connect to the test database");
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("create the test schema");
    admin.close().await;

    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .expect("connect to the test schema");
    Migrator::new(migrations_dir().as_path())
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("every migration applies in version order on a fresh database");

    let branch = unique("B");
    sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
        .bind(&branch)
        .execute(&pool)
        .await
        .expect("insert a branch");
    sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
        .bind(&branch)
        .execute(&pool)
        .await
        .expect("insert ops_state");

    (pool, BranchId::new(branch))
}

async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2::uuid, 'Test Teller', 'ACTIVE', 'test')",
    )
    .bind(id)
    .bind(ORG)
    .execute(pool)
    .await
    .expect("insert user");
    id
}

async fn next_shift(pool: &PgPool, branch: &BranchId) -> String {
    let teller_id = seed_user(pool).await;
    let shift_id = unique("shift");
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         VALUES ($1, $2, 'OPEN', now(), 'system', $3, $4)",
    )
    .bind(&shift_id)
    .bind(branch.as_str())
    .bind(unique("open-req"))
    .bind(teller_id)
    .execute(pool)
    .await
    .expect("insert shift");
    shift_id
}

async fn fund_and_open_drawer(pool: &PgPool, branch: &BranchId, shift: &str) {
    let ledger = LedgerCashPostings::new();
    let usd = CurrencyCode::parse("USD").expect("USD code");

    let s = SourceRef {
        source_type: SourceType::Opening,
        source_id: unique("fund"),
        source_key: "fund".to_string(),
    };
    let mut tx = pool.begin().await.expect("begin");
    ledger
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Bank,
                to: CashPlace::Safe(branch.clone()),
                currency: usd,
                quantity: ForeignAmount::from_decimal(Decimal::from(100)),
                source: s,
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("fund safe");
    tx.commit().await.expect("commit");

    let mut tx = pool.begin().await.expect("begin");
    post_opening_in(
        &ledger,
        &mut tx,
        CashPlace::Safe(branch.clone()),
        shift,
        &[(usd, Decimal::from(100))],
        &unique("open"),
    )
    .await
    .expect("open drawer");
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn a_shift_drawer_account_with_unknown_currency_code_is_a_fault() {
    let (pool, branch) = db().await;
    let shift = next_shift(&pool, &branch).await;
    fund_and_open_drawer(&pool, &branch, &shift).await;

    // Corrupt DRAWER account's currency to 'ZZZ', bypassing the FK under session_replication_role = replica.
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query("SET LOCAL session_replication_role = 'replica'")
        .execute(&mut *tx)
        .await
        .expect("set replica role");
    sqlx::query(
        "UPDATE cash_accounts SET currency = 'ZZZ' WHERE shift_id = $1 AND kind = 'DRAWER' AND currency = 'USD'",
    )
    .bind(&shift)
    .execute(&mut *tx)
    .await
    .expect("corrupt currency to ZZZ");
    tx.commit().await.expect("commit");

    let balances = PgLedgerShiftBalances::new(pool);
    let result = balances.shift_balances(&shift).await;
    match result {
        Err(AppError::Internal(msg)) => {
            assert!(
                msg.contains("ZZZ"),
                "expected error message to contain 'ZZZ', got: {msg}"
            );
        }
        other => panic!("expected Err(AppError::Internal(...)) containing 'ZZZ', got: {other:?}"),
    }
}

#[tokio::test]
async fn a_shift_drawer_account_with_known_currency_codes_succeeds() {
    let (pool, branch) = db().await;
    let shift = next_shift(&pool, &branch).await;
    fund_and_open_drawer(&pool, &branch, &shift).await;

    let balances = PgLedgerShiftBalances::new(pool);
    let rows = balances
        .shift_balances(&shift)
        .await
        .expect("shift balances with known codes should succeed");

    assert_eq!(rows.len(), 1, "control: expected one row per currency");
    assert_eq!(rows[0].currency, CurrencyCode::parse("USD").unwrap());
}
