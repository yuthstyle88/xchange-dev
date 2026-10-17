//! Ticket #2615 — `PgTransferRepository::find_by_id` and `find_by_id_in` return `Ok(None)`
//! for non-UUID strings rather than erroring with Postgres 22P02 `invalid_text_representation`.
//!
//! Behaviour table:
//! 1. `PgTransferRepository::find_by_id("abc")` -> `Ok(None)`
//! 2. `find_by_id_in(tx, "abc")` -> `Ok(None)`, and the transaction is still usable afterwards
//! 3. A random well-formed UUID -> `Ok(None)`
//! 4. A real transfer's id -> `Ok(Some(transfer))` with its lines

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::transfer::cash_transfer::{NewTransfer, NewTransferLine, TransferKind};
use xc_cash::domain::transfer::transfer_repository::TransferRepository;
use xc_cash::infrastructure::transfer_repository::PgTransferRepository;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

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

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
}

fn api_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Db {
    pool: PgPool,
}

impl Db {
    async fn fresh() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip."
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("tfnt");
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
            .max_connections(6)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        Migrator::new(api_dir().join("migrations").as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("every migration applies");
        Db { pool }
    }

    async fn branch(&self, label: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .unwrap();
        BranchId::new(id)
    }

    async fn person(&self, name: &str, roles: &[(Role, Option<&BranchId>)]) -> Actor {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        for (role, branch) in roles {
            sqlx::query(
                "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) VALUES ($1::uuid, $2, $3::uuid, $4)",
            )
            .bind(&id)
            .bind(role.as_str())
            .bind(ORG)
            .bind(branch.map(|b| b.as_str().to_owned()))
            .execute(&self.pool)
            .await
            .expect("give the user a role");
        }
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Actor::new(
            &id,
            ORG,
            roles
                .iter()
                .map(|(r, b)| ScopedRole::new(*r, b.cloned(), from, None))
                .collect(),
            None,
        )
    }

    async fn fifty(&self) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = 'USD' AND face_value = 50::numeric",
        )
        .fetch_one(&self.pool)
        .await
        .expect("the USD 50 face value")
    }
}

// ── Row 1: find_by_id with non-UUID returns Ok(None) ────────────────────────

#[tokio::test]
async fn test_row1_find_by_id_non_uuid_returns_none() {
    let db = Db::fresh().await;
    let repo = PgTransferRepository::new(db.pool.clone());
    let res = repo.find_by_id("abc").await;
    assert!(
        matches!(res, Ok(None)),
        "find_by_id(\"abc\") must be Ok(None), got: {res:?}"
    );
}

// ── Row 2: find_by_id_in with non-UUID returns Ok(None) and tx usable ────────

#[tokio::test]
async fn test_row2_find_by_id_in_non_uuid_returns_none_and_tx_usable() {
    let db = Db::fresh().await;
    let repo = PgTransferRepository::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin transaction");
    let res = repo.find_by_id_in(&mut tx, "abc").await;
    assert!(
        matches!(res, Ok(None)),
        "find_by_id_in(tx, \"abc\") must be Ok(None), got: {res:?}"
    );

    // Verify transaction remains usable (no aborted transaction error)
    let probe: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&mut *tx)
        .await
        .expect("transaction must remain usable after non-UUID find_by_id_in");
    assert_eq!(probe, 1);
    tx.commit().await.expect("commit transaction");
}

// ── Row 3: random well-formed UUID returns Ok(None) ─────────────────────────

#[tokio::test]
async fn test_row3_find_by_id_random_uuid_returns_none() {
    let db = Db::fresh().await;
    let repo = PgTransferRepository::new(db.pool.clone());
    let random_id = Uuid::new_v4().to_string();

    let res = repo.find_by_id(&random_id).await;
    assert!(
        matches!(res, Ok(None)),
        "find_by_id(random_uuid) must be Ok(None), got: {res:?}"
    );

    let mut tx = db.pool.begin().await.expect("begin transaction");
    let res_in = repo.find_by_id_in(&mut tx, &random_id).await;
    assert!(
        matches!(res_in, Ok(None)),
        "find_by_id_in(random_uuid) must be Ok(None), got: {res_in:?}"
    );
    tx.rollback().await.expect("rollback transaction");
}

// ── Row 4: real transfer id returns Ok(Some(transfer)) with lines ────────────

#[tokio::test]
async fn test_row4_find_by_id_real_transfer_returns_some_with_lines() {
    let db = Db::fresh().await;
    let repo = PgTransferRepository::new(db.pool.clone());
    let b1 = db.branch("B1").await;
    let b2 = db.branch("B2").await;
    let creator = db.person("creator", &[(Role::Finance, Some(&b1))]).await;

    let flow = TransferFlow::pg(db.pool.clone());
    let transfer = flow
        .create(
            &creator,
            NewTransfer {
                kind: TransferKind::Branch,
                from_branch: b1.clone(),
                to_branch: Some(b2.clone()),
                from_shift: None,
                to_shift: None,
                lines: vec![NewTransferLine {
                    currency: usd(),
                    denomination_id: db.fifty().await,
                    qty: 5,
                }],
                note: Some("rebalancing".to_string()),
                idempotency_key: unique("tf"),
            },
            at(0),
        )
        .await
        .expect("create transfer");

    // Check find_by_id with exact id
    let found = repo
        .find_by_id(&transfer.id)
        .await
        .expect("find_by_id succeeds")
        .expect("transfer exists");
    assert_eq!(found.id, transfer.id);
    assert_eq!(found.lines.len(), 1);
    assert_eq!(found.lines[0].currency, usd());
    assert_eq!(found.lines[0].qty_sent, 5);

    // Check find_by_id with uppercase UUID string (edge case §9)
    let found_upper = repo
        .find_by_id(&transfer.id.to_uppercase())
        .await
        .expect("find_by_id with uppercase UUID succeeds")
        .expect("transfer exists");
    assert_eq!(found_upper.id, transfer.id);

    // Check find_by_id_in with transaction
    let mut tx = db.pool.begin().await.expect("begin transaction");
    let found_in = repo
        .find_by_id_in(&mut tx, &transfer.id)
        .await
        .expect("find_by_id_in succeeds")
        .expect("transfer exists in tx");
    assert_eq!(found_in.id, transfer.id);
    assert_eq!(found_in.lines.len(), 1);
    assert_eq!(found_in.lines[0].currency, usd());
    assert_eq!(found_in.lines[0].qty_sent, 5);
    tx.rollback().await.expect("rollback transaction");
}
