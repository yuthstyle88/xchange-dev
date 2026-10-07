//! Ticket #169 — a custody command that waited for the branch's lock and runs after its sender's
//! account was closed. Written before the fix. Do not edit, delete or weaken; add tests in a file
//! of your own.
//!
//! The staff route reads the session (and so whether the account is active) once, before the
//! command waits for the branch's lock. Under the lock, `CustodyService` asks the ports about the
//! receiver of a proposal and never about the sender. Decided by the lane controller (#169,
//! 2026-10-07T00:00Z): when the sender is an employee whose account is not an active employee of
//! the branch — `CustodyPorts::employee_role` answers `None` inside the command's transaction —
//! the command is answered `AppError::Unauthenticated` (the route turns it into the staff door's
//! 401) and nothing is read into or written to custody. The owner is not asked.
//!
//! The fake's `set_employee(id, None)` is the closed account.
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

mod custody_common;
mod custody_service_common;
mod shift_service_common;

use custody_service_common::Desk;
use pos_error::AppError;
use serde_json::json;
use shift_service_common::TestDb;
use xc_shift::domain::custody::entities::stored_event::StoredEvent;
use xc_shift::domain::custody::state::CustodyState;

/// What the branch holds in custody, compared by value (`CustodyState` and `StoredEvent` are
/// `PartialEq`); never by `Debug` text, whose map order is not fixed.
async fn held(desk: &Desk) -> (CustodyState, Vec<StoredEvent>) {
    let state = desk
        .custody
        .state(&desk.branch)
        .await
        .expect("read the custody state");
    let events = desk
        .custody
        .events(&desk.branch)
        .await
        .expect("read the custody events");
    (state, events)
}

fn closed_session<T: std::fmt::Debug>(answer: Result<T, AppError>, what: &str) {
    match answer {
        Err(AppError::Unauthenticated) => {}
        other => panic!("{what}: expected Err(AppError::Unauthenticated), got {other:?}"),
    }
}

/// Custody on, nobody clocked in; then F1's account is closed and F1's clock-in arrives.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closed_employee_cannot_clock_in() {
    let db = TestDb::start().await;
    let mut desk = Desk::new(&db).await;
    desk.ok(
        "owner",
        "opsSetEntity",
        json!({ "company": "XC", "site": "B", "confirmSeparated": true }),
    )
    .await;
    desk.ok("owner", "opsEnable", json!({})).await;
    desk.outside.set_employee("F1", None);

    let before = held(&desk).await;
    let answer = desk
        .act("F1", "opsClock", json!({ "direction": "IN" }))
        .await;
    closed_session(answer, "F1 clocks in after the account was closed");
    assert_eq!(held(&desk).await, before, "nothing is written");
    db.drop_schema().await;
}

/// A day under way (`Desk::serving`: custody on, everyone clocked in, a shift serving); then the
/// accounts of F1, the monitor and finance are closed. A proposal, a clock-out and a target of
/// theirs are all answered as a closed session, and leave the branch as it was.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_command_of_a_closed_employee_is_run() {
    let db = TestDb::start().await;
    let mut desk = Desk::serving(&db).await;
    desk.outside.set_employee("F1", None);
    desk.outside.set_employee("MON1", None);
    desk.outside.set_employee("FIN1", None);

    let before = held(&desk).await;
    let proposal = desk.proposal(
        "SAFE_IN",
        json!({ "receiverId": "FIN2", "currency": "USD", "amount": 100 }),
    );
    closed_session(
        desk.act("F1", "opsPropose", proposal).await,
        "F1 proposes after the account was closed",
    );
    closed_session(
        desk.act("MON1", "opsClock", json!({ "direction": "OUT" }))
            .await,
        "MON1 clocks out after the account was closed",
    );
    closed_session(
        desk.act(
            "FIN1",
            "opsTarget",
            json!({ "amounts": { "THB": 90000, "USD": 2000 } }),
        )
        .await,
        "FIN1 sets a target after the account was closed",
    );
    assert_eq!(held(&desk).await, before, "nothing is written");
    db.drop_schema().await;
}

/// Holds before the fix and after it: the owner is never asked about — the fake says "nobody"
/// for the owner's id, and the owner's commands still run. `opsSetEntity` and `opsEnable` are the
/// only commands the owner may send; they are judged by role before account (#169, decided A), so
/// the owner is also sent a command it may not send, which must be refused for the role and not
/// as a closed session.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_is_not_asked() {
    let db = TestDb::start().await;
    let mut desk = Desk::new(&db).await;
    desk.outside.set_employee("owner-1", None);
    desk.outside.set_employee("owner", None);
    desk.ok(
        "owner",
        "opsSetEntity",
        json!({ "company": "XC", "site": "B", "confirmSeparated": true }),
    )
    .await;
    desk.ok("owner", "opsEnable", json!({})).await;
    // a command the owner may not send is refused for the role, not as a closed session
    let clock = desk
        .act("owner", "opsClock", json!({ "direction": "IN" }))
        .await;
    assert!(
        matches!(&clock, Err(AppError::Domain(_))),
        "the owner's clock-in is refused by role: {clock:?}"
    );
    db.drop_schema().await;
}
