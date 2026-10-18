//! Ticket #186 (the custody half of #94) — a custody row the server cannot decode is read as a
//! default instead of as a fault. Written before the fix. Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! `PgCustodyRepository` loads custody from rows whose fields live mostly in `jsonb`. Where a
//! stored value cannot be decoded the load substitutes one — an unknown currency or amount is
//! dropped to "none", an unknown role becomes `Other` (a missing one `Front`), an unreadable time
//! becomes 1970-01-01, an unknown request or event kind a fixed one — and custody then decides
//! who holds the cash from the substitute. The rule (#94): a stored value that is **present and
//! cannot be decoded** is a fault — an `AppError` other than `Domain` — never a substitute. A
//! field that is **absent by design** (no target yet, a request with no currency, no clock-out
//! yet) is not a decode failure and still loads.
//!
//! Each test writes a day's rows, then spoils one stored value with plain SQL, as a hand-edited
//! row, a value removed from a register or a future writer's bug would.
//!
//! Re-expected for CA-10 #626 by the ticket author (ruling Q7 A, design `cash-shift-transfer.md`
//! §6): the custody command engine that built the day (`Desk::serving`, the `SAFE_IN` proposal)
//! is retired, so the day is written by SQL in the shapes `PgCustodyRepository` writes
//! (`attendance_data`, the `ops_events` insert). The request half of the absent-fields case retires
//! with the request model.
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

mod custody_common;
mod custody_service_common;
mod shift_service_common;

use custody_service_common::Desk;
use pos_error::{AppError, AppResult};
use shift_service_common::TestDb;

fn a_fault<T: std::fmt::Debug>(read: AppResult<T>, what: &str) {
    match read {
        Err(AppError::Domain(sentence)) => {
            panic!("{what}: a fault is not a refusal, got Domain({sentence:?})")
        }
        Err(_) => {}
        Ok(value) => panic!("{what}: an undecodable value was read as {value:?}"),
    }
}

/// A day with F1 clocked in (no clock-out yet) and its `CLOCK_IN` event.
async fn a_day(db: &TestDb) -> Desk {
    let d = Desk::new(db).await;
    for sql in [
        // the load reads a branch's rows only once it has custody at all
        "INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN') \
         ON CONFLICT (branch_id) DO NOTHING",
        "INSERT INTO attendance (id, branch_id, position, employee_id, data) \
         VALUES ('attendance-1', $1, 0, 'F1', jsonb_build_object('role', 'FRONT', \
           'clockIn', $2::bigint, 'clockOut', NULL))",
        "INSERT INTO ops_events (id, branch_id, kind, at, actor, role, details) \
         VALUES ('event-1', $1, 'CLOCK_IN', to_timestamp($2::bigint / 1000.0), 'F1', 'FRONT', '{}')",
    ] {
        sqlx::query(sql)
            .bind(d.branch.as_str())
            .bind(custody_common::NOON)
            .execute(&db.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    d
}

async fn spoil(db: &TestDb, sql: &'static str, d: &Desk) {
    let done = sqlx::query(sql)
        .bind(d.branch.as_str())
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    assert!(done.rows_affected() > 0, "{sql}: no row was spoiled");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_attendance_role_custody_does_not_know_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_day(&db).await;
    spoil(
        &db,
        "UPDATE attendance SET data = jsonb_set(data, '{role}', '\"NOBODY\"') \
         WHERE branch_id = $1 AND employee_id = 'F1'",
        &d,
    )
    .await;
    a_fault(
        d.custody.state(&d.branch).await,
        "an attendance role NOBODY",
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_attendance_time_that_is_not_a_time_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_day(&db).await;
    spoil(
        &db,
        "UPDATE attendance SET data = jsonb_set(data, '{clockIn}', '\"yesterday\"') \
         WHERE branch_id = $1 AND employee_id = 'F1'",
        &d,
    )
    .await;
    a_fault(
        d.custody.state(&d.branch).await,
        "a clock-in of \"yesterday\"",
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_event_of_a_kind_custody_does_not_know_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_day(&db).await;
    spoil(
        &db,
        "UPDATE ops_events SET kind = 'NOPE' WHERE branch_id = $1",
        &d,
    )
    .await;
    a_fault(d.custody.events(&d.branch).await, "an event of kind NOPE");
    db.drop_schema().await;
}

/// Holds before the fix and after it: what is absent by design still loads — attendance with no
/// clock-out, no target, no cutoff — and the values that decode are read as written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn absent_fields_are_not_faults_and_decodable_rows_read_as_written() {
    let db = TestDb::start().await;
    let d = a_day(&db).await;
    let state = d
        .custody
        .state(&d.branch)
        .await
        .expect("a day that decodes loads");
    assert_eq!(state.attendance.len(), 1, "F1's attendance: {state:?}");
    assert!(
        state.target.is_none() && state.cutoff.is_none(),
        "no target, no cutoff"
    );
    assert!(
        state.attendance.iter().all(|a| a.clock_out.is_none()),
        "nobody has clocked out"
    );
    let events = d
        .custody
        .events(&d.branch)
        .await
        .expect("the events of a day that decodes load");
    assert_eq!(events.len(), 1, "the CLOCK_IN: {events:?}");
    db.drop_schema().await;
}
