//! Ticket #68 — a long note leaves a custody transfer stuck in transit for ever. Written before
//! the fix. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Committing a `SAFE_OUT` or an `EXTERNAL_IN` writes a stock movement whose reason is
//! `"<KIND> · " + note` (`custody_machine.rs`, the commit arm), and a movement's reason may hold at
//! most 200 UTF-16 units (`shift/commands/payload.rs` `read_text`, `move_stock.rs`). A longer one
//! is refused at commit, when the money is already in transit — and a request in transit cannot be
//! rejected either (`MoneyInTransit`). Owner, 2026-10-06 ("do as proposed"): refuse such a note at
//! **propose**, with the shift's own sentence; nothing is stored. A deliberate difference from v2
//! (v2 accepts it and then cannot finish it: `v2_custody_service_cases.json` story 0, steps 78–82).
//!
//! The limit is the movement's: 200 − the length of `"<KIND> · "` — 189 for `SAFE_OUT`, 186 for
//! `EXTERNAL_IN`. Other kinds keep the propose limit of 500 (their notes never enter a reason).
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

mod custody_common;
mod custody_service_common;
mod shift_service_common;

use custody_service_common::Desk;
use pos_error::AppError;
use serde_json::{Value, json};
use shift_service_common::TestDb;
use xc_shift::domain::custody::entities::stored_event::StoredEvent;
use xc_shift::domain::custody::state::CustodyState;

const TEXT_INVALID: &str = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";

fn note(units: usize) -> String {
    "n".repeat(units)
}

/// What the branch holds in custody, compared by value (`CustodyState` and `StoredEvent` are
/// `PartialEq`); never by `Debug` text, whose map order is not fixed.
async fn held(desk: &Desk) -> (CustodyState, Vec<StoredEvent>) {
    let state = desk.custody.state(&desk.branch).await.expect("state");
    let events = desk.custody.events(&desk.branch).await.expect("events");
    (state, events)
}

fn refused_with_the_shift_sentence(answer: Result<Value, AppError>, what: &str) {
    match answer {
        Err(AppError::Domain(sentence)) => assert_eq!(sentence, TEXT_INVALID, "{what}"),
        other => panic!("{what}: must be refused at propose with «{TEXT_INVALID}», got {other:?}"),
    }
}

fn safe_out(note: &str) -> Value {
    json!({ "receiverId": "FIN1", "currency": "USD", "amount": 50, "note": note })
}

fn external_in(note: &str) -> Value {
    json!({ "receiverId": "MGR1", "currency": "EUR", "amount": 1000, "note": note })
}

/// Holds before the fix and after it: a `SAFE_OUT` whose note is exactly at the limit (189) is
/// proposed, dispatched, signed and committed, and the drawer's movement carries the whole note.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_safe_out_with_a_note_at_the_limit_goes_all_the_way() {
    let db = TestDb::start().await;
    let mut d = Desk::serving(&db).await;
    let text = note(189);
    let id = d.propose("F1", "SAFE_OUT", safe_out(&text)).await;
    d.ok("BE1", "opsDispatch", json!({ "id": id })).await;
    d.ok("FIN1", "opsSign", json!({ "id": id })).await;
    d.ok("BE2", "opsCommit", json!({ "id": id })).await;
    let movements = db.service().movements(&d.branch).await.unwrap();
    assert_eq!(
        movements.last().unwrap().reason,
        format!("SAFE_OUT · {text}")
    );
    db.drop_schema().await;
}

/// One unit more is refused at propose, and nothing is stored.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_safe_out_with_a_note_one_over_the_limit_is_refused_at_propose() {
    let db = TestDb::start().await;
    let mut d = Desk::serving(&db).await;
    let before = held(&d).await;
    let payload = d.proposal("SAFE_OUT", safe_out(&note(190)));
    refused_with_the_shift_sentence(
        d.act("F1", "opsPropose", payload).await,
        "SAFE_OUT with a 190-unit note",
    );
    assert_eq!(held(&d).await, before, "nothing is stored");
    db.drop_schema().await;
}

/// Holds before the fix and after it: an `EXTERNAL_IN` whose note is exactly at the limit (186)
/// goes all the way, with the recorded people of v2's day (story 0, steps 78–82).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_external_in_with_a_note_at_the_limit_goes_all_the_way() {
    let db = TestDb::start().await;
    let mut d = Desk::serving(&db).await;
    let text = note(186);
    let id = d.propose("F1", "EXTERNAL_IN", external_in(&text)).await;
    d.ok("BE1", "opsDispatch", json!({ "id": id })).await;
    d.ok("MGR1", "opsSign", json!({ "id": id })).await;
    d.ok("FIN2", "opsSign", json!({ "id": id })).await;
    d.ok("BE1", "opsCommit", json!({ "id": id })).await;
    let movements = db.service().movements(&d.branch).await.unwrap();
    assert_eq!(
        movements.last().unwrap().reason,
        format!("EXTERNAL_IN · {text}")
    );
    db.drop_schema().await;
}

/// One unit more (187), and v2's own 195, are refused at propose; nothing is stored.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_external_in_with_a_note_over_the_limit_is_refused_at_propose() {
    let db = TestDb::start().await;
    let mut d = Desk::serving(&db).await;
    for units in [187, 195] {
        let before = held(&d).await;
        let payload = d.proposal("EXTERNAL_IN", external_in(&note(units)));
        refused_with_the_shift_sentence(
            d.act("F1", "opsPropose", payload).await,
            &format!("EXTERNAL_IN with a {units}-unit note"),
        );
        assert_eq!(held(&d).await, before, "nothing is stored ({units})");
    }
    db.drop_schema().await;
}

/// The limit counts UTF-16 units, as the movement's rule does: 186 Thai characters (one unit
/// each) are accepted, and a note of 94 characters outside the basic plane (two units each, 188
/// in all) is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_limit_counts_utf16_units_like_the_movement_does() {
    let db = TestDb::start().await;
    let mut d = Desk::serving(&db).await;
    let thai = "ก".repeat(186);
    d.propose("F1", "EXTERNAL_IN", external_in(&thai)).await;
    let mut d2 = Desk::serving(&db).await;
    let wide = "😀".repeat(94);
    let payload = d2.proposal("EXTERNAL_IN", external_in(&wide));
    refused_with_the_shift_sentence(
        d2.act("F1", "opsPropose", payload).await,
        "94 characters of two units each",
    );
    db.drop_schema().await;
}

/// Holds before the fix and after it: a kind whose note never enters a movement keeps the
/// propose limit of 500.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_safe_in_keeps_the_limit_of_500() {
    let db = TestDb::start().await;
    let mut d = Desk::serving(&db).await;
    d.propose(
        "F1",
        "SAFE_IN",
        json!({ "receiverId": "FIN1", "currency": "USD", "amount": 50, "note": note(500) }),
    )
    .await;
    db.drop_schema().await;
}
