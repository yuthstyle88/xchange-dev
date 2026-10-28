//! #5408: a count notice that fails to insert never fails the count action. Up-front test.
//!
//! Written before the implementation by the sub-controller (claude/studio-claude-16). Do not edit, delete or
//! weaken these tests; add your own in another file.
//!
//! The three count notices go out through `CloseNotices` (`xc-cash/src/application/count/close_notices.rs`). The
//! close notices already wrap each insert in a savepoint and carry on when it fails; `notify_rejected_in` must do the
//! same through one shared savepoint-and-send helper. The failing fake below runs a statement that errors **inside the
//! transaction** before it returns `Err`, as a real failed insert does; without a savepoint Postgres then refuses every
//! later statement of that transaction, so only a real `SAVEPOINT` / `ROLLBACK TO SAVEPOINT` lets the action commit.
//!
//! What this file pins ([required]):
//! - F1: a reject whose notice insert fails still commits: the answer is `Ok` with the rejection, and
//!   `rejected_by`, `rejected_at` and `reject_reason` are stored. The notice was attempted once.
//! - F2: a reject whose notice insert succeeds sends one `count_rejected` notice to `counted_by`, object
//!   `cash_count` / the count id (as today).
//! - F3: a CLOSE count whose `close_count_waiting` insert fails is still submitted (as today).
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

use chrono::{DateTime, TimeZone, Utc};
use futures::future::BoxFuture;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::domain::count::{CashCount, CountKind, CountLine, RejectError};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
use xc_kernel::notifications::{NotificationId, NotificationRequest, Notifications};

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

/// 2026-10-28 05:30 UTC, plus `seconds`.
fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 28, 5, 30, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

/// Records every request; writes nothing.
#[derive(Default)]
struct Recorder {
    sent: Mutex<Vec<NotificationRequest>>,
}

impl Notifications for Recorder {
    fn notify_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        request: &'a NotificationRequest,
    ) -> BoxFuture<'a, pos_error::AppResult<NotificationId>> {
        Box::pin(async move {
            self.sent.lock().unwrap().push(request.clone());
            Ok(NotificationId(format!("n-{}", request.event_id())))
        })
    }
}

/// Fails like a real insert: a statement errors inside the caller's transaction, then `Err` is returned.
#[derive(Default)]
struct FailingInsert {
    attempts: Mutex<Vec<String>>,
}

impl Notifications for FailingInsert {
    fn notify_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        request: &'a NotificationRequest,
    ) -> BoxFuture<'a, pos_error::AppResult<NotificationId>> {
        Box::pin(async move {
            self.attempts
                .lock()
                .unwrap()
                .push(request.template_key().to_string());
            let failed = sqlx::query("SELECT 1 / 0").execute(&mut **tx).await;
            assert!(failed.is_err(), "the failing statement must fail");
            Err(pos_error::AppError::Internal(
                "notification insert failed".to_string(),
            ))
        })
    }
}

impl FailingInsert {
    fn attempts(&self, template: &str) -> usize {
        self.attempts
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.as_str() == template)
            .count()
    }
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
        let schema = unique("rejntc");
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
        let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(dir.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("every migration applies");
        Db { pool }
    }

    /// A branch with THB 10,000 in its SAFE (ledger) and a count-difference limit of 500 THB.
    async fn branch(&self) -> BranchId {
        let id = unique("main");
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert a branch");
        sqlx::query(
            "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
             VALUES ($1::uuid, $2, 500::numeric, 'test', now())",
        )
        .bind(ORG)
        .bind(&id)
        .execute(&self.pool)
        .await
        .expect("a count-difference limit");
        let b = BranchId::new(id);
        let mut tx = self.pool.begin().await.unwrap();
        LedgerCashPostings::new()
            .post_in(
                &mut tx,
                &[PostingRequest {
                    from: CashPlace::Bank,
                    to: CashPlace::Safe(b.clone()),
                    currency: CurrencyCode::THB,
                    quantity: ForeignAmount::from_decimal(d("10000.00")),
                    source: SourceRef {
                        source_type: SourceType::Opening,
                        source_id: unique("fund"),
                        source_key: "thb".to_string(),
                    },
                    line: None,
                    reverses: None,
                }],
                None,
            )
            .await
            .unwrap_or_else(|e| panic!("fund the safe: {e:?}"));
        tx.commit().await.unwrap();
        b
    }

    async fn face(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the {currency} {face} face value: {e}"))
    }

    /// A user holding `roles` at `branch`.
    async fn person(&self, name: &str, roles: &[Role], branch: &BranchId) -> Actor {
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
                "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
                 VALUES ($1::uuid, $2, $3::uuid, $4, '2026-01-01T00:00:00Z')",
            )
            .bind(&id)
            .bind(role.as_str())
            .bind(ORG)
            .bind(branch.as_str())
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
                .map(|r| ScopedRole::new(*r, Some(branch.clone()), from, None))
                .collect(),
            None,
        )
    }
}

struct Floor {
    db: Db,
    main: BranchId,
    teller: Actor,
    manager: Actor,
    manager2: Actor,
}

/// MAIN with teller T and managers M and M2.
async fn floor() -> Floor {
    let db = Db::fresh().await;
    let main = db.branch().await;
    let teller = db.person("T", &[Role::Teller], &main).await;
    let manager = db.person("M", &[Role::BranchManager], &main).await;
    let manager2 = db.person("M2", &[Role::BranchManager], &main).await;
    Floor {
        db,
        main,
        teller,
        manager,
        manager2,
    }
}

/// THB 1000 × 2 + 100 × 3 = 2,300.
async fn thb(db: &Db) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.face("THB", "1000").await, 2),
        CountLine::new("THB", db.face("THB", "100").await, 3),
    ]
}

async fn submit(
    f: &Floor,
    counts: &CashCountService,
    who: &Actor,
    kind: CountKind,
    shift: Option<String>,
    now: DateTime<Utc>,
) -> CashCount {
    let cmd = SubmitCount {
        branch: f.main.clone(),
        shift_id: shift,
        kind,
        lines: thb(&f.db).await,
        note: None,
        idempotency_key: unique("count"),
    };
    counts
        .submit(who, cmd, now)
        .await
        .unwrap_or_else(|e| panic!("submit a {kind:?} count: {e:?}"))
}

/// Rejects in its own transaction; commits only on success.
async fn reject(
    f: &Floor,
    counts: &CashCountService,
    who: &Actor,
    count_id: &str,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<CashCount, RejectError> {
    let mut tx = f.db.pool.begin().await.expect("begin");
    let answer = counts.reject_in(&mut tx, who, count_id, reason, now).await;
    if answer.is_ok() {
        tx.commit().await.expect("commit");
    }
    answer
}

/// `rejected_by`, `rejected_at`, `reject_reason` of a count, as stored.
async fn stored(f: &Floor, id: &str) -> (Option<String>, Option<DateTime<Utc>>, Option<String>) {
    sqlx::query_as(
        "SELECT rejected_by::text, rejected_at, reject_reason FROM cash_counts WHERE id = $1::uuid",
    )
    .bind(id)
    .fetch_one(&f.db.pool)
    .await
    .expect("read the count")
}

// F1
#[tokio::test]
async fn f1_a_reject_whose_notice_fails_still_commits() {
    let f = floor().await;
    let failing = Arc::new(FailingInsert::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(failing.clone());
    let open = submit(&f, &counts, &f.teller, CountKind::Open, None, at(0)).await;

    let rejected = reject(
        &f,
        &counts,
        &f.manager,
        &open.id,
        "ยอด USD ไม่ตรง นับใหม่",
        at(120),
    )
    .await
    .expect("the reject commits although its notice failed");

    assert_eq!(
        rejected.rejection.expect("the rejection").reason,
        "ยอด USD ไม่ตรง นับใหม่"
    );
    assert_eq!(
        failing.attempts("count_rejected"),
        1,
        "the notice was attempted once"
    );
    assert_eq!(
        stored(&f, &open.id).await,
        (
            Some(f.manager.user_id.clone()),
            Some(at(120)),
            Some("ยอด USD ไม่ตรง นับใหม่".to_string())
        ),
        "the rejection is stored"
    );
}

// F2
#[tokio::test]
async fn f2_a_reject_whose_notice_succeeds_sends_it_as_today() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());
    let open = submit(&f, &counts, &f.teller, CountKind::Open, None, at(0)).await;

    reject(&f, &counts, &f.manager, &open.id, "นับใหม่", at(120))
        .await
        .expect("M rejects");

    let sent: Vec<(String, String, String, String)> = rec
        .sent
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.template_key() == "count_rejected")
        .map(|r| {
            let o = r.object().expect("an object");
            (
                r.recipient_user_id().to_string(),
                r.event_id().to_string(),
                o.object_type.clone(),
                o.object_id.clone(),
            )
        })
        .collect();
    assert_eq!(
        sent,
        vec![(
            f.teller.user_id.clone(),
            format!("count-rejected:{}", open.id),
            "cash_count".to_string(),
            open.id.clone()
        )]
    );
    assert_eq!(
        stored(&f, &open.id).await.0.as_deref(),
        Some(f.manager.user_id.as_str())
    );
}

// F3
#[tokio::test]
async fn f3_a_close_count_whose_waiting_notice_fails_is_still_submitted() {
    let f = floor().await;
    let plain = CashCountService::pg(f.db.pool.clone());
    let open = submit(&f, &plain, &f.teller, CountKind::Open, None, at(0)).await;
    let shift = plain
        .verify(&f.manager, &open.id, at(60))
        .await
        .expect("M verifies the OPEN count")
        .shift_id
        .expect("T's shift");

    let failing = Arc::new(FailingInsert::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(failing.clone());
    let close = submit(
        &f,
        &counts,
        &f.teller,
        CountKind::Close,
        Some(shift),
        at(600),
    )
    .await;

    assert!(
        failing.attempts("close_count_waiting") >= 1,
        "the waiting notice was attempted"
    );
    let found: i64 = sqlx::query_scalar("SELECT count(*) FROM cash_counts WHERE id = $1::uuid")
        .bind(&close.id)
        .fetch_one(&f.db.pool)
        .await
        .expect("read the count");
    assert_eq!(found, 1, "the CLOSE count is stored");
}
