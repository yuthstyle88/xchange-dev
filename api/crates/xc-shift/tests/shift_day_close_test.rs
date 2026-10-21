//! #4136 S1: the branch day closes with the last shift. Design `docs/design/blueprint/queue-day-close.md` §2 rows 1–3,
//! 7, 8 and §3.2: `BranchDay::day_closes_on_shift_close` and `ShiftService::close_day_if_last_in`.
//!
//! Written before the implementation by the ticket author (claude/studio-claude-9). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Each test closes a shift the way the CLOSE verify does (`close_on_verified` on the caller's transaction), then
//! asks `close_day_if_last_in` on the same transaction. A recording hook stands in for the queue.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

mod shift_service_common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use shift_service_common::{Service, TestDb, at, within};
use xc_kernel::ids::BranchId;
use xc_kernel::shift_hook::ShiftLifecycleHook;
use xc_shift::domain::custody::branch_day::BranchDay;
use xc_shift::domain::custody::value_objects::phase::Phase;

/// Records every `on_day_closed` with the OPEN-shift count it reads **on the caller's transaction**, and answers
/// an error when `fail` is set.
#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<(String, i64)>>,
    fail: bool,
}

impl Recorder {
    fn failing() -> Self {
        Self {
            calls: Mutex::default(),
            fail: true,
        }
    }

    fn calls(&self) -> Vec<(String, i64)> {
        self.calls.lock().expect("recorder lock").clone()
    }
}

impl ShiftLifecycleHook for Recorder {
    fn on_day_closed<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
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

fn service_with(db: &TestDb, hook: Arc<Recorder>) -> Service {
    db.service().with_lifecycle_hook(hook)
}

async fn store_phase(db: &TestDb, branch: &BranchId, phase: &str) {
    sqlx::query(
        "INSERT INTO ops_state (branch_id, phase) VALUES ($1, $2) \
         ON CONFLICT (branch_id) DO UPDATE SET phase = EXCLUDED.phase",
    )
    .bind(branch.as_str())
    .bind(phase)
    .execute(&db.pool)
    .await
    .expect("store the day's phase");
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

/// The CLOSE verify's order on one transaction: close the shift, then ask. Commits and answers the step's result.
async fn close_and_ask(
    service: &Service,
    db: &TestDb,
    branch: &BranchId,
    shift: &str,
) -> AppResult<bool> {
    let mut tx = db.pool.begin().await.expect("begin");
    service
        .close_on_verified(&mut tx, shift, "closer", &[], "", at(600, 0))
        .await
        .expect("close the shift");
    let fired = within(
        "close_day_if_last_in",
        service.close_day_if_last_in(&mut tx, branch, at(600, 0)),
    )
    .await;
    match fired {
        Ok(f) => {
            tx.commit().await.expect("commit");
            Ok(f)
        }
        Err(e) => {
            tx.rollback().await.expect("rollback");
            Err(e)
        }
    }
}

// ------------------------------------------------------------ the rule (no I/O)

/// §3.2: only a stored CUT_OFF with no OPEN shift left closes the day. A stored CLOSED is not a close, unlike
/// `read_phase`.
#[test]
fn the_rule_is_cut_off_and_no_open_shift() {
    let cases = [
        (Phase::CutOff, 0, true),
        (Phase::CutOff, 1, false),
        (Phase::CutOff, 3, false),
        (Phase::Open, 0, false),
        (Phase::Ready, 0, false),
        (Phase::Closed, 0, false),
        (Phase::Open, 2, false),
    ];
    for (stored, open_after, expected) in cases {
        assert_eq!(
            BranchDay::day_closes_on_shift_close(stored, open_after),
            expected,
            "stored {stored:?}, {open_after} OPEN after the close"
        );
    }
}

// ------------------------------------------------------------ rows 1 and 2

/// Rows 1 and 2: after cut-off, closing the first of two shifts does not close the day; closing the last one calls
/// the hook once, on the caller's transaction (it reads 0 OPEN there).
#[tokio::test]
async fn row1_2_the_last_close_after_cut_off_calls_the_hook_once() {
    let db = TestDb::start().await;
    let branch = db.branch("dc12").await;
    store_phase(&db, &branch, "CUT_OFF").await;
    let a = open_shift(&db, &branch, "a").await;
    let b = open_shift(&db, &branch, "b").await;
    let hook = Arc::new(Recorder::default());
    let service = service_with(&db, hook.clone());

    let first = close_and_ask(&service, &db, &branch, &a)
        .await
        .expect("first close");
    assert!(!first, "another shift is still OPEN");
    assert!(hook.calls().is_empty(), "not called while a shift is OPEN");

    let last = close_and_ask(&service, &db, &branch, &b)
        .await
        .expect("last close");
    assert!(last, "the last close after cut-off closes the day");
    assert_eq!(
        hook.calls(),
        vec![(branch.as_str().to_owned(), 0)],
        "called once, for this branch, on the transaction that closed the last shift"
    );
    assert_eq!(
        stored_phase(&db, &branch).await.as_deref(),
        Some("CUT_OFF"),
        "the step writes nothing to ops_state"
    );
}

/// Row 1: another branch's OPEN shift does not keep this branch's day open.
#[tokio::test]
async fn row1_other_branches_shifts_do_not_count() {
    let db = TestDb::start().await;
    let branch = db.branch("dcown").await;
    let other = db.branch("dcother").await;
    store_phase(&db, &branch, "CUT_OFF").await;
    store_phase(&db, &other, "CUT_OFF").await;
    let mine = open_shift(&db, &branch, "a").await;
    open_shift(&db, &other, "b").await;
    let hook = Arc::new(Recorder::default());
    let service = service_with(&db, hook.clone());

    assert!(
        close_and_ask(&service, &db, &branch, &mine)
            .await
            .expect("close")
    );
    assert_eq!(hook.calls(), vec![(branch.as_str().to_owned(), 0)]);
}

// ------------------------------------------------------------ row 3

/// Row 3: before cut-off (OPEN), at READY, at a stored CLOSED, or with no `ops_state` row, the last close does not
/// close the day.
#[tokio::test]
async fn row3_no_day_close_unless_the_stored_phase_is_cut_off() {
    let db = TestDb::start().await;
    for stored in [Some("OPEN"), Some("READY"), Some("CLOSED"), None] {
        let branch = db.branch("dc3").await;
        if let Some(phase) = stored {
            store_phase(&db, &branch, phase).await;
        }
        let shift = open_shift(&db, &branch, "a").await;
        let hook = Arc::new(Recorder::default());
        let service = service_with(&db, hook.clone());

        let fired = close_and_ask(&service, &db, &branch, &shift)
            .await
            .unwrap_or_else(|e| panic!("stored {stored:?}: {e}"));
        assert!(!fired, "stored {stored:?}: not a day close");
        assert!(
            hook.calls().is_empty(),
            "stored {stored:?}: hook not called"
        );
        assert_eq!(
            stored_phase(&db, &branch).await.as_deref(),
            stored,
            "stored {stored:?}: ops_state untouched"
        );
    }
}

// ------------------------------------------------------------ the hook's error

/// Row 5 (the shift half): a hook error is answered by the step, so the caller's transaction can roll back.
#[tokio::test]
async fn row5_the_hooks_error_is_answered() {
    let db = TestDb::start().await;
    let branch = db.branch("dc5").await;
    store_phase(&db, &branch, "CUT_OFF").await;
    let shift = open_shift(&db, &branch, "a").await;
    let hook = Arc::new(Recorder::failing());
    let service = service_with(&db, hook.clone());

    let answer = close_and_ask(&service, &db, &branch, &shift).await;
    assert!(
        answer.is_err(),
        "the hook's error is answered, got {answer:?}"
    );
    assert_eq!(hook.calls().len(), 1, "the hook was asked once");
    let status: String = sqlx::query_scalar("SELECT status FROM shifts WHERE id = $1")
        .bind(&shift)
        .fetch_one(&db.pool)
        .await
        .expect("read the shift");
    assert_eq!(
        status, "OPEN",
        "the caller rolled back, so the shift is still OPEN"
    );
}

// ------------------------------------------------------------ row 7

/// Row 7: asking again in the same state answers `true` again and writes nothing to `ops_state` or `shifts`.
#[tokio::test]
async fn row7_a_second_ask_writes_nothing() {
    let db = TestDb::start().await;
    let branch = db.branch("dc7").await;
    store_phase(&db, &branch, "CUT_OFF").await;
    let shift = open_shift(&db, &branch, "a").await;
    let hook = Arc::new(Recorder::default());
    let service = service_with(&db, hook.clone());
    assert!(
        close_and_ask(&service, &db, &branch, &shift)
            .await
            .expect("close")
    );

    let shifts_before: Vec<(String, String)> =
        sqlx::query_as("SELECT id, status FROM shifts WHERE branch_id = $1 ORDER BY id")
            .bind(branch.as_str())
            .fetch_all(&db.pool)
            .await
            .expect("read shifts");
    let mut tx = db.pool.begin().await.expect("begin");
    let again = service
        .close_day_if_last_in(&mut tx, &branch, at(700, 0))
        .await
        .expect("ask again");
    tx.commit().await.expect("commit");

    assert!(again, "the same state answers the same");
    assert_eq!(stored_phase(&db, &branch).await.as_deref(), Some("CUT_OFF"));
    let shifts_after: Vec<(String, String)> =
        sqlx::query_as("SELECT id, status FROM shifts WHERE branch_id = $1 ORDER BY id")
            .bind(branch.as_str())
            .fetch_all(&db.pool)
            .await
            .expect("read shifts");
    assert_eq!(shifts_after, shifts_before);
}

// ------------------------------------------------------------ row 8

/// Row 8: two tellers close at once, each on a transaction of its own. The first to ask sees the other's shift still
/// OPEN; the second waits on the `ops_state` lock until the first commits, then sees 0 OPEN and calls the hook.
/// Without the lock both would read the other's uncommitted shift as OPEN, and nobody would close the day.
#[tokio::test]
async fn row8_two_concurrent_last_closes_fire_exactly_once() {
    let db = TestDb::start().await;
    let branch = db.branch("dc8").await;
    store_phase(&db, &branch, "CUT_OFF").await;
    let a = open_shift(&db, &branch, "a").await;
    let b = open_shift(&db, &branch, "b").await;
    let hook = Arc::new(Recorder::default());
    let service = service_with(&db, hook.clone());

    let mut tx_a = db.pool.begin().await.expect("begin A");
    service
        .close_on_verified(&mut tx_a, &a, "closer", &[], "", at(600, 0))
        .await
        .expect("close A");
    let a_fired = within(
        "A asks",
        service.close_day_if_last_in(&mut tx_a, &branch, at(600, 0)),
    )
    .await
    .expect("A asks");
    assert!(!a_fired, "A still sees B's shift OPEN");

    let mut tx_b = db.pool.begin().await.expect("begin B");
    service
        .close_on_verified(&mut tx_b, &b, "closer", &[], "", at(601, 0))
        .await
        .expect("close B");
    let (service_b, branch_b) = (service.clone(), branch.clone());
    let b_task = tokio::spawn(async move {
        let fired = service_b
            .close_day_if_last_in(&mut tx_b, &branch_b, at(601, 0))
            .await;
        if fired.is_ok() {
            tx_b.commit().await.expect("commit B");
        }
        fired
    });

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !b_task.is_finished(),
        "B waits on the ops_state lock A holds"
    );
    tx_a.commit().await.expect("commit A");

    let b_fired = within("B asks", b_task)
        .await
        .expect("B's task")
        .expect("B asks");
    assert!(b_fired, "B, after A commits, sees 0 OPEN");
    assert_eq!(
        hook.calls(),
        vec![(branch.as_str().to_owned(), 0)],
        "exactly one call"
    );
}
