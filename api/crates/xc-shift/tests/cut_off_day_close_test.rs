//! #4136 S1: a cut-off with no shift left OPEN closes the branch day too. Design
//! `docs/design/blueprint/queue-day-close.md` §2 rows 11–14 and §3.3a: `BranchDayService::cut_off` asks
//! `ShiftService::close_day_if_last_in` on its own transaction, after it stores CUT_OFF.
//!
//! Written before the implementation by the ticket author (claude/studio-claude-9). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! The cutter is a BRANCH_MANAGER whose user id is not a UUID, so `cut_off` finds no shift of its own (as at
//! `application/branch_day/service.rs:224-227`). A recording hook stands in for the queue.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

mod shift_service_common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use shift_service_common::{Service, TestDb, at, within};
use sqlx::{Postgres, Transaction};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_kernel::shift_hook::ShiftLifecycleHook;
use xc_shift::application::branch_day::BranchDayService;
use xc_shift::domain::custody::repositories::custody_ports::{CustodyPorts, OutsideFacts};
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

/// Records every `on_day_closed` with the OPEN-shift count it reads on the caller's transaction, and answers an
/// error when `fail` is set.
#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<(String, i64)>>,
    fail: bool,
}

impl Recorder {
    fn calls(&self) -> Vec<(String, i64)> {
        self.calls.lock().expect("recorder lock").clone()
    }
}

impl ShiftLifecycleHook for Recorder {
    fn on_day_closed<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let open: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM shifts WHERE branch_id = $1 AND status = 'OPEN'",
            )
            .bind(branch.as_str())
            .fetch_one(&mut **tx)
            .await?;
            self.calls
                .lock()
                .expect("recorder lock")
                .push((branch.as_str().to_owned(), open));
            if self.fail {
                return Err(AppError::Internal("the day hook failed".into()));
            }
            Ok(())
        })
    }
}

/// The ports a cut-off reads: nothing outside is pending.
struct Ports;

impl CustodyPorts for Ports {
    fn outside_facts<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _open_shift_id: Option<&'a str>,
    ) -> BoxFuture<'a, AppResult<OutsideFacts>> {
        Box::pin(async { Ok(OutsideFacts::default()) })
    }

    fn customer_known<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async { Ok(true) })
    }

    fn evidence_owned<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _owner_id: &'a str,
        _keys: &'a [String],
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async { Ok(true) })
    }

    fn service_window<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _now: CustodyTime,
    ) -> BoxFuture<'a, AppResult<ServiceWindow>> {
        Box::pin(async {
            Ok(ServiceWindow::Open {
                open: CustodyTime::from_epoch_millis(0),
                close: CustodyTime::from_epoch_millis(i64::MAX / 2),
            })
        })
    }

    fn utc_offset_minutes<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _at: CustodyTime,
    ) -> BoxFuture<'a, AppResult<i32>> {
        Box::pin(async { Ok(420) })
    }
}

fn shifts_with(db: &TestDb, hook: Arc<Recorder>) -> Service {
    db.service().with_lifecycle_hook(hook)
}

fn branch_day(db: &TestDb, shifts: Service) -> BranchDayService {
    BranchDayService::new(
        Arc::new(PgCustodyRepository::new(db.pool.clone())),
        shifts,
        Arc::new(Ports),
    )
}

fn manager(branch: &BranchId) -> Actor {
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Actor::new(
        "bm-4136",
        ORG,
        vec![ScopedRole::new(
            Role::BranchManager,
            Some(branch.clone()),
            from,
            None,
        )],
        None,
    )
}

async fn day_open(db: &TestDb, branch: &BranchId) {
    sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
        .bind(branch.as_str())
        .execute(&db.pool)
        .await
        .expect("an OPEN branch day");
}

async fn stored_phase(db: &TestDb, branch: &BranchId) -> Option<String> {
    sqlx::query_scalar("SELECT phase FROM ops_state WHERE branch_id = $1")
        .bind(branch.as_str())
        .fetch_optional(&db.pool)
        .await
        .expect("read the day's phase")
}

async fn open_shift(db: &TestDb, branch: &BranchId, opener: &str) -> String {
    db.seed_open_shift(
        branch,
        opener,
        &shift_service_common::unique("open"),
        &[],
        at(0, 0),
    )
    .await
}

// ------------------------------------------------------------ row 11

/// Row 11: every CLOSE was verified while the day was OPEN, so the cut-off itself closes the day. The hook is
/// called once, on the cut-off's transaction (it reads 0 OPEN there).
#[tokio::test]
async fn row11_a_cut_off_with_no_open_shift_calls_the_hook_once() {
    let db = TestDb::start().await;
    let branch = db.branch("co11").await;
    day_open(&db, &branch).await;
    let hook = Arc::new(Recorder::default());
    let service = branch_day(&db, shifts_with(&db, hook.clone()));

    let mut tx = db.pool.begin().await.expect("begin");
    within(
        "cut_off",
        service.cut_off(&mut tx, &manager(&branch), &branch, at(900, 0)),
    )
    .await
    .expect("cut_off");
    assert_eq!(
        hook.calls(),
        vec![(branch.as_str().to_owned(), 0)],
        "called once, on the cut-off's transaction"
    );
    tx.commit().await.expect("commit");
    assert_eq!(stored_phase(&db, &branch).await.as_deref(), Some("CUT_OFF"));
}

// ------------------------------------------------------------ row 12

/// Row 12: a shift is still OPEN at cut-off, so the day is not closed yet; that shift's later CLOSE is row 1.
#[tokio::test]
async fn row12_a_cut_off_with_an_open_shift_does_not_call_the_hook() {
    let db = TestDb::start().await;
    let branch = db.branch("co12").await;
    day_open(&db, &branch).await;
    open_shift(&db, &branch, "a").await;
    let hook = Arc::new(Recorder::default());
    let service = branch_day(&db, shifts_with(&db, hook.clone()));

    let mut tx = db.pool.begin().await.expect("begin");
    within(
        "cut_off",
        service.cut_off(&mut tx, &manager(&branch), &branch, at(900, 0)),
    )
    .await
    .expect("cut_off");
    tx.commit().await.expect("commit");

    assert!(hook.calls().is_empty(), "not called while a shift is OPEN");
    assert_eq!(stored_phase(&db, &branch).await.as_deref(), Some("CUT_OFF"));
}

// ------------------------------------------------------------ row 13

/// Row 13: the hook's error is the cut-off's error; the caller rolls back and the day is still OPEN.
#[tokio::test]
async fn row13_the_hooks_error_fails_the_cut_off() {
    let db = TestDb::start().await;
    let branch = db.branch("co13").await;
    day_open(&db, &branch).await;
    let hook = Arc::new(Recorder {
        calls: Mutex::default(),
        fail: true,
    });
    let service = branch_day(&db, shifts_with(&db, hook.clone()));

    let mut tx = db.pool.begin().await.expect("begin");
    let answer = within(
        "cut_off",
        service.cut_off(&mut tx, &manager(&branch), &branch, at(900, 0)),
    )
    .await;
    assert!(
        answer.is_err(),
        "the hook's error is answered, got {answer:?}"
    );
    tx.rollback().await.expect("rollback");

    assert_eq!(hook.calls().len(), 1, "the hook was asked once");
    assert_eq!(
        stored_phase(&db, &branch).await.as_deref(),
        Some("OPEN"),
        "the rolled-back cut-off leaves the day OPEN"
    );
}

// ------------------------------------------------------------ row 14

/// Row 14, the verify first: the last CLOSE verify holds the `ops_state` lock (it read OPEN, so it did not fire);
/// the cut-off waits on it, then sees 0 OPEN and fires. Exactly one call.
#[tokio::test]
async fn row14_the_last_verify_then_a_concurrent_cut_off_fire_once() {
    let db = TestDb::start().await;
    let branch = db.branch("co14a").await;
    day_open(&db, &branch).await;
    let shift = open_shift(&db, &branch, "a").await;
    let hook = Arc::new(Recorder::default());
    let shifts = shifts_with(&db, hook.clone());
    let service = branch_day(&db, shifts.clone());

    let mut verify = db.pool.begin().await.expect("begin the verify");
    shifts
        .close_on_verified(&mut verify, &shift, "closer", &[], "", at(600, 0))
        .await
        .expect("close the last shift");
    let fired = within(
        "the verify asks",
        shifts.close_day_if_last_in(&mut verify, &branch, at(600, 0)),
    )
    .await
    .expect("the verify asks");
    assert!(!fired, "the day is still OPEN at the verify");

    let (cutter, b) = (manager(&branch), branch.clone());
    let pool = db.pool.clone();
    let cut = tokio::spawn(async move {
        let mut tx = pool.begin().await.expect("begin the cut-off");
        let answer = service.cut_off(&mut tx, &cutter, &b, at(601, 0)).await;
        if answer.is_ok() {
            tx.commit().await.expect("commit the cut-off");
        }
        answer.map(|_| ())
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !cut.is_finished(),
        "the cut-off waits on the ops_state row the verify holds"
    );
    verify.commit().await.expect("commit the verify");

    within("the cut-off", cut)
        .await
        .expect("the cut-off's task")
        .expect("cut_off");
    assert_eq!(
        hook.calls(),
        vec![(branch.as_str().to_owned(), 0)],
        "exactly one call"
    );
}

/// Row 14, the cut-off first: the cut-off stores CUT_OFF while the last shift's close is not yet committed, so it
/// counts that shift OPEN and does not fire; the verify then waits on the row the cut-off wrote, sees CUT_OFF and
/// 0 OPEN, and fires. Exactly one call.
#[tokio::test]
async fn row14_a_cut_off_then_the_concurrent_last_verify_fire_once() {
    let db = TestDb::start().await;
    let branch = db.branch("co14b").await;
    day_open(&db, &branch).await;
    let shift = open_shift(&db, &branch, "a").await;
    let hook = Arc::new(Recorder::default());
    let shifts = shifts_with(&db, hook.clone());
    let service = branch_day(&db, shifts.clone());

    let mut verify = db.pool.begin().await.expect("begin the verify");
    shifts
        .close_on_verified(&mut verify, &shift, "closer", &[], "", at(600, 0))
        .await
        .expect("close the last shift, not committed yet");

    let mut cut = db.pool.begin().await.expect("begin the cut-off");
    within(
        "cut_off",
        service.cut_off(&mut cut, &manager(&branch), &branch, at(601, 0)),
    )
    .await
    .expect("cut_off");
    assert!(
        hook.calls().is_empty(),
        "the cut-off still sees the uncommitted shift OPEN"
    );

    let (s, b) = (shifts.clone(), branch.clone());
    let ask = tokio::spawn(async move {
        let fired = s.close_day_if_last_in(&mut verify, &b, at(602, 0)).await;
        if fired.is_ok() {
            verify.commit().await.expect("commit the verify");
        }
        fired
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !ask.is_finished(),
        "the verify waits on the ops_state row the cut-off wrote"
    );
    cut.commit().await.expect("commit the cut-off");

    let fired = within("the verify asks", ask)
        .await
        .expect("the verify's task")
        .expect("the verify asks");
    assert!(
        fired,
        "after the cut-off commits the verify sees CUT_OFF and 0 OPEN"
    );
    assert_eq!(
        hook.calls(),
        vec![(branch.as_str().to_owned(), 0)],
        "exactly one call"
    );
}
