//! Tests for denomination face value lookups (ticket #5041).
//!
//! A denomination with a NULL face_value is refused with NotFound { entity: "denomination", id },
//! exactly as an unknown denomination ID.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::{HandoverService, StartHandover};
use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::count::{CountKind, CountLine};
use xc_cash::domain::transfer::cash_transfer::{NewTransfer, NewTransferLine, TransferKind};
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
static LG4_FUND: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

async fn lg4_fund_safe(pool: &PgPool, branch: &str, currency: &str, amount: &str) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
    let mut tx = pool.begin().await.expect("begin");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Bank,
                to: CashPlace::Safe(BranchId::new(branch)),
                currency: xc_kernel::money::CurrencyCode::parse(currency).expect("a currency"),
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: format!(
                        "lg4-fund:{branch}:{currency}:{}",
                        LG4_FUND.fetch_add(1, Ordering::Relaxed)
                    ),
                    source_key: "fund".to_string(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("fund safe");
    tx.commit().await.expect("commit");
}

struct Db {
    pool: PgPool,
    schema: String,
}

impl Db {
    async fn fresh() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set. Point it at an existing database ending in `_test`.");
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("dfv5041");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect to admin");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to schema pool");
        Migrator::new(migrations_dir().as_path())
            .await
            .expect("read migrations")
            .run(&pool)
            .await
            .expect("apply migrations");

        Db { pool, schema }
    }

    async fn drop_schema(self) {
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {} CASCADE", self.schema)))
            .execute(&self.pool)
            .await
            .expect("drop schema");
    }

    async fn branch(&self, label: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert branch");
        lg4_fund_safe(&self.pool, &id, "THB", "1000000.00").await;
        lg4_fund_safe(&self.pool, &id, "USD", "10000.00").await;
        BranchId::new(id)
    }

    async fn person(&self, name: &str, roles: &[Role]) -> Actor {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .expect("insert user");

        for role in roles {
            sqlx::query(
                "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
                 VALUES ($1::uuid, $2, $3::uuid, NULL, '2026-01-01T00:00:00Z')",
            )
            .bind(&id)
            .bind(role.as_str())
            .bind(ORG)
            .execute(&self.pool)
            .await
            .expect("insert role");
        }
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Actor::new(
            &id,
            ORG,
            roles
                .iter()
                .map(|r| ScopedRole::new(*r, None, from, None))
                .collect(),
            None,
        )
    }

    async fn denomination(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("denomination {currency} {face}: {e}"))
    }

    async fn null_face_denomination(&self, currency: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value IS NULL LIMIT 1",
        )
        .bind(currency)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("null face denomination for {currency}: {e}"))
    }

    fn counts(&self) -> CashCountService {
        CashCountService::pg(self.pool.clone())
    }

    fn handovers(&self) -> HandoverService {
        HandoverService::pg(self.pool.clone())
    }

    async fn handover_rows(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM handovers")
            .fetch_one(&self.pool)
            .await
            .expect("count handovers")
    }

    async fn transfer_count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM cash_transfers")
            .fetch_one(&self.pool)
            .await
            .expect("count cash_transfers")
    }
}

struct Floor {
    db: Db,
    main: BranchId,
    a: Actor,
    b: Actor,
    a_shift: String,
}

async fn floor() -> Floor {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let a = db.person("Ploy", &[Role::Teller]).await;
    let b = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    let lines = vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("USD", db.denomination("USD", "100").await, 3),
    ];
    let counts = db.counts();
    let open = counts
        .submit(
            &a,
            SubmitCount {
                branch: main.clone(),
                shift_id: None,
                kind: CountKind::Open,
                lines,
                note: None,
                idempotency_key: unique("open"),
            },
            at(0),
        )
        .await
        .expect("submit OPEN count");
    let a_shift = counts
        .verify(&manager, &open.id, at(60))
        .await
        .expect("verify OPEN count")
        .shift_id
        .expect("shift id");

    Floor {
        db,
        main,
        a,
        b,
        a_shift,
    }
}

fn start_cmd(f: &Floor, lines: Vec<CountLine>) -> StartHandover {
    StartHandover {
        branch: f.main.clone(),
        from_shift_id: f.a_shift.clone(),
        receiver_id: f.b.user_id.clone(),
        lines,
        note: None,
        idempotency_key: unique("handover"),
    }
}

// ── N1: Hand-over start with a line naming a NULL-face denomination ─────────

#[tokio::test]
async fn n1_handover_start_with_null_face_denomination_answers_not_found() {
    let f = floor().await;
    let null_denom_id = f.db.null_face_denomination("THB").await;
    let lines = vec![CountLine::new("THB", null_denom_id, 1)];

    let res = f
        .db
        .handovers()
        .start(&f.a, start_cmd(&f, lines), at(600))
        .await;

    match res {
        Err(AppError::NotFound { entity, id }) => {
            assert_eq!(entity, "denomination");
            assert_eq!(id, null_denom_id.to_string());
        }
        other => panic!("expected AppError::NotFound for {null_denom_id}, got {other:?}"),
    }
    assert_eq!(f.db.handover_rows().await, 0);
    f.db.drop_schema().await;
}

// ── N2: Transfer create with a line naming a NULL-face denomination ─────────

#[tokio::test]
async fn n2_transfer_create_with_null_face_denomination_answers_not_found() {
    let db = Db::fresh().await;
    let b1 = db.branch("B1").await;
    let b2 = db.branch("B2").await;
    let creator = db.person("creator", &[Role::Finance]).await;
    let flow = TransferFlow::pg(db.pool.clone());

    let null_denom_id = db.null_face_denomination("USD").await;
    let cmd = NewTransfer {
        kind: TransferKind::Branch,
        from_branch: b1,
        to_branch: Some(b2),
        from_shift: None,
        to_shift: None,
        lines: vec![NewTransferLine {
            currency: usd(),
            denomination_id: null_denom_id,
            qty: 5,
        }],
        note: None,
        idempotency_key: unique("t1"),
    };

    let res = flow.create(&creator, cmd, at(0)).await;
    match res {
        Err(AppError::NotFound { entity, id }) => {
            assert_eq!(entity, "denomination");
            assert_eq!(id, null_denom_id.to_string());
        }
        other => panic!("expected AppError::NotFound for {null_denom_id}, got: {other:?}"),
    }
    assert_eq!(db.transfer_count().await, 0);
    db.drop_schema().await;
}

// ── N3: Real denominations succeed as today ─────────────────────────────────

#[tokio::test]
async fn n3_real_denomination_succeeds_as_today() {
    let f = floor().await;
    let thb_1000 = f.db.denomination("THB", "1000").await;
    let lines = vec![CountLine::new("THB", thb_1000, 2)];

    let h = f
        .db
        .handovers()
        .start(&f.a, start_cmd(&f, lines), at(600))
        .await
        .expect("handover start with real denomination succeeds");

    assert_eq!(f.db.handover_rows().await, 1);
    assert_eq!(h.status, xc_cash::domain::handover::HandoverStatus::Pending);
    f.db.drop_schema().await;
}
