//! XC-WALKIN-QUEUE WQ-1 (`docs/design/walkin-queue.md` W3, W4, W6–W8, §4, §8 row 1): `WalkInDesk`
//! against a real PostgreSQL. A desk sale's customer enters the queue like a booking: one ticket,
//! SERVING for the teller at once, without moving anyone else's place; the sale's completion
//! finishes it.
//!
//! Written before the implementation by the sub-controller (claude/e2926a49). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Signatures this test assumes (the skeleton on `feat/walkin-queue`):
//! - `WalkInDesk::new(TicketIssuer, QueueDesk, Arc<dyn QueueRepository>, StatusService)`;
//! - `WalkInDesk::serve_in(tx, &Actor, &BranchId, customer_id, sale_key, may_issue, now)
//!   -> AppResult<QueueTicket>`, storing a new ticket under `desk-sale:<sale_key>`;
//! - `WalkInDesk::finish_for_sale_in(tx, &BranchId, ticket_id, now) -> AppResult<bool>`;
//! - the refusals answer `QueueError`'s mapping: `Validation(code)`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;
mod queue_desk_common;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use queue_common::*;
use queue_desk_common::*;
use xc_auth::domain::access::Actor;
use xc_queue::application::issue::walk_in_desk::WalkInDesk;
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::queue_ticket::{QueueTicket, TicketStatus};

// ------------------------------------------------------------ fixtures

fn walk_in_desk(db: &TestDb, q: &Queue) -> WalkInDesk {
    WalkInDesk::new(
        db.issuer(),
        q.desk.clone(),
        q.repo.clone(),
        q.status.clone(),
    )
}

/// An open MAIN queue and its services.
async fn open_main() -> (TestDb, Queue) {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let q = queue(&db);
    (db, q)
}

fn t(user_id: &str) -> Actor {
    teller(user_id, "MAIN")
}

/// `serve_in` in a branch transaction of its own.
async fn serve(
    db: &TestDb,
    wd: &WalkInDesk,
    actor: &Actor,
    customer: Option<&str>,
    sale_key: &str,
    may_issue: bool,
    now: DateTime<Utc>,
) -> AppResult<QueueTicket> {
    let wd = wd.clone();
    let actor = actor.clone();
    let customer = customer.map(str::to_owned);
    let key = sale_key.to_owned();
    db.branch_tx()
        .run(&main_branch(), move |tx| {
            Box::pin(async move {
                wd.serve_in(
                    tx,
                    &actor,
                    &main_branch(),
                    customer.as_deref(),
                    &key,
                    may_issue,
                    now,
                )
                .await
            })
        })
        .await
}

/// `finish_for_sale_in` in a branch transaction of its own.
async fn finish_for_sale(
    db: &TestDb,
    wd: &WalkInDesk,
    ticket_id: &str,
    now: DateTime<Utc>,
) -> AppResult<bool> {
    let wd = wd.clone();
    let id = ticket_id.to_owned();
    db.branch_tx()
        .run(&main_branch(), move |tx| {
            Box::pin(async move { wd.finish_for_sale_in(tx, &main_branch(), &id, now).await })
        })
        .await
}

async fn issued(db: &TestDb, source: IssueSource, key: &str, now: DateTime<Utc>) -> QueueTicket {
    db.issuer()
        .issue(&main_branch(), source, key, now)
        .await
        .unwrap_or_else(|e| panic!("issue {key}: {e:?}"))
}

async fn ticket_count(db: &TestDb) -> i64 {
    db.count("SELECT count(*) FROM queue_tickets WHERE branch_id = 'MAIN'")
        .await
}

async fn key_of(db: &TestDb, ticket_id: &str) -> String {
    db.text(&format!(
        "SELECT idempotency_key FROM queue_tickets WHERE id = '{ticket_id}'::uuid"
    ))
    .await
    .expect("the ticket's key")
}

// ------------------------------------------------------------ W3, W4 (4): a new Desk ticket

#[tokio::test]
async fn a_new_desk_ticket_is_serving_for_the_caller_with_issue_call_and_start_events() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let wd = walk_in_desk(&db, &q);

    let ticket = serve(&db, &wd, &t(&a), None, "sale-1", true, at(3, 0))
        .await
        .expect("served");

    assert_eq!(ticket.status, TicketStatus::Serving);
    assert_eq!(ticket.counter_teller_id.as_deref(), Some(a.as_str()));
    assert!(ticket.customer_id.is_none() && ticket.booking_id.is_none());
    assert!(ticket.kiosk_request_id.is_none());
    assert!(
        ticket.display_number.starts_with('W'),
        "{}",
        ticket.display_number
    );
    assert_eq!(
        stored(&db, &ticket.id).await,
        format!("SERVING {}", ticket.display_number)
    );
    assert_eq!(key_of(&db, &ticket.id).await, "desk-sale:sale-1");
    assert_eq!(
        events(&db, &ticket.id).await,
        vec![
            "ISSUE WAITING -".to_owned(),
            format!("CALL CALLED {a}"),
            format!("START SERVING {a}"),
        ]
    );
    db.finish().await;
}

#[tokio::test]
async fn the_waiting_tickets_call_order_does_not_change() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let b = on_duty(&db, &q, "MAIN", "Teller B", at(2, 0)).await;
    let c = on_duty(&db, &q, "MAIN", "Teller C", at(2, 0)).await;
    let w1 = issued(&db, IssueSource::Desk, "w1", at(3, 0)).await;
    db.add_booking("MAIN", "BK-1").await;
    let b1 = issued(
        &db,
        IssueSource::Booking {
            booking_id: "BK-1".to_owned(),
        },
        "booking:BK-1",
        at(3, 1),
    )
    .await;
    let wd = walk_in_desk(&db, &q);

    let sale = serve(&db, &wd, &t(&a), None, "sale-1", true, at(3, 2))
        .await
        .expect("served");
    assert_ne!(sale.id, w1.id, "the waiting walk-in keeps its place");

    let first = q
        .desk
        .call_next(&t(&b), &main_branch(), at(3, 10))
        .await
        .expect("B calls");
    let second = q
        .desk
        .call_next(&t(&c), &main_branch(), at(3, 11))
        .await
        .expect("C calls");
    assert_eq!(first.id, b1.id, "the booking line is still called first");
    assert_eq!(second.id, w1.id, "then the earlier walk-in");
    db.finish().await;
}

// ------------------------------------------------------------ W5, W4 (1): one entry per sale

#[tokio::test]
async fn the_same_sale_key_twice_answers_one_ticket() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let wd = walk_in_desk(&db, &q);

    let first = serve(&db, &wd, &t(&a), None, "sale-1", true, at(3, 0))
        .await
        .expect("served");
    let again = serve(&db, &wd, &t(&a), None, "sale-1", true, at(3, 1))
        .await
        .expect("replayed");

    assert_eq!(again.id, first.id);
    assert_eq!(again.status, TicketStatus::Serving);
    assert_eq!(ticket_count(&db).await, 1);
    db.finish().await;
}

// ------------------------------------------------------------ W4 (2): the caller's held walk-in

#[tokio::test]
async fn the_callers_called_walk_in_from_the_board_is_started_and_returned_with_no_new_number() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let board = issued(&db, IssueSource::Desk, "board-1", at(3, 0)).await;
    q.desk
        .call_next(&t(&a), &main_branch(), at(3, 1))
        .await
        .expect("A calls from the board");
    let wd = walk_in_desk(&db, &q);

    let ticket = serve(&db, &wd, &t(&a), None, "sale-1", true, at(3, 2))
        .await
        .expect("served");

    assert_eq!(ticket.id, board.id);
    assert_eq!(ticket.status, TicketStatus::Serving);
    assert_eq!(ticket_count(&db).await, 1, "no new number");
    assert_eq!(
        stored(&db, &board.id).await,
        format!("SERVING {}", board.display_number)
    );
    db.finish().await;
}

// ------------------------------------------------------------ W4 (3): the customer's App ticket

#[tokio::test]
async fn the_customers_waiting_app_ticket_is_served_instead_of_a_new_one() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let app = issued(
        &db,
        IssueSource::App {
            customer_id: "CUST-9".to_owned(),
        },
        "app-1",
        at(3, 0),
    )
    .await;
    let wd = walk_in_desk(&db, &q);

    let ticket = serve(&db, &wd, &t(&a), Some("CUST-9"), "sale-1", true, at(3, 1))
        .await
        .expect("served");

    assert_eq!(ticket.id, app.id);
    assert_eq!(ticket.status, TicketStatus::Serving);
    assert_eq!(ticket.counter_teller_id.as_deref(), Some(a.as_str()));
    assert_eq!(ticket_count(&db).await, 1);
    db.finish().await;
}

// ------------------------------------------------------------ W4: what the caller already holds

#[tokio::test]
async fn a_held_booking_ticket_refuses_already_serving() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    db.add_booking("MAIN", "BK-1").await;
    issued(
        &db,
        IssueSource::Booking {
            booking_id: "BK-1".to_owned(),
        },
        "booking:BK-1",
        at(3, 0),
    )
    .await;
    q.desk
        .call_next(&t(&a), &main_branch(), at(3, 1))
        .await
        .expect("A calls the booking");
    let wd = walk_in_desk(&db, &q);

    let got = serve(&db, &wd, &t(&a), None, "sale-1", true, at(3, 2)).await;

    assert_eq!(refused(got), "alreadyServing");
    assert_eq!(ticket_count(&db).await, 1);
    db.finish().await;
}

#[tokio::test]
async fn a_held_kiosk_ticket_refuses_already_serving() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    db.add_kiosk_request("MAIN", "KR-1").await;
    issued(
        &db,
        IssueSource::Kiosk {
            request_id: "KR-1".to_owned(),
        },
        "kiosk:KR-1",
        at(3, 0),
    )
    .await;
    q.desk
        .call_next(&t(&a), &main_branch(), at(3, 1))
        .await
        .expect("A calls the kiosk ticket");
    let wd = walk_in_desk(&db, &q);

    let got = serve(&db, &wd, &t(&a), None, "sale-1", true, at(3, 2)).await;

    assert_eq!(refused(got), "alreadyServing");
    assert_eq!(ticket_count(&db).await, 1);
    db.finish().await;
}

#[tokio::test]
async fn a_held_ticket_of_another_desk_sale_refuses_already_serving() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let wd = walk_in_desk(&db, &q);
    serve(&db, &wd, &t(&a), None, "sale-1", true, at(3, 0))
        .await
        .expect("the first sale");

    let got = serve(&db, &wd, &t(&a), None, "sale-2", true, at(3, 1)).await;

    assert_eq!(refused(got), "alreadyServing");
    assert_eq!(ticket_count(&db).await, 1);
    db.finish().await;
}

// ------------------------------------------------------------ W6: the 10-minute cutoff

#[tokio::test]
async fn may_issue_false_refuses_a_new_ticket_but_answers_a_replay() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let wd = walk_in_desk(&db, &q);
    let first = serve(&db, &wd, &t(&a), None, "sale-1", true, at(3, 0))
        .await
        .expect("served inside the hours");

    let replay = serve(&db, &wd, &t(&a), None, "sale-1", false, at(3, 55))
        .await
        .expect("a replay answers at any time");
    assert_eq!(replay.id, first.id);

    let b = on_duty(&db, &q, "MAIN", "Teller B", at(2, 0)).await;
    let got = serve(&db, &wd, &t(&b), None, "sale-2", false, at(3, 55)).await;
    assert_eq!(refused(got), "outsideServiceHours");
    assert_eq!(ticket_count(&db).await, 1, "no new ticket");
    db.finish().await;
}

// ------------------------------------------------------------ W7: a closed queue

#[tokio::test]
async fn a_closed_queue_refuses_queue_closed_and_uses_no_number() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let wd = walk_in_desk(&db, &q);
    db.close_queue("MAIN").await;

    let got = serve(&db, &wd, &t(&a), None, "sale-1", true, at(3, 0)).await;
    assert_eq!(refused(got), "queueClosed");
    assert_eq!(ticket_count(&db).await, 0);

    db.open_queue("MAIN").await;
    let ticket = serve(&db, &wd, &t(&a), None, "sale-2", true, at(3, 1))
        .await
        .expect("served once open");
    assert_eq!(ticket.sequence, 1, "the refusal used no number");
    db.finish().await;
}

// ------------------------------------------------------------ W8: finishing with the sale

#[tokio::test]
async fn finish_for_sale_moves_serving_to_completed_with_a_finish_event() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let wd = walk_in_desk(&db, &q);
    let ticket = serve(&db, &wd, &t(&a), None, "sale-1", true, at(3, 0))
        .await
        .expect("served");

    let done = finish_for_sale(&db, &wd, &ticket.id, at(3, 20))
        .await
        .expect("finished");

    assert!(done);
    assert_eq!(
        stored(&db, &ticket.id).await,
        format!("COMPLETED {}", ticket.display_number)
    );
    let kinds = events(&db, &ticket.id).await;
    assert_eq!(kinds.len(), 4, "{kinds:?}");
    assert!(
        kinds[3].starts_with("FINISH COMPLETED "),
        "the last event is the finish: {kinds:?}"
    );
    db.finish().await;
}

#[tokio::test]
async fn finish_for_sale_answers_false_and_changes_nothing_on_a_completed_ticket() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let wd = walk_in_desk(&db, &q);
    let ticket = serve(&db, &wd, &t(&a), None, "sale-1", true, at(3, 0))
        .await
        .expect("served");
    q.desk
        .finish(&t(&a), &main_branch(), &ticket.id, at(3, 10))
        .await
        .expect("finished from the board");
    let before = events(&db, &ticket.id).await;

    let done = finish_for_sale(&db, &wd, &ticket.id, at(3, 20))
        .await
        .expect("answers");

    assert!(!done);
    assert_eq!(
        stored(&db, &ticket.id).await,
        format!("COMPLETED {}", ticket.display_number)
    );
    assert_eq!(events(&db, &ticket.id).await, before, "no new event");
    db.finish().await;
}

#[tokio::test]
async fn finish_for_sale_answers_false_and_changes_nothing_on_a_cancelled_ticket() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let waiting = issued(&db, IssueSource::Desk, "w1", at(3, 0)).await;
    q.desk
        .cancel(&t(&a), &main_branch(), &waiting.id, "left", at(3, 5))
        .await
        .expect("cancelled");
    let wd = walk_in_desk(&db, &q);
    let before = events(&db, &waiting.id).await;

    let done = finish_for_sale(&db, &wd, &waiting.id, at(3, 20))
        .await
        .expect("answers");

    assert!(!done);
    assert_eq!(
        stored(&db, &waiting.id).await,
        format!("CANCELLED {}", waiting.display_number)
    );
    assert_eq!(events(&db, &waiting.id).await, before, "no new event");
    db.finish().await;
}
