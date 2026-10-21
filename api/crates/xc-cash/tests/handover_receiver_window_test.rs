//! Up-front tests for #2846 (merged with #2879): `HandoverService::start_in` must accept as receiver exactly the people
//! who hold `ShiftOperate` at the branch **at `now`** — the same rule `handoverReceivers` and `accept_in` apply.
//!
//! Written before the fix by the ticket writer claude/dell-claude-4, on the BE controller claude/56a74fcd's order.
//! Do not edit, delete or weaken these tests; add your own in another file.
//!
//! **The bug** (main `2d5ecfde`): the receiver `SELECT EXISTS` in `start_in`
//! (`api/crates/xc-cash/src/application/handover/handover_service.rs:142-163`) checks `r.valid_to` but not
//! `r.valid_from`, so a user whose role starts later gets a PENDING handover that nobody can accept until then.
//!
//! | Row | Receiver at `now` | Test | Today |
//! |---|---|---|---|
//! | R1 | TELLER of the branch, role starts tomorrow | `r1_role_starting_after_now_is_refused` | red |
//! | R2 | BRANCH_MANAGER org-wide, started yesterday | `r2_org_wide_manager_started_yesterday_is_pending` | green (kept) |
//! | R3 | TELLER of the branch, role starts exactly at `now` | `r3_role_starting_exactly_now_is_pending` | green (kept) |
//! | R4 | TELLER of the branch, role ended exactly at `now` | `r4_role_ended_exactly_now_is_refused` | green (kept) |
//! | R5 | FINANCE only, org-wide (no `ShiftOperate`) | `r5_role_without_shift_operate_is_refused` | green (kept) |
//! | R6 | TELLER of another branch only | `r6_teller_of_another_branch_is_refused` | green (kept) |
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing skips.

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
use xc_cash::domain::count::{CountKind, CountLine};
use xc_cash::domain::handover::HandoverStatus;
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

static LG4_FUND: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
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
        .expect("fund the safe through the ledger");
    tx.commit().await.expect("commit");
}

struct Db {
    pool: PgPool,
    schema: String,
}

impl Db {
    async fn fresh() -> Db {
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
        let schema = unique("ho1935");
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
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply migrations");
        Db { pool, schema }
    }

    async fn drop_schema(self) {
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.pool)
        .await
        .expect("drop test schema");
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
        lg4_fund_safe(&self.pool, &id, "JPY", "100000").await;
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
                "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
                 VALUES ($1::uuid, $2, $3::uuid, NULL)",
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
}

struct Floor {
    db: Db,
    main: BranchId,
    a: Actor,
    a_shift: String,
}

async fn floor() -> Floor {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let a = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    let lines = vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 3),
        CountLine::new("USD", db.denomination("USD", "100").await, 3),
        CountLine::new("JPY", db.denomination("JPY", "10000").await, 2),
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
        a_shift,
    }
}

use xc_cash::application::handover::handover_service::INVALID_RECEIVER;

/// A receiver with one role row whose window is given explicitly (the shared fixtures leave `valid_from` to the
/// database default `now()`, the wall clock, which is after the fixed test clock `at()`).
async fn receiver(
    db: &Db,
    role: Role,
    branch: Option<&BranchId>,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
) -> String {
    let id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(ORG)
    .bind(unique("receiver"))
    .fetch_one(&db.pool)
    .await
    .expect("insert receiver");
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from, valid_to) \
         VALUES ($1::uuid, $2, $3::uuid, $4, $5, $6)",
    )
    .bind(&id)
    .bind(role.as_str())
    .bind(ORG)
    .bind(branch.map(|b| b.as_str().to_owned()))
    .bind(valid_from)
    .bind(valid_to)
    .execute(&db.pool)
    .await
    .expect("insert receiver role");
    id
}

const NOW: i64 = 600;
const DAY: i64 = 86_400;

async fn start_to(
    f: &Floor,
    receiver_id: String,
) -> Result<xc_cash::domain::handover::Handover, AppError> {
    let lines = vec![CountLine::new(
        "THB",
        f.db.denomination("THB", "1000").await,
        1,
    )];
    f.db.handovers()
        .start(
            &f.a,
            StartHandover {
                branch: f.main.clone(),
                from_shift_id: f.a_shift.clone(),
                receiver_id,
                lines,
                note: None,
                idempotency_key: unique("handover"),
            },
            at(NOW),
        )
        .await
}

async fn assert_refused(f: Floor, receiver_id: String, row: &str) {
    match start_to(&f, receiver_id).await {
        Err(AppError::Validation(msg)) => assert_eq!(msg, INVALID_RECEIVER, "{row}: refusal text"),
        other => panic!("{row}: expected Validation(INVALID_RECEIVER), got {other:?}"),
    }
    assert_eq!(f.db.handover_rows().await, 0, "{row}: nothing written");
    f.db.drop_schema().await;
}

async fn assert_pending(f: Floor, receiver_id: String, row: &str) {
    let h = start_to(&f, receiver_id)
        .await
        .unwrap_or_else(|e| panic!("{row}: expected a PENDING handover, got {e:?}"));
    assert_eq!(h.status, HandoverStatus::Pending, "{row}");
    f.db.drop_schema().await;
}

#[tokio::test]
async fn r1_role_starting_after_now_is_refused() {
    let f = floor().await;
    let r = receiver(&f.db, Role::Teller, Some(&f.main), at(NOW + DAY), None).await;
    assert_refused(f, r, "R1").await;
}

#[tokio::test]
async fn r2_org_wide_manager_started_yesterday_is_pending() {
    let f = floor().await;
    let r = receiver(&f.db, Role::BranchManager, None, at(NOW - DAY), None).await;
    assert_pending(f, r, "R2").await;
}

#[tokio::test]
async fn r3_role_starting_exactly_now_is_pending() {
    let f = floor().await;
    let r = receiver(&f.db, Role::Teller, Some(&f.main), at(NOW), None).await;
    assert_pending(f, r, "R3").await;
}

#[tokio::test]
async fn r4_role_ended_exactly_now_is_refused() {
    let f = floor().await;
    let r = receiver(
        &f.db,
        Role::Teller,
        Some(&f.main),
        at(NOW - DAY),
        Some(at(NOW)),
    )
    .await;
    assert_refused(f, r, "R4").await;
}

#[tokio::test]
async fn r5_role_without_shift_operate_is_refused() {
    let f = floor().await;
    let r = receiver(&f.db, Role::Finance, None, at(NOW - DAY), None).await;
    assert_refused(f, r, "R5").await;
}

#[tokio::test]
async fn r6_teller_of_another_branch_is_refused() {
    let f = floor().await;
    let other = f.db.branch("other").await;
    let r = receiver(&f.db, Role::Teller, Some(&other), at(NOW - DAY), None).await;
    assert_refused(f, r, "R6").await;
}
