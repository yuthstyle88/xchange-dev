//! #2614: Accepting or rejecting a cash hand-over with a non-UUID or unknown id
//! answers NotFound ("handover"), not 500 serverError.

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
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::{HandoverService, StartHandover};
use xc_cash::domain::count::{CountKind, CountLine};
use xc_cash::domain::handover::{Handover, HandoverStatus};
use xc_kernel::ids::BranchId;

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

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

struct Db {
    pool: PgPool,
    migrator: Migrator,
}

impl Db {
    async fn new() -> Db {
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
        let schema = unique("ca4_unk");
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
        let migrator = Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations");
        Db { pool, migrator }
    }

    async fn fresh() -> Db {
        let db = Db::new().await;
        db.migrator
            .run(&db.pool)
            .await
            .expect("every migration applies in version order on a fresh database");
        db
    }

    async fn branch(&self, label: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert a branch of the organisation");
        lg4_fund_safe(&self.pool, &id, "THB", "1000000.00").await;
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
        .expect("insert a user");
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
            .expect("give the user a role");
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
        .unwrap_or_else(|e| panic!("the {currency} {face} face value: {e}"))
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

static LG4_FUND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
                        LG4_FUND.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
    tx.commit().await.expect("commit funding");
}

async fn thb_2300(db: &Db) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 3),
    ]
}

async fn opened(db: &Db, branch: &BranchId, teller: &Actor, manager: &Actor) -> String {
    let counts = db.counts();
    let count = counts
        .submit(
            teller,
            SubmitCount {
                branch: branch.clone(),
                shift_id: None,
                kind: CountKind::Open,
                lines: thb_2300(db).await,
                note: None,
                idempotency_key: unique("open"),
            },
            at(0),
        )
        .await
        .expect("the teller submits the OPEN count");
    counts
        .verify(manager, &count.id, at(60))
        .await
        .expect("the BRANCH_MANAGER verifies the OPEN count")
        .shift_id
        .expect("a verified OPEN count names the shift it opened")
}

fn start_cmd(
    branch: &BranchId,
    from_shift: &str,
    receiver: &Actor,
    lines: Vec<CountLine>,
) -> StartHandover {
    StartHandover {
        branch: branch.clone(),
        from_shift_id: from_shift.to_owned(),
        receiver_id: receiver.user_id.clone(),
        lines,
        note: None,
        idempotency_key: unique("handover"),
    }
}

#[allow(dead_code)]
struct Floor {
    db: Db,
    main: BranchId,
    a: Actor,
    b: Actor,
    manager: Actor,
    a_shift: String,
}

async fn floor() -> Floor {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let a = db.person("Ploy", &[Role::Teller]).await;
    let b = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let a_shift = opened(&db, &main, &a, &manager).await;
    Floor {
        db,
        main,
        a,
        b,
        manager,
        a_shift,
    }
}

async fn started(f: &Floor) -> Handover {
    f.db.handovers()
        .start(
            &f.a,
            start_cmd(&f.main, &f.a_shift, &f.b, thb_2300(&f.db).await),
            at(600),
        )
        .await
        .expect("A starts the hand-over of their own shift to B")
}

fn assert_not_found<T: std::fmt::Debug>(result: Result<T, AppError>, expected_id: &str) {
    match result {
        Err(AppError::NotFound { entity, id }) => {
            assert_eq!(entity, "handover");
            assert_eq!(id, expected_id);
        }
        other => panic!("expected AppError::NotFound for {expected_id}, got {other:?}"),
    }
}

// ── Part 8 Behaviour Table Tests ────────────────────────────────────────────

#[tokio::test]
async fn row1_accept_in_with_non_uuid_returns_not_found() {
    let f = floor().await;
    let count_before = f.db.handover_rows().await;

    let mut tx = f.db.pool.begin().await.expect("begin tx");
    let result =
        f.db.handovers()
            .accept_in(&mut tx, &f.b, "abc", at(700))
            .await;

    assert_not_found(result, "abc");
    assert_eq!(
        f.db.handover_rows().await,
        count_before,
        "nothing is written"
    );
}

#[tokio::test]
async fn row2_accept_in_with_random_uuid_returns_not_found() {
    let f = floor().await;
    let random_id = uuid::Uuid::new_v4().to_string();

    let mut tx = f.db.pool.begin().await.expect("begin tx");
    let result =
        f.db.handovers()
            .accept_in(&mut tx, &f.b, &random_id, at(700))
            .await;

    assert_not_found(result, &random_id);
}

#[tokio::test]
async fn row3_reject_in_with_non_uuid_returns_not_found() {
    let f = floor().await;
    let count_before = f.db.handover_rows().await;

    let mut tx = f.db.pool.begin().await.expect("begin tx");
    let result =
        f.db.handovers()
            .reject_in(&mut tx, &f.b, "abc", at(700))
            .await;

    assert_not_found(result, "abc");
    assert_eq!(
        f.db.handover_rows().await,
        count_before,
        "nothing is written"
    );
}

#[tokio::test]
async fn row4_reject_in_with_random_uuid_returns_not_found() {
    let f = floor().await;
    let random_id = uuid::Uuid::new_v4().to_string();

    let mut tx = f.db.pool.begin().await.expect("begin tx");
    let result =
        f.db.handovers()
            .reject_in(&mut tx, &f.b, &random_id, at(700))
            .await;

    assert_not_found(result, &random_id);
}

#[tokio::test]
async fn row5_accept_and_reject_of_real_pending_handover() {
    let f = floor().await;

    // Test accept on real pending handover
    let h1 = started(&f).await;
    let mut tx_accept = f.db.pool.begin().await.expect("begin tx accept");
    let accepted =
        f.db.handovers()
            .accept_in(&mut tx_accept, &f.b, &h1.id, at(700))
            .await
            .expect("accept real pending handover");
    tx_accept.commit().await.expect("commit accept");
    assert_eq!(accepted.status, HandoverStatus::Accepted);

    // Test reject on real pending handover
    // Need a second shift or second floor for reject
    let f2 = floor().await;
    let h2 = started(&f2).await;
    let mut tx_reject = f2.db.pool.begin().await.expect("begin tx reject");
    let rejected = f2
        .db
        .handovers()
        .reject_in(&mut tx_reject, &f2.b, &h2.id, at(700))
        .await
        .expect("reject real pending handover");
    tx_reject.commit().await.expect("commit reject");
    assert_eq!(rejected.status, HandoverStatus::Rejected);
}

#[tokio::test]
async fn edge_case_uppercase_uuid_is_accepted() {
    let f = floor().await;
    let h = started(&f).await;
    let upper_id = h.id.to_uppercase();
    assert_ne!(upper_id, h.id);

    let mut tx = f.db.pool.begin().await.expect("begin tx");
    let accepted =
        f.db.handovers()
            .accept_in(&mut tx, &f.b, &upper_id, at(700))
            .await
            .expect("accepts uppercase uuid string");
    tx.commit().await.expect("commit");
    assert_eq!(accepted.status, HandoverStatus::Accepted);
}
