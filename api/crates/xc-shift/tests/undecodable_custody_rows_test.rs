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
//! Each test builds a real day through the service (`Desk::serving`, plus a `SAFE_IN` proposal
//! where a currency and an amount are needed), then spoils one stored value with plain SQL, as a
//! hand-edited row, a value removed from a register or a future writer's bug would.
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

mod custody_common;
mod custody_service_common;
mod shift_service_common;

use custody_service_common::Desk;
use pos_error::{AppError, AppResult};
use serde_json::json;
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

/// A serving day with one pending `SAFE_IN` of USD 50 from F1 to FIN1.
async fn a_day_with_a_transfer(db: &TestDb) -> Desk {
    let mut d = Desk::serving(db).await;
    d.propose(
        "F1",
        "SAFE_IN",
        json!({ "receiverId": "FIN1", "currency": "USD", "amount": 50 }),
    )
    .await;
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
    let d = a_day_with_a_transfer(&db).await;
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
    let d = a_day_with_a_transfer(&db).await;
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
    let d = a_day_with_a_transfer(&db).await;
    spoil(
        &db,
        "UPDATE ops_events SET kind = 'NOPE' WHERE branch_id = $1",
        &d,
    )
    .await;
    a_fault(d.custody.events(&d.branch).await, "an event of kind NOPE");
    db.drop_schema().await;
}

/// Holds before the fix and after it: what is absent by design still loads — a request with no
/// currency (the day's OPEN), attendance with no clock-out, no target, no cutoff — and the values
/// that decode are read as written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn absent_fields_are_not_faults_and_decodable_rows_read_as_written() {
    let db = TestDb::start().await;
    let d = a_day_with_a_transfer(&db).await;
    let state = d
        .custody
        .state(&d.branch)
        .await
        .expect("a day that decodes loads");
    let transfer = state
        .requests
        .iter()
        .find(|r| r.currency.is_some())
        .expect("the SAFE_IN has its currency");
    assert_eq!(transfer.currency.unwrap().as_str(), "USD");
    assert_eq!(transfer.amount.unwrap().to_string(), "50.00");
    assert!(
        state.requests.iter().any(|r| r.currency.is_none()),
        "the day's OPEN request has no currency, and that is not a fault"
    );
    assert!(
        state.attendance.iter().all(|a| a.clock_out.is_none()),
        "nobody has clocked out"
    );
    d.custody
        .events(&d.branch)
        .await
        .expect("the events of a day that decodes load");
    db.drop_schema().await;
}
