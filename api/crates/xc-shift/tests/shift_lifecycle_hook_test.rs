//! #1128 (QU-A3) — `ShiftService` tells its `ShiftLifecycleHook` when a teller's shift opens or
//! closes, inside the shift's own transaction. Written before the implementation by
//! claude/dell-claude-6 on the order of the lane controller claude/ad05bf8d. Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! The queue side (`CounterService` as the hook staffs and releases the counter) is
//! `xc-queue/tests/queue_counter_hook_test.rs`. This file holds the shift side: a recording hook
//! shows who is told what, and when.
//!
//! Signatures this test assumes (`[required]` once pushed):
//! - `xc_kernel::shift_hook::ShiftLifecycleHook::{on_opened, on_closed}(tx, &BranchId,
//!   teller_user_id: &str, now) -> BoxFuture<AppResult<()>>`;
//! - `ShiftService::with_lifecycle_hook(self, Arc<dyn ShiftLifecycleHook>) -> Self`;
//! - `ShiftService::{open_in_as, close_in_as}(tx, &BranchId, cmd, actor, teller_user_id:
//!   Option<Uuid>, now)`. `open_in` / `close_in` keep their signatures (the provided
//!   `shift_service_test.rs` and `api/tests/abandon_request_test.rs` call them) and never call
//!   the hook.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod shift_service_common;

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use serde_json::{Value, json};
use shift_service_common::{Service, TestDb, at};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::shift_hook::ShiftLifecycleHook;
use xc_shift::domain::shift::commands::close_shift::CloseShift;
use xc_shift::domain::shift::commands::open_shift::OpenShift;
use xc_shift::domain::shift::entities::shift::{Shift, ShiftStatus};

/// One call the hook received: which, where, for whom, and how many OPEN shifts of that teller
/// the hook's own transaction saw at that moment.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Told {
    what: &'static str,
    branch: String,
    teller: String,
    open_shifts_seen: i64,
}

/// Records every call; refuses every call when `refuse` is set.
#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<Told>>,
    refuse: bool,
}

impl Recorder {
    fn refusing() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            refuse: true,
        }
    }

    fn calls(&self) -> Vec<Told> {
        self.calls.lock().unwrap().clone()
    }

    fn record<'a>(
        &'a self,
        what: &'static str,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        teller: &'a str,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let open_shifts_seen: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM shifts \
                 WHERE branch_id = $1 AND teller_id::text = $2 AND status = 'OPEN'",
            )
            .bind(branch.as_str())
            .bind(teller)
            .fetch_one(&mut **tx)
            .await?;
            self.calls.lock().unwrap().push(Told {
                what,
                branch: branch.as_str().to_owned(),
                teller: teller.to_owned(),
                open_shifts_seen,
            });
            if self.refuse {
                return Err(AppError::Validation("counterRefused".to_owned()));
            }
            Ok(())
        })
    }
}

impl ShiftLifecycleHook for Recorder {
    fn on_opened<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        teller_user_id: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        self.record("opened", tx, branch, teller_user_id)
    }

    fn on_closed<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        teller_user_id: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        self.record("closed", tx, branch, teller_user_id)
    }
}

const ACTOR: &str = "opener";

fn open_cmd(request_id: &str) -> OpenShift {
    OpenShift::from_payload(&json!({ "requestId": request_id, "opening": { "USD": 10 } }))
        .expect("a well-formed open payload")
}

fn close_cmd(request_id: &str, shift_id: &str) -> CloseShift {
    let usd: Value = json!({ "USD": 10 });
    CloseShift::from_payload(&json!({
        "requestId": request_id, "shiftId": shift_id, "counted": usd,
        "expected": usd, "note": "",
    }))
    .expect("a well-formed close payload")
}

fn rid(label: &str) -> String {
    format!("req-{label}-{}", Uuid::new_v4())
}

/// The `users.id` the seeded opener `ACTOR` resolves to: the Blueprint session's teller.
async fn teller_of(db: &TestDb) -> Uuid {
    let id: String = sqlx::query_scalar("SELECT id::text FROM users WHERE legacy_employee_id = $1")
        .bind(ACTOR)
        .fetch_one(&db.pool)
        .await
        .expect("the seeded opener's user");
    Uuid::parse_str(&id).expect("a users.id")
}

fn hooked(db: &TestDb, hook: Arc<dyn ShiftLifecycleHook>) -> Service {
    db.service().with_lifecycle_hook(hook)
}

async fn open_as(
    db: &TestDb,
    service: &Service,
    branch: &BranchId,
    teller: Option<Uuid>,
) -> AppResult<Shift> {
    let (svc, b, cmd) = (service.clone(), branch.clone(), open_cmd(&rid("open")));
    db.branch_tx()
        .run(branch, move |tx| {
            Box::pin(async move { svc.open_in_as(tx, &b, cmd, ACTOR, teller, at(0, 0)).await })
        })
        .await
}

async fn close_as(
    db: &TestDb,
    service: &Service,
    branch: &BranchId,
    shift_id: &str,
    teller: Option<Uuid>,
) -> AppResult<Shift> {
    let (svc, b, cmd) = (
        service.clone(),
        branch.clone(),
        close_cmd(&rid("close"), shift_id),
    );
    db.branch_tx()
        .run(branch, move |tx| {
            Box::pin(async move { svc.close_in_as(tx, &b, cmd, ACTOR, teller, at(60, 0)).await })
        })
        .await
}

async fn counters_in(db: &TestDb, branch: &BranchId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM queue_counters WHERE branch_id = $1")
        .bind(branch.as_str())
        .fetch_one(&db.pool)
        .await
        .expect("count the branch's counters")
}

// (a)
#[tokio::test(flavor = "multi_thread")]
async fn opening_a_shift_as_a_session_teller_tells_the_hook_in_the_same_transaction() {
    let db = TestDb::start().await;
    let branch = db.branch("qa3open").await;
    let teller = teller_of(&db).await;
    let hook = Arc::new(Recorder::default());
    let service = hooked(&db, hook.clone());

    let shift = open_as(&db, &service, &branch, Some(teller))
        .await
        .expect("the open succeeds");

    assert_eq!(shift.status, ShiftStatus::Open);
    assert_eq!(
        hook.calls(),
        vec![Told {
            what: "opened",
            branch: branch.as_str().to_owned(),
            teller: teller.to_string(),
            // the hook runs after the shift row is written, inside the same transaction
            open_shifts_seen: 1,
        }],
        "opening a shift as a session teller must call on_opened once, for that teller and branch"
    );
    db.drop_schema().await;
}

// (b)
#[tokio::test(flavor = "multi_thread")]
async fn closing_the_shift_as_a_session_teller_tells_the_hook_to_release_the_counter() {
    let db = TestDb::start().await;
    let branch = db.branch("qa3close").await;
    let teller = teller_of(&db).await;
    let hook = Arc::new(Recorder::default());
    let service = hooked(&db, hook.clone());

    let shift = open_as(&db, &service, &branch, Some(teller))
        .await
        .expect("the open succeeds");
    let closed = close_as(&db, &service, &branch, &shift.id, Some(teller))
        .await
        .expect("the close succeeds");

    assert_eq!(closed.status, ShiftStatus::Closed);
    let closes: Vec<Told> = hook
        .calls()
        .into_iter()
        .filter(|t| t.what == "closed")
        .collect();
    assert_eq!(
        closes,
        vec![Told {
            what: "closed",
            branch: branch.as_str().to_owned(),
            teller: teller.to_string(),
            // the shift row is already CLOSED in the hook's transaction
            open_shifts_seen: 0,
        }],
        "closing the shift as a session teller must call on_closed once, for that teller and branch"
    );
    db.drop_schema().await;
}

// (c)
#[tokio::test(flavor = "multi_thread")]
async fn a_legacy_open_and_close_with_no_session_user_never_reach_the_hook() {
    let db = TestDb::start().await;
    let branch = db.branch("qa3legacy").await;
    let hook = Arc::new(Recorder::default());
    let service = hooked(&db, hook.clone());

    // the session-aware forms with no user …
    let shift = open_as(&db, &service, &branch, None)
        .await
        .expect("a legacy open succeeds");
    close_as(&db, &service, &branch, &shift.id, None)
        .await
        .expect("a legacy close succeeds");
    // … and the unchanged legacy forms
    let (svc, b) = (service.clone(), branch.clone());
    db.branch_tx()
        .run(&branch, move |tx| {
            Box::pin(async move {
                let s = svc
                    .open_in(tx, &b, open_cmd(&rid("open")), ACTOR, at(120, 0))
                    .await?;
                svc.close_in(tx, &b, close_cmd(&rid("close"), &s.id), ACTOR, at(180, 0))
                    .await
            })
        })
        .await
        .expect("the legacy forms succeed");

    assert_eq!(hook.calls(), vec![], "no session user: the hook is skipped");
    assert_eq!(counters_in(&db, &branch).await, 0, "no counter was touched");
    db.drop_schema().await;
}

// (d)
#[tokio::test(flavor = "multi_thread")]
async fn a_hook_that_fails_rolls_the_open_back() {
    let db = TestDb::start().await;
    let branch = db.branch("qa3openfail").await;
    let teller = teller_of(&db).await;
    let service = hooked(&db, Arc::new(Recorder::refusing()));

    let refused = open_as(&db, &service, &branch, Some(teller)).await;

    assert!(
        matches!(&refused, Err(AppError::Validation(c)) if c == "counterRefused"),
        "the hook's refusal must be the open's answer, got {refused:?}"
    );
    assert_eq!(
        service.shifts(&branch).await.unwrap(),
        vec![],
        "a refused hook leaves no shift row"
    );
    assert_eq!(db.version(&branch).await, 0, "and no branch version step");
    db.drop_schema().await;
}

// (d), the close
#[tokio::test(flavor = "multi_thread")]
async fn a_hook_that_fails_rolls_the_close_back() {
    let db = TestDb::start().await;
    let branch = db.branch("qa3closefail").await;
    let teller = teller_of(&db).await;
    let shift = open_as(&db, &db.service(), &branch, Some(teller))
        .await
        .expect("the open succeeds");
    let version = db.version(&branch).await;
    let service = hooked(&db, Arc::new(Recorder::refusing()));

    let refused = close_as(&db, &service, &branch, &shift.id, Some(teller)).await;

    assert!(
        matches!(&refused, Err(AppError::Validation(c)) if c == "counterRefused"),
        "the hook's refusal must be the close's answer, got {refused:?}"
    );
    assert_eq!(
        service
            .shift(&branch, &shift.id)
            .await
            .unwrap()
            .map(|s| s.status),
        Some(xc_shift::domain::shift::entities::shift::ShiftStatus::Open),
        "a refused hook leaves the shift OPEN"
    );
    assert_eq!(
        db.version(&branch).await,
        version,
        "and no branch version step"
    );
    db.drop_schema().await;
}
