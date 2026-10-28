//! #5397 (C5): a rejected count tells the person who counted it. Up-front tests for
//! `CashCountService::reject_in` with notices set.
//!
//! Written before the implementation by the sub-controller (claude/studio-claude-16). Do not edit, delete or
//! weaken these tests; add your own in another file.
//!
//! The seam is the one `shift_close_notices_test.rs` uses: `CashCountService::with_notices(Arc<dyn Notifications>)`.
//! With it set, a successful `reject_in` sends, through `Notifications::notify_in` on the reject's own transaction,
//! exactly one request:
//! - template `count_rejected` (rendered by `xc-notify/src/templates/count_rejected.rs`, pinned by
//!   `xc-notify/tests/count_rejected_template_test.rs`);
//! - recipient: the count's `counted_by`, and nobody else (not the rejecter, not the branch's managers);
//! - event id `count-rejected:{count id}`; object `cash_count` / the count id;
//! - params, exactly these keys:
//!   - `kind`: the count's kind code (`OPEN`, `CLOSE`, `SAFE_OPEN`, `SAFE`);
//!   - `branch`: the count's branch id;
//!   - `counted_on`: the day of `counted_at` in Thai time (UTC+7), `YYYY-MM-DD`;
//!   - `counted_time`: the time of `counted_at` in Thai time, `HH:MM`;
//!   - `reason`: the stored (trimmed) reason, verbatim; personal data in it is masked with
//!     `xc_kernel::pii::redact_text`, so a reason holding a phone number still reaches the counter.
//!
//! What this file pins ([required]):
//! - N1: rejecting an OPEN count and a CLOSE count each send the notice above to the teller who counted.
//! - N1b: a reason with a phone number still sends the notice, with the number masked.
//! - N3: a refused reject (not waiting, same counter, empty reason) sends nothing.
//! - N4: without `with_notices`, the reject succeeds and nothing else changes.
//! - N5 (claude/553782's addendum, #5397, 2026-10-28 19:08 ICT): the notice's object is `cash_count` / the rejected
//!   count's id, as `CloseNotices`' notices are, so the staff bell (`web/src/shared/notifications/NoticeBell.tsx:62`)
//!   pops its toast.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

use std::collections::BTreeMap;
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
use xc_cash::domain::count::{CashCount, CountKind, CountLine, RejectError};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
use xc_kernel::notifications::{NotificationId, NotificationRequest, Notifications};

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const TEMPLATE: &str = "count_rejected";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

/// 2026-10-28 05:30 UTC = 12:30 Thai time, plus `seconds`.
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

impl Recorder {
    /// Every `count_rejected` request sent so far.
    fn rejected(&self) -> Vec<NotificationRequest> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.template_key() == TEMPLATE)
            .cloned()
            .collect()
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

fn expected_params(
    kind: &str,
    branch: &BranchId,
    day: &str,
    time: &str,
    reason: &str,
) -> BTreeMap<String, String> {
    [
        ("kind", kind),
        ("branch", branch.as_str()),
        ("counted_on", day),
        ("counted_time", time),
        ("reason", reason),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

fn assert_one_notice(
    rec: &Recorder,
    count: &CashCount,
    to: &Actor,
    params: BTreeMap<String, String>,
) {
    let sent = rec.rejected();
    assert_eq!(sent.len(), 1, "exactly one count_rejected notice: {sent:?}");
    let n = &sent[0];
    assert_eq!(n.recipient_user_id(), to.user_id, "it goes to the counter");
    assert_eq!(n.event_id(), format!("count-rejected:{}", count.id));
    let object = n.object().expect("the notice names the count");
    assert_eq!(object.object_type, "cash_count");
    assert_eq!(object.object_id, count.id);
    assert_eq!(n.params(), &params);
}

// N1: an OPEN count.
#[tokio::test]
async fn n1_rejecting_an_open_count_tells_the_teller_who_counted_it() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());
    let open = submit(&f, &counts, &f.teller, CountKind::Open, None, at(0)).await;

    reject(
        &f,
        &counts,
        &f.manager,
        &open.id,
        "  ยอด USD ไม่ตรง นับใหม่  ",
        at(120),
    )
    .await
    .expect("M rejects the OPEN count");

    assert_one_notice(
        &rec,
        &open,
        &f.teller,
        expected_params(
            "OPEN",
            &f.main,
            "2026-10-28",
            "12:30",
            "ยอด USD ไม่ตรง นับใหม่",
        ),
    );
}

// N1: a CLOSE count; only the counter hears of the rejection, not the branch's managers.
#[tokio::test]
async fn n1_rejecting_a_close_count_tells_only_the_teller_who_counted_it() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());
    let open = submit(&f, &counts, &f.teller, CountKind::Open, None, at(0)).await;
    let shift = counts
        .verify(&f.manager, &open.id, at(60))
        .await
        .expect("M verifies the OPEN count")
        .shift_id
        .expect("T's shift");
    // 05:30 UTC + 3h05m = 08:35 UTC = 15:35 Thai time.
    let close = submit(
        &f,
        &counts,
        &f.teller,
        CountKind::Close,
        Some(shift),
        at(3 * 3600 + 300),
    )
    .await;

    reject(
        &f,
        &counts,
        &f.manager2,
        &close.id,
        "ขาด 100 บาท",
        at(3 * 3600 + 600),
    )
    .await
    .expect("M2 rejects the CLOSE count");

    assert_one_notice(
        &rec,
        &close,
        &f.teller,
        expected_params("CLOSE", &f.main, "2026-10-28", "15:35", "ขาด 100 บาท"),
    );
}

// N1: the Thai day, not the UTC day (17:00 UTC rolls the date).
#[tokio::test]
async fn n1_the_count_time_is_thai_time() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());
    // 2026-10-28 17:45 UTC = 2026-10-29 00:45 Thai time.
    let late = Utc.with_ymd_and_hms(2026, 10, 28, 17, 45, 0).unwrap();
    let open = submit(&f, &counts, &f.teller, CountKind::Open, None, late).await;

    reject(
        &f,
        &counts,
        &f.manager,
        &open.id,
        "นับใหม่",
        late + chrono::Duration::seconds(60),
    )
    .await
    .expect("M rejects");

    assert_one_notice(
        &rec,
        &open,
        &f.teller,
        expected_params("OPEN", &f.main, "2026-10-29", "00:45", "นับใหม่"),
    );
}

// N1b: personal data in the reason is masked; the notice is still sent.
#[tokio::test]
async fn n1b_a_reason_with_a_phone_number_is_masked_and_still_sent() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());
    let open = submit(&f, &counts, &f.teller, CountKind::Open, None, at(0)).await;

    reject(
        &f,
        &counts,
        &f.manager,
        &open.id,
        "โทรหา 0812345678 ก่อนนับใหม่",
        at(120),
    )
    .await
    .expect("M rejects");

    assert_one_notice(
        &rec,
        &open,
        &f.teller,
        expected_params("OPEN", &f.main, "2026-10-28", "12:30", "โทรหา *** ก่อนนับใหม่"),
    );
}

// N3: each refusal sends nothing.
#[tokio::test]
async fn n3_a_refused_reject_sends_nothing() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());
    let open = submit(&f, &counts, &f.teller, CountKind::Open, None, at(0)).await;

    // Empty reason.
    match reject(&f, &counts, &f.manager, &open.id, "   ", at(60)).await {
        Err(RejectError::ReasonRequired) => {}
        other => panic!("expected ReasonRequired, got {other:?}"),
    }
    assert!(rec.rejected().is_empty(), "an empty reason sends nothing");

    // Same counter: D (teller and manager) counts, then tries to reject their own count.
    let both =
        f.db.person("D", &[Role::Teller, Role::BranchManager], &f.main)
            .await;
    let own = submit(&f, &counts, &both, CountKind::Open, None, at(30)).await;
    match reject(&f, &counts, &both, &own.id, "นับใหม่", at(60)).await {
        Err(RejectError::SameCounter) => {}
        other => panic!("expected SameCounter, got {other:?}"),
    }
    assert!(
        rec.rejected().is_empty(),
        "the counter's own reject sends nothing"
    );

    // Not waiting: rejected once (one notice), then again.
    reject(&f, &counts, &f.manager, &open.id, "นับใหม่", at(90))
        .await
        .expect("the first reject");
    assert_eq!(rec.rejected().len(), 1, "the first reject sends one notice");
    match reject(&f, &counts, &f.manager2, &open.id, "อีกครั้ง", at(120)).await {
        Err(RejectError::NotWaiting) => {}
        other => panic!("expected NotWaiting, got {other:?}"),
    }
    assert_eq!(
        rec.rejected().len(),
        1,
        "a reject of a count that is no longer waiting sends nothing more"
    );
}

// N5: the object names the rejected count, so the staff bell toasts.
#[tokio::test]
async fn n5_the_notice_names_the_rejected_cash_count() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());
    let first = submit(&f, &counts, &f.teller, CountKind::Open, None, at(0)).await;
    reject(&f, &counts, &f.manager, &first.id, "นับใหม่", at(60))
        .await
        .expect("M rejects the first count");
    let second = submit(&f, &counts, &f.teller, CountKind::Open, None, at(120)).await;
    reject(&f, &counts, &f.manager, &second.id, "นับใหม่อีกครั้ง", at(180))
        .await
        .expect("M rejects the second count");

    let objects: Vec<(String, String)> = rec
        .rejected()
        .iter()
        .map(|n| {
            let o = n
                .object()
                .expect("every count_rejected notice has an object");
            (o.object_type.clone(), o.object_id.clone())
        })
        .collect();
    assert_eq!(
        objects,
        vec![
            ("cash_count".to_string(), first.id.clone()),
            ("cash_count".to_string(), second.id.clone()),
        ],
        "each notice names its own rejected count"
    );
}

// N4: no notices port — the reject still succeeds.
#[tokio::test]
async fn n4_without_notices_the_reject_still_succeeds() {
    let f = floor().await;
    let counts = CashCountService::pg(f.db.pool.clone());
    let open = submit(&f, &counts, &f.teller, CountKind::Open, None, at(0)).await;

    let rejected = reject(&f, &counts, &f.manager, &open.id, "นับใหม่", at(60))
        .await
        .expect("the reject succeeds without notices");
    assert_eq!(rejected.rejection.expect("the rejection").reason, "นับใหม่");
}
