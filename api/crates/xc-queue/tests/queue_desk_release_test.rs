//! #5541 (design `docs/design/blueprint/queue-tickets.md` §4.6, owner 2026-10-29 "ยกเลิกได้ ระบุว่าไม่มีการขาย"):
//! `QueueDesk::release` ends a SERVING ticket whose sale never happened as CANCELLED with reason `noSale` and a
//! CANCEL event, so the teller's counter is free for the next customer. Who may send it is `finish`'s rule (QU-A2):
//! the serving teller, or a BRANCH_MANAGER of the branch, each on an OPEN shift. A ticket a saved sale names is
//! pinned through the route (`api/tests/queue_release_no_sale_route_test.rs`), where transactions are made.
//!
//! Written before the implementation by the ticket author (claude/2ca5874b). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! Signature this test assumes (skeleton on `test/5541`, [required]):
//! `QueueDesk::release(&self, &Actor, &BranchId, ticket_id: &str, now) -> AppResult<QueueTicket>`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;
mod queue_desk_common;

use chrono::{DateTime, Utc};
use queue_common::*;
use queue_desk_common::*;
use xc_auth::domain::access::Actor;
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::queue_ticket::{QueueTicket, TicketStatus};

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

/// A walk-in that `teller_id` called and started: SERVING, held by them.
async fn serving(db: &TestDb, q: &Queue, teller_id: &str, key: &str) -> QueueTicket {
    walk_in(db, key, at(3, 0)).await;
    let called = q
        .desk
        .call_next(&t(teller_id), &main_branch(), at(3, 10))
        .await
        .expect("call");
    q.desk
        .start(&t(teller_id), &main_branch(), &called.id, at(3, 11))
        .await
        .expect("start")
}

async fn ended_reason(db: &TestDb, id: &str) -> Option<String> {
    db.text(&format!(
        "SELECT ended_reason FROM queue_tickets WHERE id = '{id}'::uuid"
    ))
    .await
}

async fn last_event_reason(db: &TestDb, id: &str) -> Option<String> {
    db.text(&format!(
        "SELECT reason FROM queue_events WHERE ticket_id = '{id}'::uuid ORDER BY at DESC, created_at DESC LIMIT 1"
    ))
    .await
}

#[tokio::test]
async fn the_serving_teller_releases_a_ticket_with_no_sale_and_can_take_the_next_customer() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let b = main_branch();
    let ticket = serving(&db, &q, &a, "w1").await;
    let version = queue_version(&db, "MAIN").await;

    let released = q
        .desk
        .release(&t(&a), &b, &ticket.id, at(3, 20))
        .await
        .expect("the serving teller releases it");

    assert_eq!(released.status, TicketStatus::Cancelled);
    assert_eq!(released.ended_reason.as_deref(), Some("noSale"));
    assert_eq!(stored(&db, &ticket.id).await, "CANCELLED W001");
    assert_eq!(
        ended_reason(&db, &ticket.id).await.as_deref(),
        Some("noSale")
    );
    assert_eq!(
        events(&db, &ticket.id).await.last().cloned(),
        Some(format!("CANCEL CANCELLED {a}")),
        "one CANCEL event by the teller"
    );
    assert_eq!(
        last_event_reason(&db, &ticket.id).await.as_deref(),
        Some("noSale")
    );
    assert!(
        queue_version(&db, "MAIN").await > version,
        "the branch's queue version moves"
    );

    walk_in(&db, "w2", at(3, 21)).await;
    let next = q
        .desk
        .call_next(&t(&a), &b, at(3, 22))
        .await
        .expect("the counter is free again: no alreadyServing");
    assert_eq!(next.display_number, "W002");
    db.finish().await;
}

#[tokio::test]
async fn another_teller_is_refused_and_a_branch_manager_on_duty_releases() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let other = on_duty(&db, &q, "MAIN", "Teller B", at(2, 0)).await;
    let boss = on_duty(&db, &q, "MAIN", "Manager", at(2, 0)).await;
    let b = main_branch();
    let ticket = serving(&db, &q, &a, "w1").await;

    assert_eq!(
        refused(q.desk.release(&t(&other), &b, &ticket.id, at(3, 20)).await),
        "notYourTicket"
    );
    assert_eq!(stored(&db, &ticket.id).await, "SERVING W001");

    q.desk
        .release(&manager(&boss, "MAIN"), &b, &ticket.id, at(3, 21))
        .await
        .expect("BRANCH_MANAGER releases another's ticket");
    assert_eq!(stored(&db, &ticket.id).await, "CANCELLED W001");
    assert_eq!(
        events(&db, &ticket.id).await.last().cloned(),
        Some(format!("CANCEL CANCELLED {boss}"))
    );
    db.finish().await;
}

#[tokio::test]
async fn only_a_serving_ticket_is_released_any_other_state_is_stale_and_unchanged() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let b = main_branch();

    let waiting = walk_in(&db, "w1", at(3, 0)).await;
    assert_eq!(
        refused(q.desk.release(&t(&a), &b, &waiting.id, at(3, 1)).await),
        "staleVersion",
        "WAITING"
    );
    assert_eq!(stored(&db, &waiting.id).await, "WAITING W001");

    let called = q.desk.call_next(&t(&a), &b, at(3, 2)).await.expect("call");
    assert_eq!(
        refused(q.desk.release(&t(&a), &b, &called.id, at(3, 3)).await),
        "staleVersion",
        "CALLED: the teller cancels or skips it instead"
    );
    assert_eq!(stored(&db, &called.id).await, "CALLED W001");

    q.desk
        .start(&t(&a), &b, &called.id, at(3, 4))
        .await
        .expect("start");
    q.desk
        .finish(&t(&a), &b, &called.id, at(3, 5))
        .await
        .expect("finish");
    assert_eq!(
        refused(q.desk.release(&t(&a), &b, &called.id, at(3, 6)).await),
        "staleVersion",
        "COMPLETED is final"
    );
    assert_eq!(stored(&db, &called.id).await, "COMPLETED W001");
    db.finish().await;
}

#[tokio::test]
async fn a_teller_without_an_open_shift_is_refused() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let ticket = serving(&db, &q, &a, "w1").await;
    let off = db.add_user("Off duty").await;

    assert_eq!(
        refused(
            q.desk
                .release(
                    &manager(&off, "MAIN"),
                    &main_branch(),
                    &ticket.id,
                    at(3, 20)
                )
                .await
        ),
        "noOpenShift"
    );
    assert_eq!(stored(&db, &ticket.id).await, "SERVING W001");
    db.finish().await;
}
