//! #5643: a verified count tells the person who counted it, as a rejected one does (#5397). Up-front tests for
//! `CashCountService::verify_in` with notices set.
//!
//! Written before the implementation by the sub-controller (claude/studio-claude-16). Do not edit, delete or
//! weaken these tests; add your own in another file. Design: `docs/design/blueprint/cash-count-notices.md`.
//!
//! The seam is the one `count_reject_notice_test.rs` uses: `CashCountService::with_notices(Arc<dyn Notifications>)`.
//! With it set, a `verify_in` that answers `CountVerified::Verified` sends, through `Notifications::notify_in` on
//! the verify's own transaction, exactly one more request:
//! - template `count_verified` (rendered by `xc-notify/src/templates/count_verified.rs`, pinned by
//!   `xc-notify/tests/count_verified_template_test.rs`);
//! - recipient: the count's `counted_by`, and nobody else (not the verifier, not the branch's managers);
//! - event id `count-verified:{count id}`; object `cash_count` / the count id;
//! - params, exactly these keys:
//!   - `kind`: the count's kind code (`OPEN`, `CLOSE`, `SAFE_OPEN`, `SAFE`);
//!   - `branch`: the count's branch id;
//!   - `counted_on`: the day of `counted_at` in Thai time (UTC+7), `YYYY-MM-DD`;
//!   - `counted_time`: the time of `counted_at` in Thai time, `HH:MM`.
//!
//! What this file pins ([required]):
//! - V1: verifying an OPEN, a CLOSE and a SAFE count each send the notice above to whoever counted it.
//! - V2: a verify that answers `AwaitingFinance` sends no `count_verified`; once FINANCE approves, the next verify
//!   sends exactly one (design §3, the AWAITING_FINANCE ruling).
//! - V3: a refused verify (the counter verifying their own count, a count already verified) sends nothing more.
//! - V4: a `count_verified` insert that fails never fails the verify (#5408): the OPEN count is verified, the shift
//!   is open, and the notice was attempted once.
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
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::count_service::CountVerified;
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::ledger_approval::count_difference::register as register_count_difference;
use xc_cash::domain::count::{CashCount, CountKind, CountLine};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
use xc_kernel::notifications::{NotificationId, NotificationRequest, Notifications};

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const TEMPLATE: &str = "count_verified";

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
    /// Every `count_verified` request sent so far.
    fn verified(&self) -> Vec<NotificationRequest> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.template_key() == TEMPLATE)
            .cloned()
            .collect()
    }

    fn clear(&self) {
        self.sent.lock().unwrap().clear();
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
        let schema = unique("verntc");
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
    finance: Actor,
}

/// MAIN with teller T, managers M and M2, and FINANCE F.
async fn floor() -> Floor {
    let db = Db::fresh().await;
    let main = db.branch().await;
    let teller = db.person("T", &[Role::Teller], &main).await;
    let manager = db.person("M", &[Role::BranchManager], &main).await;
    let manager2 = db.person("M2", &[Role::BranchManager], &main).await;
    let finance = db.person("F", &[Role::Finance], &main).await;
    Floor {
        db,
        main,
        teller,
        manager,
        manager2,
        finance,
    }
}

/// THB 1000 × 2 + 100 × 3 = 2,300.
async fn thb(db: &Db) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.face("THB", "1000").await, 2),
        CountLine::new("THB", db.face("THB", "100").await, 3),
    ]
}

async fn submit_lines(
    f: &Floor,
    counts: &CashCountService,
    who: &Actor,
    kind: CountKind,
    shift: Option<String>,
    lines: Vec<CountLine>,
    now: DateTime<Utc>,
) -> CashCount {
    let cmd = SubmitCount {
        branch: f.main.clone(),
        shift_id: shift,
        kind,
        lines,
        note: None,
        idempotency_key: unique("count"),
    };
    counts
        .submit(who, cmd, now)
        .await
        .unwrap_or_else(|e| panic!("submit a {kind:?} count: {e:?}"))
}

async fn submit(
    f: &Floor,
    counts: &CashCountService,
    who: &Actor,
    kind: CountKind,
    shift: Option<String>,
    now: DateTime<Utc>,
) -> CashCount {
    let lines = thb(&f.db).await;
    submit_lines(f, counts, who, kind, shift, lines, now).await
}

/// Verifies in its own transaction and commits whatever it answers, as the route's seam does.
async fn verify(
    f: &Floor,
    counts: &CashCountService,
    who: &Actor,
    count_id: &str,
    now: DateTime<Utc>,
) -> pos_error::AppResult<CountVerified> {
    let mut tx = f.db.pool.begin().await.expect("begin");
    let answer = counts.verify_in(&mut tx, who, count_id, now).await;
    if answer.is_ok() {
        tx.commit().await.expect("commit");
    }
    answer
}

/// T's OPEN count at 12:30 Thai time, verified by M at 12:31: answers T's shift id.
async fn open_shift(f: &Floor, counts: &CashCountService) -> String {
    let open = submit(f, counts, &f.teller, CountKind::Open, None, at(0)).await;
    match verify(f, counts, &f.manager, &open.id, at(60))
        .await
        .expect("M verifies the OPEN count")
    {
        CountVerified::Verified(c) => c.shift_id.expect("T's shift"),
        other => panic!("the OPEN count verifies: {other:?}"),
    }
}

fn expected_params(
    kind: &str,
    branch: &BranchId,
    day: &str,
    time: &str,
) -> BTreeMap<String, String> {
    [
        ("kind", kind),
        ("branch", branch.as_str()),
        ("counted_on", day),
        ("counted_time", time),
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
    let sent = rec.verified();
    assert_eq!(sent.len(), 1, "exactly one count_verified notice: {sent:?}");
    let n = &sent[0];
    assert_eq!(n.recipient_user_id(), to.user_id, "it goes to the counter");
    assert_eq!(n.event_id(), format!("count-verified:{}", count.id));
    let object = n.object().expect("the notice names the count");
    assert_eq!(object.object_type, "cash_count");
    assert_eq!(object.object_id, count.id);
    assert_eq!(n.params(), &params);
}

// V1: an OPEN count.
#[tokio::test]
async fn v1_verifying_an_open_count_tells_the_teller_who_counted_it() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());
    let open = submit(&f, &counts, &f.teller, CountKind::Open, None, at(0)).await;

    let answer = verify(&f, &counts, &f.manager, &open.id, at(60))
        .await
        .expect("M verifies the OPEN count");
    assert!(matches!(answer, CountVerified::Verified(_)), "{answer:?}");

    assert_one_notice(
        &rec,
        &open,
        &f.teller,
        expected_params("OPEN", &f.main, "2026-10-28", "12:30"),
    );
}

// V1: a CLOSE count. The managers' `shift_close_certified` notices are not this file's business; only the counter
// gets `count_verified`.
#[tokio::test]
async fn v1_verifying_a_close_count_tells_only_the_teller_who_counted_it() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());
    let shift = open_shift(&f, &counts).await;
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
    rec.clear();

    verify(&f, &counts, &f.manager2, &close.id, at(3 * 3600 + 600))
        .await
        .expect("M2 verifies the exact CLOSE count");

    assert_one_notice(
        &rec,
        &close,
        &f.teller,
        expected_params("CLOSE", &f.main, "2026-10-28", "15:35"),
    );
}

// V1: a SAFE count, counted by a manager and verified by another; the Thai day rolls at 17:00 UTC.
#[tokio::test]
async fn v1_verifying_a_safe_count_tells_the_manager_who_counted_it() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());
    // 2026-10-28 05:30 UTC + 12h = 17:30 UTC = 2026-10-29 00:30 Thai time.
    let safe = submit(
        &f,
        &counts,
        &f.manager,
        CountKind::Safe,
        None,
        at(12 * 3600),
    )
    .await;

    verify(&f, &counts, &f.manager2, &safe.id, at(12 * 3600 + 60))
        .await
        .expect("M2 verifies M's SAFE count");

    assert_one_notice(
        &rec,
        &safe,
        &f.manager,
        expected_params("SAFE", &f.main, "2026-10-29", "00:30"),
    );
}

// V2: AWAITING_FINANCE sends nothing; the verify after FINANCE approves sends the one notice.
#[tokio::test]
async fn v2_awaiting_finance_sends_nothing_and_the_final_verify_sends_one() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());
    let shift = open_shift(&f, &counts).await;
    // Expected 2,300; counted 1,000 ⇒ 1,300 short, above the 500 limit.
    let short = vec![CountLine::new("THB", f.db.face("THB", "1000").await, 1)];
    let close = submit_lines(
        &f,
        &counts,
        &f.teller,
        CountKind::Close,
        Some(shift),
        short,
        at(600),
    )
    .await;
    rec.clear();

    let request = match verify(&f, &counts, &f.manager, &close.id, at(660))
        .await
        .expect("the verify answers")
    {
        CountVerified::AwaitingFinance(id) => id,
        other => panic!("the short CLOSE count waits for FINANCE: {other:?}"),
    };
    assert!(
        rec.verified().is_empty(),
        "AWAITING_FINANCE sends no count_verified: {:?}",
        rec.verified()
    );

    // While the request is PENDING, verifying again still sends nothing.
    let again = verify(&f, &counts, &f.manager, &close.id, at(670))
        .await
        .expect("the verify answers");
    assert!(
        matches!(again, CountVerified::AwaitingFinance(_)),
        "{again:?}"
    );
    assert!(rec.verified().is_empty(), "{:?}", rec.verified());

    let mut registry = ApprovalRegistry::new();
    register_count_difference(&mut registry, f.db.pool.clone());
    let approvals = ApprovalService::new(
        registry,
        Arc::new(PgApprovalRepository::new(f.db.pool.clone())),
        Arc::new(PgAuditLog::new(f.db.pool.clone())),
    );
    let mut tx = f.db.pool.begin().await.expect("begin");
    approvals
        .decide(
            &mut tx,
            &RequestId::new(&request),
            &f.finance,
            Outcome::Approved,
            "approved difference",
            &unique("decide"),
        )
        .await
        .expect("FINANCE approves the difference");
    tx.commit().await.expect("commit");
    assert!(
        rec.verified().is_empty(),
        "FINANCE's decision sends no count_verified by itself"
    );

    verify(&f, &counts, &f.manager, &close.id, at(700))
        .await
        .expect("M verifies after FINANCE approves");

    assert_one_notice(
        &rec,
        &close,
        &f.teller,
        expected_params("CLOSE", &f.main, "2026-10-28", "12:40"),
    );
}

// V3: refused verifies send nothing.
#[tokio::test]
async fn v3_a_refused_verify_sends_nothing() {
    let f = floor().await;
    let rec = Arc::new(Recorder::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(rec.clone());

    // The counter verifying their own count.
    let safe = submit(&f, &counts, &f.manager, CountKind::Safe, None, at(0)).await;
    verify(&f, &counts, &f.manager, &safe.id, at(60))
        .await
        .expect_err("M may not verify M's own count");
    assert!(rec.verified().is_empty(), "{:?}", rec.verified());

    // A count already verified.
    let open = submit(&f, &counts, &f.teller, CountKind::Open, None, at(120)).await;
    verify(&f, &counts, &f.manager, &open.id, at(180))
        .await
        .expect("M verifies the OPEN count");
    rec.clear();
    verify(&f, &counts, &f.manager2, &open.id, at(240))
        .await
        .expect_err("a verified count is not verified twice");
    assert!(rec.verified().is_empty(), "{:?}", rec.verified());
}

// V4: a failed notice insert never fails the verify (#5408).
#[tokio::test]
async fn v4_a_verify_whose_notice_fails_still_commits() {
    let f = floor().await;
    let failing = Arc::new(FailingInsert::default());
    let counts = CashCountService::pg(f.db.pool.clone()).with_notices(failing.clone());
    let open = submit(&f, &counts, &f.teller, CountKind::Open, None, at(0)).await;

    let answer = verify(&f, &counts, &f.manager, &open.id, at(60))
        .await
        .expect("the verify succeeds although its notice failed");
    assert!(matches!(answer, CountVerified::Verified(_)), "{answer:?}");

    let attempts = failing
        .attempts
        .lock()
        .unwrap()
        .iter()
        .filter(|t| t.as_str() == TEMPLATE)
        .count();
    assert_eq!(attempts, 1, "the count_verified notice was attempted once");

    let verified_by: Option<String> =
        sqlx::query_scalar("SELECT verified_by::text FROM cash_counts WHERE id = $1::uuid")
            .bind(&open.id)
            .fetch_one(&f.db.pool)
            .await
            .expect("read the count");
    assert_eq!(verified_by.as_deref(), Some(f.manager.user_id.as_str()));
    let open_shifts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN'",
    )
    .bind(&f.teller.user_id)
    .fetch_one(&f.db.pool)
    .await
    .expect("read shifts");
    assert_eq!(open_shifts, 1, "T's shift is open");
}
