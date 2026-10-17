//! Ticket #2617 — Creating a cash transfer validates its lines in `TransferFlow::create_in`:
//! 1. A line with a `denomination_id` that does not exist -> `Err(AppError::NotFound { entity: "denomination", id })`; no `cash_transfers` row written
//! 2. Two lines with the same `denomination_id` -> `Err(AppError::Validation(..))`; no row written
//! 3. A line whose `currency` differs from the denomination's -> `Err(AppError::Validation(..))`; no row written
//! 4. Valid lines (distinct, known, matching currency) -> Transfer created as today (kept)

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::transfer::cash_transfer::{NewTransfer, NewTransferLine, TransferKind};
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

fn thb() -> CurrencyCode {
    CurrencyCode::parse("THB").unwrap()
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
        let schema = unique("tclt");
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

    async fn denomination(&self, currency: &str, face_value: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face_value)
        .fetch_one(&self.pool)
        .await
        .expect("the denomination")
    }

    async fn transfer_count(&self) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM cash_transfers")
            .fetch_one(&self.pool)
            .await
            .expect("count cash_transfers")
    }
}

// ── Row 1: Line with unknown denomination_id answers NotFound and writes no row ──

#[tokio::test]
async fn test_row1_unknown_denomination_id_answers_not_found_and_writes_no_row() {
    let db = Db::fresh().await;
    let b1 = db.branch("B1").await;
    let b2 = db.branch("B2").await;
    let creator = db.person("creator", &[(Role::Finance, Some(&b1))]).await;
    let flow = TransferFlow::pg(db.pool.clone());

    let non_existent_id = 999_999_001i64;
    let cmd = NewTransfer {
        kind: TransferKind::Branch,
        from_branch: b1,
        to_branch: Some(b2),
        from_shift: None,
        to_shift: None,
        lines: vec![NewTransferLine {
            currency: usd(),
            denomination_id: non_existent_id,
            qty: 5,
        }],
        note: None,
        idempotency_key: unique("t1"),
    };

    let res = flow.create(&creator, cmd, at(0)).await;
    match res {
        Err(AppError::NotFound { entity, id }) => {
            assert_eq!(entity, "denomination");
            assert_eq!(id, non_existent_id.to_string());
        }
        other => panic!("expected AppError::NotFound, got: {other:?}"),
    }

    assert_eq!(db.transfer_count().await, 0);
}

// ── Row 2: Two lines with same denomination_id answer Validation and write no row ──

#[tokio::test]
async fn test_row2_duplicate_denomination_id_answers_validation_and_writes_no_row() {
    let db = Db::fresh().await;
    let b1 = db.branch("B1").await;
    let b2 = db.branch("B2").await;
    let creator = db.person("creator", &[(Role::Finance, Some(&b1))]).await;
    let flow = TransferFlow::pg(db.pool.clone());

    let fifty = db.denomination("USD", "50").await;
    let cmd = NewTransfer {
        kind: TransferKind::Branch,
        from_branch: b1,
        to_branch: Some(b2),
        from_shift: None,
        to_shift: None,
        lines: vec![
            NewTransferLine {
                currency: usd(),
                denomination_id: fifty,
                qty: 5,
            },
            NewTransferLine {
                currency: usd(),
                denomination_id: fifty,
                qty: 10,
            },
        ],
        note: None,
        idempotency_key: unique("t2"),
    };

    let res = flow.create(&creator, cmd, at(0)).await;
    match res {
        Err(AppError::Validation(msg)) => {
            assert!(
                msg.contains("duplicate denomination_id"),
                "expected duplicate denomination error message, got: {msg}"
            );
        }
        other => panic!("expected AppError::Validation, got: {other:?}"),
    }

    assert_eq!(db.transfer_count().await, 0);
}

// ── Row 3: Currency mismatch answers Validation and writes no row ─────────────

#[tokio::test]
async fn test_row3_currency_mismatch_answers_validation_and_writes_no_row() {
    let db = Db::fresh().await;
    let b1 = db.branch("B1").await;
    let b2 = db.branch("B2").await;
    let creator = db.person("creator", &[(Role::Finance, Some(&b1))]).await;
    let flow = TransferFlow::pg(db.pool.clone());

    // THB 1000 denomination paired with USD currency
    let thb_1000 = db.denomination("THB", "1000").await;
    let cmd = NewTransfer {
        kind: TransferKind::Branch,
        from_branch: b1,
        to_branch: Some(b2),
        from_shift: None,
        to_shift: None,
        lines: vec![NewTransferLine {
            currency: usd(),
            denomination_id: thb_1000,
            qty: 2,
        }],
        note: None,
        idempotency_key: unique("t3"),
    };

    let res = flow.create(&creator, cmd, at(0)).await;
    match res {
        Err(AppError::Validation(msg)) => {
            assert!(
                msg.contains("does not match denomination currency"),
                "expected currency mismatch error message, got: {msg}"
            );
        }
        other => panic!("expected AppError::Validation, got: {other:?}"),
    }

    assert_eq!(db.transfer_count().await, 0);
}

// ── Row 4: Valid lines create transfer as today (kept) ─────────────────────────

#[tokio::test]
async fn test_row4_valid_lines_create_transfer() {
    let db = Db::fresh().await;
    let b1 = db.branch("B1").await;
    let b2 = db.branch("B2").await;
    let creator = db.person("creator", &[(Role::Finance, Some(&b1))]).await;
    let flow = TransferFlow::pg(db.pool.clone());

    let fifty = db.denomination("USD", "50").await;
    let hundred = db.denomination("USD", "100").await;
    let thb_1000 = db.denomination("THB", "1000").await;

    let cmd = NewTransfer {
        kind: TransferKind::Branch,
        from_branch: b1,
        to_branch: Some(b2),
        from_shift: None,
        to_shift: None,
        lines: vec![
            NewTransferLine {
                currency: usd(),
                denomination_id: fifty,
                qty: 5,
            },
            NewTransferLine {
                currency: usd(),
                denomination_id: hundred,
                qty: 2,
            },
            NewTransferLine {
                currency: thb(),
                denomination_id: thb_1000,
                qty: 10,
            },
        ],
        note: Some("normal transfer".to_string()),
        idempotency_key: unique("t4"),
    };

    let transfer = flow
        .create(&creator, cmd, at(0))
        .await
        .expect("create transfer succeeds");
    assert_eq!(transfer.lines.len(), 3);
    assert_eq!(db.transfer_count().await, 1);
}

// ── Edge Case §9: Idempotent replay answers before line checks ────────────────

#[tokio::test]
async fn test_edge_case_idempotent_replay_answers_before_line_checks() {
    let db = Db::fresh().await;
    let b1 = db.branch("B1").await;
    let b2 = db.branch("B2").await;
    let creator = db.person("creator", &[(Role::Finance, Some(&b1))]).await;
    let flow = TransferFlow::pg(db.pool.clone());

    let fifty = db.denomination("USD", "50").await;
    let ikey = unique("idem");

    let cmd = NewTransfer {
        kind: TransferKind::Branch,
        from_branch: b1.clone(),
        to_branch: Some(b2.clone()),
        from_shift: None,
        to_shift: None,
        lines: vec![NewTransferLine {
            currency: usd(),
            denomination_id: fifty,
            qty: 5,
        }],
        note: None,
        idempotency_key: ikey.clone(),
    };

    let first = flow
        .create(&creator, cmd, at(0))
        .await
        .expect("first create succeeds");

    // Second call with same idempotency_key but invalid lines (e.g. non-existent denomination)
    // must return the existing transfer without failing on line checks.
    let replay_cmd = NewTransfer {
        kind: TransferKind::Branch,
        from_branch: b1,
        to_branch: Some(b2),
        from_shift: None,
        to_shift: None,
        lines: vec![NewTransferLine {
            currency: usd(),
            denomination_id: 999_999_999,
            qty: 5,
        }],
        note: None,
        idempotency_key: ikey,
    };

    let second = flow
        .create(&creator, replay_cmd, at(5))
        .await
        .expect("replay succeeds");
    assert_eq!(first.id, second.id);
    assert_eq!(db.transfer_count().await, 1);
}
