//! Staff corrections §4 A, rows A1, A2, A11, A12 (`docs/design/staff-corrections.md` SC-D2..SC-D5):
//! `BranchDayService::close_for` against a real PostgreSQL, on a transaction that already holds the branch lock
//! (as the seam runs a command). A3–A10 themselves are `close_for_check_test.rs`; here one refusal shows they
//! reach the caller and write nothing, and the branch timezone comes from `branches.timezone`.
//!
//! Written before the implementation by the sub-controller (SC-0, claude/studio-claude-4). Do not edit, delete or
//! weaken these tests; add your own in another file. Needs `TEST_DATABASE_URL`; nothing here skips.

mod shift_service_common;

use std::sync::Arc;

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use xc_auth::domain::access::{Actor, Refusal, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_shift::application::branch_day::service::{BranchDayService, ClockDirection};
use xc_shift::domain::custody::repositories::custody_ports::{CustodyPorts, OutsideFacts};
use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
use xc_shift::domain::custody::value_objects::close_for::{
    CloseFor, CloseForError, CloseForRefusal,
};
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use shift_service_common::TestDb;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const MANAGER_ID: &str = "00000000-0000-4000-8000-0000000000a1";
const TELLER_ID: &str = "00000000-0000-4000-8000-0000000000a2";
const OTHER_MANAGER_ID: &str = "00000000-0000-4000-8000-0000000000a3";

fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, mo, d, h, mi, 0)
        .single()
        .expect("a valid time")
}

/// The teller's forgotten clock-in: 2026-10-26 09:00 ICT.
fn clocked_in_at() -> DateTime<Utc> {
    utc(2026, 10, 26, 2, 0)
}

/// The manager acts on 2026-10-28 10:00 ICT.
fn now() -> DateTime<Utc> {
    utc(2026, 10, 28, 3, 0)
}

/// The real clock-out the manager types: 2026-10-26 18:00 ICT.
fn entered() -> DateTime<Utc> {
    utc(2026, 10, 26, 11, 0)
}

fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn person(user_id: &str, role: Role, branch: &BranchId) -> Actor {
    Actor::new(
        user_id,
        ORG,
        vec![ScopedRole::new(
            role,
            Some(branch.clone()),
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            None,
        )],
        None,
    )
}

struct TestPorts;

impl CustodyPorts for TestPorts {
    fn outside_facts<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _open_shift_id: Option<&'a str>,
    ) -> BoxFuture<'a, AppResult<OutsideFacts>> {
        Box::pin(async {
            Ok(OutsideFacts {
                has_live_rate: true,
                rates_updated: "2026-10-11T08:00:00Z".to_string(),
                ..Default::default()
            })
        })
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

fn make_service(db: &TestDb) -> BranchDayService<PgCustodyRepository, PgShiftRepository> {
    let repo = Arc::new(PgCustodyRepository::new(db.pool.clone()));
    BranchDayService::new(repo, db.service(), Arc::new(TestPorts))
}

/// A transaction holding the branch lock, as the seam opens one (`BranchTx::run`).
async fn locked_tx(pool: &PgPool, branch: &BranchId) -> Transaction<'static, Postgres> {
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query(
        "INSERT INTO branch_versions (branch_id, version) VALUES ($1, 0) ON CONFLICT (branch_id) DO NOTHING",
    )
    .bind(branch.as_str())
    .execute(&mut *tx)
    .await
    .expect("branch_versions row");
    sqlx::query("SELECT version FROM branch_versions WHERE branch_id = $1 FOR UPDATE")
        .bind(branch.as_str())
        .execute(&mut *tx)
        .await
        .expect("lock the branch");
    tx
}

/// The teller clocks in at `clocked_in_at()`; answers the attendance id.
async fn forgotten_clock_in(db: &TestDb, service: &BranchDayService, branch: &BranchId) -> String {
    let teller = person(TELLER_ID, Role::Teller, branch);
    let mut tx = locked_tx(&db.pool, branch).await;
    service
        .clock(
            &mut tx,
            &teller,
            branch,
            ClockDirection::In,
            clocked_in_at(),
        )
        .await
        .expect("clock in");
    tx.commit().await.expect("commit");
    sqlx::query_scalar("SELECT id FROM attendance WHERE branch_id = $1 AND employee_id = $2")
        .bind(branch.as_str())
        .bind(TELLER_ID)
        .fetch_one(&db.pool)
        .await
        .expect("the attendance row")
}

/// `close_for` on a locked transaction: commit on `Ok`, roll back otherwise.
async fn close_for(
    db: &TestDb,
    service: &BranchDayService,
    who: &Actor,
    branch: &BranchId,
    cmd: CloseFor,
    at: DateTime<Utc>,
) -> Result<xc_shift::domain::custody::entities::attendance::Attendance, CloseForError> {
    let mut tx = locked_tx(&db.pool, branch).await;
    let answer = service.close_for(&mut tx, who, branch, cmd, at).await;
    if answer.is_ok() {
        tx.commit().await.expect("commit");
    } else {
        tx.rollback().await.expect("rollback");
    }
    answer
}

fn cmd(id: &str, clock_out: DateTime<Utc>, reason: &str) -> CloseFor {
    CloseFor {
        attendance_id: id.to_owned(),
        clock_out,
        reason: reason.to_owned(),
    }
}

/// `(kind, actor, details, at)` of every event of `branch`, oldest first.
async fn events(db: &TestDb, branch: &BranchId) -> Vec<(String, String, Value, DateTime<Utc>)> {
    sqlx::query_as(
        "SELECT kind, actor, details, at FROM ops_events WHERE branch_id = $1 ORDER BY seq",
    )
    .bind(branch.as_str())
    .fetch_all(&db.pool)
    .await
    .expect("read ops_events")
}

async fn clock_out_of(db: &TestDb, branch: &BranchId, id: &str) -> Option<i64> {
    let state = PgCustodyRepository::new(db.pool.clone())
        .load_state(branch)
        .await
        .expect("load the branch state");
    state
        .attendance
        .iter()
        .find(|a| a.id == id)
        .unwrap_or_else(|| panic!("attendance {id}"))
        .clock_out
        .map(CustodyTime::epoch_millis)
}

// A11: the row is closed at the entered time, one CLOCK_OUT_FOR records who, whom, why, both times, and when.
#[tokio::test]
async fn a_manager_closes_a_forgotten_clock_in() {
    let db = TestDb::start().await;
    let branch = db.branch("sc_close").await;
    let service = make_service(&db);
    let id = forgotten_clock_in(&db, &service, &branch).await;
    let manager = person(MANAGER_ID, Role::BranchManager, &branch);
    let before = events(&db, &branch).await.len();

    let row = close_for(
        &db,
        &service,
        &manager,
        &branch,
        cmd(&id, entered(), " ลืมลงเวลาออก "),
        now(),
    )
    .await
    .expect("the manager closes it");

    assert_eq!(row.id, id);
    assert_eq!(row.employee_id, TELLER_ID);
    assert_eq!(
        row.clock_out.map(CustodyTime::epoch_millis),
        Some(entered().timestamp_millis())
    );
    assert_eq!(
        clock_out_of(&db, &branch, &id).await,
        Some(entered().timestamp_millis()),
        "stored"
    );

    let after = events(&db, &branch).await;
    assert_eq!(after.len(), before + 1, "exactly one new event");
    let (kind, actor, details, at) = after.last().cloned().expect("the new event");
    assert_eq!(kind, "CLOCK_OUT_FOR");
    assert_eq!(actor, MANAGER_ID, "the manager is the actor");
    assert_eq!(at, now(), "recorded at the time of the correction");
    assert_eq!(details["attendanceId"], id.as_str());
    assert_eq!(details["employeeId"], TELLER_ID);
    assert_eq!(details["clockIn"], stamp(clocked_in_at()).as_str());
    assert_eq!(
        details["clockOut"],
        stamp(entered()).as_str(),
        "the entered time"
    );
    assert_eq!(details["reason"], "ลืมลงเวลาออก", "trimmed");
    db.drop_schema().await;
}

// A1: no AttendanceCloseFor (a teller) → roleNotAllowed; a manager of another branch → outOfScope. Nothing written.
#[tokio::test]
async fn without_the_permission_on_the_branch_it_is_refused() {
    let db = TestDb::start().await;
    let branch = db.branch("sc_perm").await;
    let elsewhere = db.branch("sc_perm_other").await;
    let service = make_service(&db);
    let id = forgotten_clock_in(&db, &service, &branch).await;
    let before = events(&db, &branch).await.len();

    let teller = person(
        "00000000-0000-4000-8000-0000000000a4",
        Role::Teller,
        &branch,
    );
    match close_for(
        &db,
        &service,
        &teller,
        &branch,
        cmd(&id, entered(), "why"),
        now(),
    )
    .await
    {
        Err(CloseForError::NotAllowed(Refusal::RoleNotAllowed)) => {}
        other => panic!("a teller: expected NotAllowed(RoleNotAllowed), got {other:?}"),
    }
    let other = person(OTHER_MANAGER_ID, Role::BranchManager, &elsewhere);
    match close_for(
        &db,
        &service,
        &other,
        &branch,
        cmd(&id, entered(), "why"),
        now(),
    )
    .await
    {
        Err(CloseForError::NotAllowed(Refusal::OutOfScope)) => {}
        other => panic!("another branch's manager: expected NotAllowed(OutOfScope), got {other:?}"),
    }
    assert_eq!(clock_out_of(&db, &branch, &id).await, None, "still open");
    assert_eq!(events(&db, &branch).await.len(), before, "nothing written");
    db.drop_schema().await;
}

// A2: an unknown id, and a row of another branch, are not found here.
#[tokio::test]
async fn an_unknown_row_or_another_branchs_row_is_not_found() {
    let db = TestDb::start().await;
    let branch = db.branch("sc_nf").await;
    let elsewhere = db.branch("sc_nf_other").await;
    let service = make_service(&db);
    let theirs = forgotten_clock_in(&db, &service, &elsewhere).await;
    let manager = person(MANAGER_ID, Role::BranchManager, &branch);

    for id in ["no-such-row", theirs.as_str()] {
        match close_for(
            &db,
            &service,
            &manager,
            &branch,
            cmd(id, entered(), "why"),
            now(),
        )
        .await
        {
            Err(CloseForError::NotFound) => {}
            other => panic!("{id}: expected NotFound, got {other:?}"),
        }
    }
    assert_eq!(clock_out_of(&db, &elsewhere, &theirs).await, None);
    db.drop_schema().await;
}

// A3–A10 reach the caller as Refused and write nothing (one of them here; all of them in close_for_check_test).
#[tokio::test]
async fn a_refusal_of_the_check_writes_nothing() {
    let db = TestDb::start().await;
    let branch = db.branch("sc_ref").await;
    let service = make_service(&db);
    let id = forgotten_clock_in(&db, &service, &branch).await;
    let manager = person(MANAGER_ID, Role::BranchManager, &branch);
    let before = events(&db, &branch).await.len();

    match close_for(
        &db,
        &service,
        &manager,
        &branch,
        cmd(&id, entered(), "   "),
        now(),
    )
    .await
    {
        Err(CloseForError::Refused(CloseForRefusal::ReasonRequired)) => {}
        other => panic!("expected Refused(ReasonRequired), got {other:?}"),
    }
    assert_eq!(clock_out_of(&db, &branch, &id).await, None);
    assert_eq!(events(&db, &branch).await.len(), before);
    db.drop_schema().await;
}

// SC-D4: "today" is the branch's, from `branches.timezone`. At 2026-10-28 00:30 ICT (17:30 UTC on the 27th),
// a clock-out at 2026-10-27 23:30 ICT (16:30 UTC) is an earlier day in Bangkok but today on a UTC branch.
#[tokio::test]
async fn the_branch_timezone_decides_the_earlier_day() {
    let db = TestDb::start().await;
    let bangkok = db.branch("sc_tz_bkk").await;
    let on_utc = db.branch("sc_tz_utc").await;
    sqlx::query("UPDATE branches SET timezone = 'UTC' WHERE id = $1")
        .bind(on_utc.as_str())
        .execute(&db.pool)
        .await
        .expect("put the branch on UTC");
    let service = make_service(&db);
    let late_now = utc(2026, 10, 27, 17, 30);
    let late_out = utc(2026, 10, 27, 16, 30);

    let id = forgotten_clock_in(&db, &service, &bangkok).await;
    let manager = person(MANAGER_ID, Role::BranchManager, &bangkok);
    close_for(
        &db,
        &service,
        &manager,
        &bangkok,
        cmd(&id, late_out, "why"),
        late_now,
    )
    .await
    .expect("an earlier day in Bangkok");

    let id = forgotten_clock_in(&db, &service, &on_utc).await;
    let manager = person(MANAGER_ID, Role::BranchManager, &on_utc);
    match close_for(
        &db,
        &service,
        &manager,
        &on_utc,
        cmd(&id, late_out, "why"),
        late_now,
    )
    .await
    {
        Err(CloseForError::Refused(CloseForRefusal::NotEarlierDay)) => {}
        other => panic!("today on a UTC branch: expected NotEarlierDay, got {other:?}"),
    }
    db.drop_schema().await;
}

// A12: after the correction the employee clocks in again and out again as before.
#[tokio::test]
async fn the_employee_clocks_as_before_after_a_correction() {
    let db = TestDb::start().await;
    let branch = db.branch("sc_again").await;
    let service = make_service(&db);
    let id = forgotten_clock_in(&db, &service, &branch).await;
    let manager = person(MANAGER_ID, Role::BranchManager, &branch);
    close_for(
        &db,
        &service,
        &manager,
        &branch,
        cmd(&id, entered(), "forgot"),
        now(),
    )
    .await
    .expect("close it");

    let teller = person(TELLER_ID, Role::Teller, &branch);
    let mut tx = locked_tx(&db.pool, &branch).await;
    service
        .clock(&mut tx, &teller, &branch, ClockDirection::In, now())
        .await
        .expect("clock in again");
    tx.commit().await.expect("commit");
    let open: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM attendance WHERE branch_id = $1 AND employee_id = $2 AND data->>'clockOut' IS NULL",
    )
    .bind(branch.as_str())
    .bind(TELLER_ID)
    .fetch_all(&db.pool)
    .await
    .expect("open rows");
    assert_eq!(open.len(), 1, "a new open row");
    assert_ne!(open[0], id, "not the corrected one");

    let mut tx = locked_tx(&db.pool, &branch).await;
    service
        .clock(
            &mut tx,
            &teller,
            &branch,
            ClockDirection::Out,
            utc(2026, 10, 28, 10, 0),
        )
        .await
        .expect("clock out as before");
    tx.commit().await.expect("commit");
    assert_eq!(
        clock_out_of(&db, &branch, &open[0]).await,
        Some(utc(2026, 10, 28, 10, 0).timestamp_millis())
    );
    let kinds: Vec<String> = events(&db, &branch)
        .await
        .into_iter()
        .map(|e| e.0)
        .collect();
    assert_eq!(
        kinds,
        ["CLOCK_IN", "CLOCK_OUT_FOR", "CLOCK_IN", "CLOCK_OUT"]
    );
    db.drop_schema().await;
}
