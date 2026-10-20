//! Up-front specification tests for the queue-desk on-duty ticket (gap survey defect 1, #564
//! issuecomment-6262974911): recall, skip, start, finish and cancel need the caller's OPEN shift in
//! the branch (`noOpenShift`), as call and call next already do.
//!
//! Design: `docs/design/blueprint/queue-tickets.md` §3 `:62` ("Call next, call chosen, recall, skip,
//! start, finish, cancel | `QueueOperate` | the caller must have an OPEN shift (Q7)"), Q7 `:46`, and
//! the state row `:137` (a PAUSED counter cannot *call*; nothing else is said of pause). A counter row
//! exists exactly while the teller's shift is OPEN (§4.1, `:113`).
//!
//! Written before the implementation by the ticket author claude/dell-claude-6 (order from the xchange
//! BE controller claude/a90ab16c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;
mod queue_desk_common;

use chrono::{DateTime, Utc};
use queue_common::*;
use queue_desk_common::*;
use xc_auth::domain::access::Actor;
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::queue_ticket::QueueTicket;

async fn walk_in(db: &TestDb, key: &str, now: DateTime<Utc>) -> QueueTicket {
    db.issuer()
        .issue(&main_branch(), IssueSource::Desk, key, now)
        .await
        .unwrap_or_else(|e| panic!("issue {key}: {e:?}"))
}

async fn open_main() -> (TestDb, Queue) {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let q = queue(&db);
    (db, q)
}

fn t(user_id: &str) -> Actor {
    teller(user_id, "MAIN")
}

/// Teller A, on duty, calls w1; then A's shift closes. Answers (A, the called ticket).
async fn called_then_off_duty(db: &TestDb, q: &Queue) -> (String, QueueTicket) {
    let a = on_duty(db, q, "MAIN", "Teller A", at(2, 0)).await;
    walk_in(db, "w1", at(3, 0)).await;
    let called = q
        .desk
        .call_next(&t(&a), &main_branch(), at(3, 10))
        .await
        .expect("call while on duty");
    close_shift(db, q, "MAIN", &a, at(3, 20)).await;
    (a, called)
}

/// 1. Recall by a teller whose shift has closed: `noOpenShift`, and the ticket stays as it was.
#[tokio::test]
async fn recall_needs_an_open_shift() {
    let (db, q) = open_main().await;
    let (a, called) = called_then_off_duty(&db, &q).await;
    assert_eq!(
        refused(
            q.desk
                .recall(&t(&a), &main_branch(), &called.id, at(3, 30))
                .await
        ),
        "noOpenShift"
    );
    assert_eq!(stored(&db, &called.id).await, "CALLED W001");
    db.finish().await;
}

/// 2. Skip by a teller whose shift has closed: `noOpenShift`; the ticket is not made NO_SHOW.
#[tokio::test]
async fn skip_needs_an_open_shift() {
    let (db, q) = open_main().await;
    let (a, called) = called_then_off_duty(&db, &q).await;
    assert_eq!(
        refused(
            q.desk
                .skip(&t(&a), &main_branch(), &called.id, at(3, 30))
                .await
        ),
        "noOpenShift"
    );
    assert_eq!(stored(&db, &called.id).await, "CALLED W001");
    db.finish().await;
}

/// 3. Start by a teller whose shift has closed: `noOpenShift`; the ticket is not SERVING.
#[tokio::test]
async fn start_needs_an_open_shift() {
    let (db, q) = open_main().await;
    let (a, called) = called_then_off_duty(&db, &q).await;
    assert_eq!(
        refused(
            q.desk
                .start(&t(&a), &main_branch(), &called.id, at(3, 30))
                .await
        ),
        "noOpenShift"
    );
    assert_eq!(stored(&db, &called.id).await, "CALLED W001");
    db.finish().await;
}

/// 4. Finish by a teller whose shift closed while serving: `noOpenShift`; the ticket stays SERVING.
#[tokio::test]
async fn finish_needs_an_open_shift() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    walk_in(&db, "w1", at(3, 0)).await;
    let called = q
        .desk
        .call_next(&t(&a), &main_branch(), at(3, 10))
        .await
        .expect("call");
    q.desk
        .start(&t(&a), &main_branch(), &called.id, at(3, 12))
        .await
        .expect("start");
    close_shift(&db, &q, "MAIN", &a, at(3, 20)).await;
    assert_eq!(
        refused(
            q.desk
                .finish(&t(&a), &main_branch(), &called.id, at(3, 30))
                .await
        ),
        "noOpenShift"
    );
    assert_eq!(stored(&db, &called.id).await, "SERVING W001");
    db.finish().await;
}

/// 5. Cancel of a WAITING ticket by a teller who was never on duty: `noOpenShift`.
#[tokio::test]
async fn cancel_needs_an_open_shift() {
    let (db, q) = open_main().await;
    let off = db.add_user("Teller Off").await;
    let w1 = walk_in(&db, "w1", at(3, 0)).await;
    assert_eq!(
        refused(
            q.desk
                .cancel(&t(&off), &main_branch(), &w1.id, "left", at(3, 30))
                .await
        ),
        "noOpenShift"
    );
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");
    db.finish().await;
}

/// 6. A BRANCH_MANAGER acting on another teller's ticket also needs an OPEN shift (§3: "the caller").
#[tokio::test]
async fn a_branch_manager_without_an_open_shift_cannot_recall() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    walk_in(&db, "w1", at(3, 0)).await;
    let called = q
        .desk
        .call_next(&t(&a), &main_branch(), at(3, 10))
        .await
        .expect("call");
    let boss = db.add_user("Manager Off").await;
    assert_eq!(
        refused(
            q.desk
                .recall(
                    &manager(&boss, "MAIN"),
                    &main_branch(),
                    &called.id,
                    at(3, 30)
                )
                .await
        ),
        "noOpenShift"
    );
    db.finish().await;
}

/// 7. Kept: a PAUSED counter is still on duty. Pause blocks calling only (`:137`), so a paused
/// teller still recalls, starts and finishes their own ticket.
#[tokio::test]
async fn a_paused_counter_may_still_recall_start_and_finish() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    walk_in(&db, "w1", at(3, 0)).await;
    let called = q
        .desk
        .call_next(&t(&a), &main_branch(), at(3, 10))
        .await
        .expect("call");
    q.counters
        .pause(&t(&a), &main_branch(), at(3, 11))
        .await
        .expect("pause");
    q.desk
        .recall(&t(&a), &main_branch(), &called.id, at(3, 12))
        .await
        .expect("recall while paused");
    q.desk
        .start(&t(&a), &main_branch(), &called.id, at(3, 13))
        .await
        .expect("start while paused");
    q.desk
        .finish(&t(&a), &main_branch(), &called.id, at(3, 14))
        .await
        .expect("finish while paused");
    assert_eq!(stored(&db, &called.id).await, "COMPLETED W001");
    db.finish().await;
}
