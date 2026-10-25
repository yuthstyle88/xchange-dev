//! Shift-close certification — up-front tests for the notices: a close count that waits for certification, and a
//! certified close count, each notify the organisation's Executives and the branch's BranchManagers at once, in the
//! event's own transaction (owner, 2026-10-25).
//!
//! Written before the implementation by the ticket author (claude/bc8fd29a, xchange BE lane controller).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Owner, verbatim (#564, claude/8be3a451's records): *"ต้องให้ เจ้าหน้าที่ ระดับ ผจก ขึ้นไป รับรอง แล้ว มีรายงานแจ้ง"*,
//! *"แต่ต้องมีการแจ้งเตือน กิจกรรม นี้ ทันที"* and *"รายงานนี้ต้อง ส่ง แจ้งเตือน เจ้าของ"*. In claude/bc8fd29a's session:
//! in-app now, phone Push later (#564 issuecomment-6392731530).
//!
//! The seam: `CashCountService::with_notices(self, Arc<dyn xc_kernel::notifications::Notifications>) -> Self` (a
//! builder; without it nothing is sent, so every existing caller keeps its behaviour). With it set:
//! - `submit_in` of a **CLOSE** count sends `close_count_waiting`;
//! - `verify_in` of a **CLOSE** count that ends `CountVerified::Verified` sends `shift_close_certified`.
//!
//! Each is sent once per recipient through `Notifications::notify_in` on the same transaction. The object is
//! `cash_count` / the count id. The recipients are every ACTIVE user with a current EXECUTIVE row (organisation-wide)
//! in the count's organisation, plus every ACTIVE user with a current BRANCH_MANAGER row at the count's branch, minus
//! the acting user.
//!
//! What this file pins ([required]):
//! - N1: an OPEN count's submit and verify send nothing.
//! - N2: a CLOSE count's submit sends `close_count_waiting` to the branch's manager and the Executive, and not to the
//!   counting teller nor to another branch's manager.
//! - N3: certifying that CLOSE count sends `shift_close_certified` to the Executive and to the branch's other manager,
//!   and not to the certifying manager, the teller, nor another branch's manager.
//! - N4: each notice names the count (`cash_count`, its id) and carries `branch` and `shift` params.
//! - N5: without `with_notices`, a CLOSE count's submit and verify behave as before and send nothing.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};
use futures::future::BoxFuture;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::domain::count::{CountKind, CountLine};
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

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 25, 2, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
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

impl Recorder {
    /// `(template, recipient)` of everything sent, sorted.
    fn sent(&self) -> Vec<(String, String)> {
        let mut v: Vec<_> = self
            .sent
            .lock()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r.template_key().to_string(),
                    r.recipient_user_id().to_string(),
                )
            })
            .collect();
        v.sort();
        v
    }

    fn clear(&self) {
        self.sent.lock().unwrap().clear();
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
        let schema = unique("closentc");
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

    /// A branch with THB 10,000 in its SAFE (ledger) and a count-difference limit of `limit_thb`.
    async fn branch(&self, limit_thb: &str) -> BranchId {
        let id = unique("main");
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert a branch");
        sqlx::query(
            "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
             VALUES ($1::uuid, $2, $3::numeric, 'test', now())",
        )
        .bind(ORG)
        .bind(&id)
        .bind(limit_thb)
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

    /// A user holding `role`, at `branch` (`None` = organisation-wide).
    async fn person(&self, name: &str, role: Role, branch: Option<&BranchId>) -> Actor {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
             VALUES ($1::uuid, $2, $3::uuid, $4, '2026-01-01T00:00:00Z')",
        )
        .bind(&id)
        .bind(role.as_str())
        .bind(ORG)
        .bind(branch.map(|b| b.as_str().to_string()))
        .execute(&self.pool)
        .await
        .expect("give the user a role");
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Actor::new(
            &id,
            ORG,
            vec![ScopedRole::new(role, branch.cloned(), from, None)],
            None,
        )
    }
}

/// THB 1000 × 2 + 100 × 3 = 2,300.
async fn thb(db: &Db) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.face("THB", "1000").await, 2),
        CountLine::new("THB", db.face("THB", "100").await, 3),
    ]
}

struct Floor {
    db: Db,
    main: BranchId,
    teller: Actor,
    manager: Actor,
    manager2: Actor,
    other_manager: Actor,
    executive: Actor,
}

/// MAIN with teller T, managers M and M2, an Executive E; branch B2 with its own manager O.
async fn floor() -> Floor {
    let db = Db::fresh().await;
    let main = db.branch("500").await;
    let b2 = db.branch("500").await;
    let teller = db.person("T", Role::Teller, Some(&main)).await;
    let manager = db.person("M", Role::BranchManager, Some(&main)).await;
    let manager2 = db.person("M2", Role::BranchManager, Some(&main)).await;
    let other_manager = db.person("O", Role::BranchManager, Some(&b2)).await;
    let executive = db.person("E", Role::Executive, None).await;
    Floor {
        db,
        main,
        teller,
        manager,
        manager2,
        other_manager,
        executive,
    }
}

fn submit(f: &Floor, kind: CountKind, shift: Option<String>, key: &str) -> SubmitCount {
    SubmitCount {
        branch: f.main.clone(),
        shift_id: shift,
        kind,
        lines: Vec::new(),
        note: None,
        idempotency_key: unique(key),
    }
}

/// The OPEN count of T, verified by M: answers T's shift id.
async fn open_shift(f: &Floor, counts: &CashCountService) -> String {
    let mut cmd = submit(f, CountKind::Open, None, "open");
    cmd.lines = thb(&f.db).await;
    let open = counts
        .submit(&f.teller, cmd, at(0))
        .await
        .expect("T submits the OPEN count");
    counts
        .verify(&f.manager, &open.id, at(60))
        .await
        .expect("M verifies the OPEN count")
        .shift_id
        .expect("T's shift")
}

async fn close_count(f: &Floor, counts: &CashCountService, shift: &str) -> String {
    let mut cmd = submit(f, CountKind::Close, Some(shift.to_string()), "close");
    cmd.lines = thb(&f.db).await;
    counts
        .submit(&f.teller, cmd, at(600))
        .await
        .expect("T submits the CLOSE count")
        .id
}

fn pairs(template: &str, who: &[&Actor]) -> Vec<(String, String)> {
    let mut v: Vec<_> = who
        .iter()
        .map(|a| (template.to_string(), a.user_id.clone()))
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn n1_an_open_count_sends_nothing() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());
    open_shift(&f, &counts).await;
    assert!(
        rec.sent().is_empty(),
        "OPEN submit/verify sent {:?}",
        rec.sent()
    );
}

#[tokio::test]
async fn n2_a_close_count_waiting_notifies_the_branch_managers_and_the_executive() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());
    let shift = open_shift(&f, &counts).await;
    rec.clear();

    close_count(&f, &counts, &shift).await;

    assert_eq!(
        rec.sent(),
        pairs(
            "close_count_waiting",
            &[&f.manager, &f.manager2, &f.executive]
        ),
        "the branch's managers and the Executive; not the teller, not B2's manager"
    );
}

#[tokio::test]
async fn n3_a_certified_close_count_notifies_the_executive_and_the_other_managers() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());
    let shift = open_shift(&f, &counts).await;
    let close = close_count(&f, &counts, &shift).await;
    rec.clear();

    counts
        .verify(&f.manager, &close, at(700))
        .await
        .expect("M certifies the exact CLOSE count");

    assert_eq!(
        rec.sent(),
        pairs("shift_close_certified", &[&f.manager2, &f.executive]),
        "not the certifying manager, not the teller, not B2's manager"
    );
    let _ = &f.other_manager;
    let _ = &f.teller;
}

#[tokio::test]
async fn n4_each_notice_names_the_count_and_carries_branch_and_shift() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());
    let shift = open_shift(&f, &counts).await;
    let close = close_count(&f, &counts, &shift).await;
    counts
        .verify(&f.manager, &close, at(700))
        .await
        .expect("M certifies the CLOSE count");

    let sent = rec.sent.lock().unwrap().clone();
    assert!(!sent.is_empty());
    for r in &sent {
        let object = r.object().expect("the notice names its object");
        assert_eq!(object.object_type, "cash_count");
        assert_eq!(object.object_id, close);
        assert_eq!(
            r.params().get("branch").map(String::as_str),
            Some(f.main.as_str())
        );
        assert_eq!(
            r.params().get("shift").map(String::as_str),
            Some(shift.as_str())
        );
    }
}

#[tokio::test]
async fn n5_without_notices_a_close_count_is_submitted_and_certified_as_before() {
    let f = floor().await;
    let counts = CashCountService::pg(f.db.pool.clone());
    let shift = open_shift(&f, &counts).await;
    let close = close_count(&f, &counts, &shift).await;
    counts
        .verify(&f.manager, &close, at(700))
        .await
        .expect("M certifies the CLOSE count with no notices set");
    let verified_by: Option<String> =
        sqlx::query_scalar("SELECT verified_by::text FROM cash_counts WHERE id = $1::uuid")
            .bind(&close)
            .fetch_one(&f.db.pool)
            .await
            .expect("read the CLOSE count");
    assert_eq!(verified_by, Some(f.manager.user_id.clone()));
}
