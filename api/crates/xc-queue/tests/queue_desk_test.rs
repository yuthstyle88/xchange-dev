//! QU-A2 (#556 design row QU-3) — `QueueDesk` against a real PostgreSQL: call next in
//! `CallOrder`, call a chosen ticket, recall, skip, start, finish and cancel; who may send each;
//! one ticket per teller at a time; the lost race; one event and one branch version per
//! transition. Written before the implementation by the ticket author (claude/b060e9). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/queue-tickets.md` §3, §4.2, §4.3, Q6, Q7, Q10, Q11, §9.1 row
//! QU-3. Owner Q-A2 (the booking line is called first) and Q-A3 (skip is NO_SHOW, final).
//!
//! Signatures this test assumes (QU-A2 scaffolding, `[required]` once pushed):
//! - `QueueDesk::{call_next(&Actor, &BranchId, now), call/recall/skip/start/finish(&Actor,
//!   &BranchId, ticket_id, now), cancel(&Actor, &BranchId, ticket_id, reason, now)}`, each
//!   `-> AppResult<QueueTicket>`;
//! - `QueueRepository::lock_next_waiting_in(tx, &BranchId) -> AppResult<Option<QueueTicket>>`;
//! - a permission refusal answers `AppError::Forbidden` [proposed]; the queue's refusals answer
//!   `QueueError`'s mapping (QU-A1): `Validation(code)`, and `Conflict("staleVersion")`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;
mod queue_desk_common;

use chrono::{DateTime, Utc};
use queue_common::*;
use queue_desk_common::*;
use xc_auth::domain::access::Actor;
use xc_kernel::ids::BranchId;
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::queue_ticket::{QueueTicket, TicketStatus};
use xc_queue::domain::ticket::service_type::ServiceType;

async fn walk_in(db: &TestDb, key: &str, now: DateTime<Utc>) -> QueueTicket {
    db.issuer()
        .issue(&main_branch(), IssueSource::Desk, key, now)
        .await
        .unwrap_or_else(|e| panic!("issue {key}: {e:?}"))
}

async fn booked(db: &TestDb, booking_id: &str, now: DateTime<Utc>) -> QueueTicket {
    db.add_booking("MAIN", booking_id).await;
    db.issuer()
        .issue(
            &main_branch(),
            IssueSource::Booking {
                booking_id: booking_id.to_owned(),
            },
            &format!("booking:{booking_id}"),
            now,
        )
        .await
        .unwrap_or_else(|e| panic!("issue for {booking_id}: {e:?}"))
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

#[tokio::test]
async fn call_next_takes_the_booking_line_first_then_the_lowest_number() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let b = on_duty(&db, &q, "MAIN", "Teller B", at(2, 0)).await;
    let c = on_duty(&db, &q, "MAIN", "Teller C", at(2, 0)).await;
    walk_in(&db, "w1", at(3, 0)).await;
    walk_in(&db, "w2", at(3, 1)).await;
    booked(&db, "BK-1", at(3, 2)).await;

    let first = q
        .desk
        .call_next(&t(&a), &main_branch(), at(3, 10))
        .await
        .expect("A calls");
    let second = q
        .desk
        .call_next(&t(&b), &main_branch(), at(3, 11))
        .await
        .expect("B calls");
    let third = q
        .desk
        .call_next(&t(&c), &main_branch(), at(3, 12))
        .await
        .expect("C calls");

    assert_eq!(
        (first.display_number.as_str(), first.service_type),
        ("B001", ServiceType::Booking),
        "the booking line is called before walk-ins that waited longer (owner Q-A2)"
    );
    assert_eq!(
        second.display_number, "W001",
        "then the lowest walk-in number"
    );
    assert_eq!(third.display_number, "W002");
    for (ticket, who, m) in [(&first, &a, 10), (&second, &b, 11), (&third, &c, 12)] {
        assert_eq!(ticket.status, TicketStatus::Called);
        assert_eq!(
            ticket.counter_teller_id.as_deref(),
            Some(who.as_str()),
            "the caller's ticket"
        );
        assert_eq!(ticket.called_at, Some(at(3, m)));
        assert_eq!(
            stored(&db, &ticket.id).await,
            format!("CALLED {}", ticket.display_number)
        );
        assert_eq!(
            events(&db, &ticket.id).await,
            vec!["ISSUE WAITING -".to_owned(), format!("CALL CALLED {who}")],
            "one CALL event with the caller as actor"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn two_tellers_calling_next_at_once_get_two_different_tickets() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let b = on_duty(&db, &q, "MAIN", "Teller B", at(2, 0)).await;
    walk_in(&db, "w1", at(3, 0)).await;
    walk_in(&db, "w2", at(3, 1)).await;

    let (ta, tb, br) = (t(&a), t(&b), main_branch());
    let (ra, rb) = tokio::join!(
        q.desk.call_next(&ta, &br, at(3, 10)),
        q.desk.call_next(&tb, &br, at(3, 10)),
    );
    let (ra, rb) = (ra.expect("A calls"), rb.expect("B calls"));
    assert_ne!(ra.id, rb.id, "two tellers never call the same ticket (Q11)");
    let mut numbers = vec![ra.display_number.clone(), rb.display_number.clone()];
    numbers.sort();
    assert_eq!(numbers, vec!["W001", "W002"]);
    assert_eq!(
        db.count("SELECT count(*) FROM queue_tickets WHERE status = 'CALLED'")
            .await,
        2
    );
    db.finish().await;
}

#[tokio::test]
async fn the_next_waiting_ticket_is_locked_and_a_locked_one_is_passed_over() {
    let (db, q) = open_main().await;
    let w1 = walk_in(&db, "w1", at(3, 0)).await;
    let w2 = walk_in(&db, "w2", at(3, 1)).await;
    let b1 = booked(&db, "BK-1", at(3, 2)).await;

    let mut first_tx = db.pool.begin().await.expect("begin");
    let first = q
        .repo
        .lock_next_waiting_in(&mut first_tx, &main_branch())
        .await
        .expect("lock the next")
        .expect("something waits");
    assert_eq!(first.id, b1.id, "BOOKING first (owner Q-A2)");

    let mut second_tx = db.pool.begin().await.expect("begin");
    let second = q
        .repo
        .lock_next_waiting_in(&mut second_tx, &main_branch())
        .await
        .expect("lock the next")
        .expect("something waits");
    assert_eq!(
        second.id, w1.id,
        "a ticket another transaction holds is skipped, not waited for (FOR UPDATE SKIP LOCKED)"
    );

    let mut third_tx = db.pool.begin().await.expect("begin");
    let third = q
        .repo
        .lock_next_waiting_in(&mut third_tx, &main_branch())
        .await
        .expect("lock the next")
        .expect("something waits");
    assert_eq!(third.id, w2.id);

    let mut fourth_tx = db.pool.begin().await.expect("begin");
    let none = q
        .repo
        .lock_next_waiting_in(&mut fourth_tx, &main_branch())
        .await
        .expect("lock the next");
    assert_eq!(none, None, "every waiting ticket is held");
    for tx in [first_tx, second_tx, third_tx, fourth_tx] {
        tx.rollback().await.expect("rollback");
    }
    db.finish().await;
}

#[tokio::test]
async fn a_teller_may_call_a_chosen_waiting_ticket() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let w1 = walk_in(&db, "w1", at(3, 0)).await;
    let w2 = walk_in(&db, "w2", at(3, 1)).await;

    let called = q
        .desk
        .call(&t(&a), &main_branch(), &w2.id, at(3, 10))
        .await
        .expect("call W002");
    assert_eq!(
        (
            called.id.as_str(),
            called.status,
            called.counter_teller_id.as_deref()
        ),
        (w2.id.as_str(), TicketStatus::Called, Some(a.as_str()))
    );
    assert_eq!(
        stored(&db, &w1.id).await,
        "WAITING W001",
        "the one before it still waits"
    );
    db.finish().await;
}

#[tokio::test]
async fn two_tellers_calling_the_same_ticket_at_once_one_gets_stale_version() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let b = on_duty(&db, &q, "MAIN", "Teller B", at(2, 0)).await;
    let w1 = walk_in(&db, "w1", at(3, 0)).await;

    let (ta, tb, br) = (t(&a), t(&b), main_branch());
    let (ra, rb) = tokio::join!(
        q.desk.call(&ta, &br, &w1.id, at(3, 10)),
        q.desk.call(&tb, &br, &w1.id, at(3, 10)),
    );
    let (winner, loser) = match (ra, rb) {
        (Ok(t), Err(e)) => ((t, a.clone()), e),
        (Err(e), Ok(t)) => ((t, b.clone()), e),
        other => panic!("exactly one call wins: {other:?}"),
    };
    assert_eq!(
        code(&loser),
        "staleVersion",
        "a lost race answers STALE_VERSION (Q11)"
    );
    assert_eq!(
        winner.0.counter_teller_id.as_deref(),
        Some(winner.1.as_str())
    );
    assert_eq!(
        events(&db, &w1.id).await.len(),
        2,
        "ISSUE and one CALL; the refused call wrote nothing"
    );
    db.finish().await;
}

#[tokio::test]
async fn recall_keeps_called_and_its_time_and_adds_an_event() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    walk_in(&db, "w1", at(3, 0)).await;
    let called = q
        .desk
        .call_next(&t(&a), &main_branch(), at(3, 10))
        .await
        .expect("call");

    let recalled = q
        .desk
        .recall(&t(&a), &main_branch(), &called.id, at(3, 15))
        .await
        .expect("recall");
    assert_eq!(recalled.status, TicketStatus::Called);
    assert_eq!(
        recalled.called_at,
        Some(at(3, 10)),
        "recall does not move called_at"
    );
    assert_eq!(
        events(&db, &called.id).await,
        vec![
            "ISSUE WAITING -".to_owned(),
            format!("CALL CALLED {a}"),
            format!("RECALL CALLED {a}"),
        ]
    );
    db.finish().await;
}

#[tokio::test]
async fn skip_gives_no_show_and_nothing_moves_it_again() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    walk_in(&db, "w1", at(3, 0)).await;
    let called = q
        .desk
        .call_next(&t(&a), &main_branch(), at(3, 10))
        .await
        .expect("call");

    let skipped = q
        .desk
        .skip(&t(&a), &main_branch(), &called.id, at(3, 15))
        .await
        .expect("skip");
    assert_eq!(
        skipped.status,
        TicketStatus::NoShow,
        "skip is NO_SHOW (owner Q-A3)"
    );
    assert_eq!(stored(&db, &called.id).await, "NO_SHOW W001");

    let (ta, b) = (t(&a), main_branch());
    let id = called.id.as_str();
    assert_eq!(
        refused(q.desk.call(&ta, &b, id, at(3, 16)).await),
        "staleVersion"
    );
    assert_eq!(
        refused(q.desk.recall(&ta, &b, id, at(3, 16)).await),
        "staleVersion"
    );
    assert_eq!(
        refused(q.desk.skip(&ta, &b, id, at(3, 16)).await),
        "staleVersion"
    );
    assert_eq!(
        refused(q.desk.start(&ta, &b, id, at(3, 16)).await),
        "staleVersion"
    );
    assert_eq!(
        refused(q.desk.finish(&ta, &b, id, at(3, 16)).await),
        "staleVersion"
    );
    assert_eq!(
        refused(q.desk.cancel(&ta, &b, id, "customerLeft", at(3, 16)).await),
        "staleVersion"
    );
    assert_eq!(
        stored(&db, id).await,
        "NO_SHOW W001",
        "final: the customer takes a new number"
    );
    assert_eq!(
        events(&db, id).await,
        vec![
            "ISSUE WAITING -".to_owned(),
            format!("CALL CALLED {a}"),
            format!("SKIP NO_SHOW {a}"),
        ]
    );
    db.finish().await;
}

#[tokio::test]
async fn start_and_finish_walk_a_called_ticket_to_completed() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    walk_in(&db, "w1", at(3, 0)).await;
    let called = q
        .desk
        .call_next(&t(&a), &main_branch(), at(3, 10))
        .await
        .expect("call");

    let serving = q
        .desk
        .start(&t(&a), &main_branch(), &called.id, at(3, 12))
        .await
        .expect("start");
    assert_eq!(
        (serving.status, serving.served_at),
        (TicketStatus::Serving, Some(at(3, 12)))
    );
    let done = q
        .desk
        .finish(&t(&a), &main_branch(), &called.id, at(3, 20))
        .await
        .expect("finish");
    assert_eq!(
        (done.status, done.completed_at),
        (TicketStatus::Completed, Some(at(3, 20)))
    );
    assert_eq!(
        db.text(&format!(
            "SELECT status || ' ' || (completed_at - served_at)::text FROM queue_tickets \
             WHERE id = '{}'::uuid",
            called.id
        ))
        .await
        .as_deref(),
        Some("COMPLETED 00:08:00"),
        "the service time the estimate reads (served_at → completed_at)"
    );
    assert_eq!(
        events(&db, &called.id).await,
        vec![
            "ISSUE WAITING -".to_owned(),
            format!("CALL CALLED {a}"),
            format!("START SERVING {a}"),
            format!("FINISH COMPLETED {a}"),
        ]
    );
    db.finish().await;
}

#[tokio::test]
async fn start_and_finish_only_from_the_right_state_else_stale_version() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    walk_in(&db, "w1", at(3, 0)).await;
    let (ta, b) = (t(&a), main_branch());
    let called = q.desk.call_next(&ta, &b, at(3, 10)).await.expect("call");
    let id = called.id.as_str();

    assert_eq!(
        refused(q.desk.finish(&ta, &b, id, at(3, 11)).await),
        "staleVersion",
        "CALLED cannot finish"
    );
    q.desk.start(&ta, &b, id, at(3, 12)).await.expect("start");
    assert_eq!(
        refused(q.desk.start(&ta, &b, id, at(3, 13)).await),
        "staleVersion",
        "SERVING cannot start"
    );
    assert_eq!(
        refused(q.desk.recall(&ta, &b, id, at(3, 13)).await),
        "staleVersion",
        "SERVING cannot be recalled"
    );
    assert_eq!(
        refused(q.desk.skip(&ta, &b, id, at(3, 13)).await),
        "staleVersion",
        "SERVING cannot be skipped"
    );
    assert_eq!(
        refused(q.desk.cancel(&ta, &b, id, "customerLeft", at(3, 13)).await),
        "staleVersion",
        "SERVING ends only in COMPLETED (Q5)"
    );
    q.desk.finish(&ta, &b, id, at(3, 20)).await.expect("finish");
    assert_eq!(
        refused(q.desk.finish(&ta, &b, id, at(3, 21)).await),
        "staleVersion",
        "COMPLETED is final"
    );
    assert_eq!(stored(&db, id).await, "COMPLETED W001");
    assert_eq!(
        events(&db, id).await.len(),
        4,
        "ISSUE, CALL, START, FINISH; no refusal wrote an event"
    );
    db.finish().await;
}

#[tokio::test]
async fn cancel_from_waiting_or_called_keeps_the_reason() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let (ta, b) = (t(&a), main_branch());
    let w1 = walk_in(&db, "w1", at(3, 0)).await;
    let w2 = walk_in(&db, "w2", at(3, 1)).await;

    let waiting_cancelled = q
        .desk
        .cancel(&ta, &b, &w2.id, "customerLeft", at(3, 5))
        .await
        .expect("cancel WAITING");
    assert_eq!(
        (
            waiting_cancelled.status,
            waiting_cancelled.ended_reason.as_deref()
        ),
        (TicketStatus::Cancelled, Some("customerLeft"))
    );
    let called = q.desk.call_next(&ta, &b, at(3, 10)).await.expect("call");
    assert_eq!(called.id, w1.id, "a cancelled ticket is not called");
    let called_cancelled = q
        .desk
        .cancel(&ta, &b, &w1.id, "wrongLine", at(3, 11))
        .await
        .expect("cancel CALLED");
    assert_eq!(called_cancelled.ended_reason.as_deref(), Some("wrongLine"));

    for (id, reason) in [(&w2.id, "customerLeft"), (&w1.id, "wrongLine")] {
        assert_eq!(
            db.text(&format!(
                "SELECT t.status || ' ' || t.ended_reason || ' ' || e.kind || ' ' || e.reason \
                 FROM queue_tickets t JOIN queue_events e ON e.ticket_id = t.id AND e.kind = 'CANCEL' \
                 WHERE t.id = '{id}'::uuid"
            ))
            .await
            .as_deref(),
            Some(format!("CANCELLED {reason} CANCEL {reason}").as_str()),
            "the reason is on the ticket and on its CANCEL event"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn a_teller_holding_a_called_or_serving_ticket_cannot_call() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let (ta, b) = (t(&a), main_branch());
    walk_in(&db, "w1", at(3, 0)).await;
    let w2 = walk_in(&db, "w2", at(3, 1)).await;
    let held = q.desk.call_next(&ta, &b, at(3, 10)).await.expect("call");

    assert_eq!(
        refused(q.desk.call_next(&ta, &b, at(3, 11)).await),
        "alreadyServing",
        "holding a CALLED ticket"
    );
    assert_eq!(
        refused(q.desk.call(&ta, &b, &w2.id, at(3, 11)).await),
        "alreadyServing"
    );
    q.desk
        .start(&ta, &b, &held.id, at(3, 12))
        .await
        .expect("start");
    assert_eq!(
        refused(q.desk.call_next(&ta, &b, at(3, 13)).await),
        "alreadyServing",
        "holding a SERVING ticket"
    );
    assert_eq!(
        stored(&db, &w2.id).await,
        "WAITING W002",
        "the refusals moved nothing"
    );

    q.desk
        .finish(&ta, &b, &held.id, at(3, 20))
        .await
        .expect("finish");
    let next = q
        .desk
        .call_next(&ta, &b, at(3, 21))
        .await
        .expect("free again after finishing");
    assert_eq!(next.id, w2.id);
    db.finish().await;
}

#[tokio::test]
async fn a_teller_without_an_open_shift_cannot_call() {
    let (db, q) = open_main().await;
    let off = db.add_user("Teller Off").await;
    let w1 = walk_in(&db, "w1", at(3, 0)).await;
    let (toff, b) = (t(&off), main_branch());

    assert_eq!(
        refused(q.desk.call_next(&toff, &b, at(3, 10)).await),
        "noOpenShift",
        "no shift at all (Q7)"
    );
    assert_eq!(
        refused(q.desk.call(&toff, &b, &w1.id, at(3, 10)).await),
        "noOpenShift"
    );

    let leaver = on_duty(&db, &q, "MAIN", "Teller Leaver", at(2, 0)).await;
    close_shift(&db, &q, "MAIN", &leaver, at(2, 30)).await;
    assert_eq!(
        refused(q.desk.call_next(&t(&leaver), &b, at(3, 10)).await),
        "noOpenShift",
        "a closed shift is no shift"
    );

    let elsewhere = db.add_user("Teller Elsewhere").await;
    db.add_branch("B2", "Asia/Bangkok").await;
    open_shift(&db, &q, "B2", &elsewhere, at(2, 0)).await;
    let both = Actor::new(
        &elsewhere,
        ORG_1,
        [
            teller(&elsewhere, "MAIN").roles,
            teller(&elsewhere, "B2").roles,
        ]
        .concat(),
        None,
    );
    assert_eq!(
        refused(q.desk.call_next(&both, &b, at(3, 10)).await),
        "noOpenShift",
        "an OPEN shift in another branch does not count"
    );
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");
    db.finish().await;
}

#[tokio::test]
async fn start_finish_and_recall_belong_to_the_caller_unless_branch_manager() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let other = on_duty(&db, &q, "MAIN", "Teller B", at(2, 0)).await;
    let boss = on_duty(&db, &q, "MAIN", "Manager", at(2, 0)).await;
    let b = main_branch();
    walk_in(&db, "w1", at(3, 0)).await;
    let called = q
        .desk
        .call_next(&t(&a), &b, at(3, 10))
        .await
        .expect("A calls");
    let id = called.id.as_str();

    assert_eq!(
        refused(q.desk.recall(&t(&other), &b, id, at(3, 11)).await),
        "notYourTicket"
    );
    assert_eq!(
        refused(q.desk.start(&t(&other), &b, id, at(3, 11)).await),
        "notYourTicket"
    );
    assert_eq!(stored(&db, id).await, "CALLED W001");

    let m = manager(&boss, "MAIN");
    q.desk
        .recall(&m, &b, id, at(3, 12))
        .await
        .expect("BRANCH_MANAGER recalls another's ticket");
    q.desk
        .start(&m, &b, id, at(3, 13))
        .await
        .expect("BRANCH_MANAGER starts it");
    assert_eq!(
        refused(q.desk.finish(&t(&other), &b, id, at(3, 14)).await),
        "notYourTicket"
    );
    q.desk
        .finish(&m, &b, id, at(3, 15))
        .await
        .expect("BRANCH_MANAGER finishes it");
    assert_eq!(stored(&db, id).await, "COMPLETED W001");
    assert_eq!(
        events(&db, id).await,
        vec![
            "ISSUE WAITING -".to_owned(),
            format!("CALL CALLED {a}"),
            format!("RECALL CALLED {boss}"),
            format!("START SERVING {boss}"),
            format!("FINISH COMPLETED {boss}"),
        ],
        "each event names who acted"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_customer_or_a_teller_of_another_branch_has_no_command() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let w1 = walk_in(&db, "w1", at(3, 0)).await;
    let w2 = walk_in(&db, "w2", at(3, 1)).await;
    q.desk
        .call(&t(&a), &main_branch(), &w2.id, at(3, 2))
        .await
        .expect("A calls W002");
    let cust = db.add_user("A customer").await;
    let stranger = on_duty(&db, &q, "MAIN", "Teller of B2", at(2, 0)).await;
    let outsiders = [
        ("a customer (Q10)", customer(&cust)),
        ("a TELLER of another branch", teller(&stranger, "B2")),
        (
            "a TELLER of another organisation",
            Actor::new(
                &stranger,
                "00000000-0000-0000-0000-000000000099",
                teller(&stranger, "MAIN").roles,
                None,
            ),
        ),
    ];
    let b = main_branch();
    for (who, actor) in &outsiders {
        for (cmd, result) in [
            ("call_next", q.desk.call_next(actor, &b, at(3, 10)).await),
            ("call", q.desk.call(actor, &b, &w1.id, at(3, 10)).await),
            ("recall", q.desk.recall(actor, &b, &w2.id, at(3, 10)).await),
            ("skip", q.desk.skip(actor, &b, &w2.id, at(3, 10)).await),
            ("start", q.desk.start(actor, &b, &w2.id, at(3, 10)).await),
            ("finish", q.desk.finish(actor, &b, &w2.id, at(3, 10)).await),
            (
                "cancel",
                q.desk.cancel(actor, &b, &w1.id, "x", at(3, 10)).await,
            ),
        ] {
            assert_eq!(
                refused(result),
                "forbidden",
                "{who}: {cmd} needs QueueOperate in MAIN"
            );
        }
    }
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");
    assert_eq!(stored(&db, &w2.id).await, "CALLED W002");
    assert_eq!(
        db.count("SELECT count(*) FROM queue_events").await,
        3,
        "two ISSUE, one CALL"
    );
    db.finish().await;
}

#[tokio::test]
async fn nothing_waiting_and_an_unknown_ticket_are_refused() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let (ta, b) = (t(&a), main_branch());
    assert_eq!(
        refused(q.desk.call_next(&ta, &b, at(3, 0)).await),
        "nothingWaiting"
    );

    let unknown = "00000000-0000-0000-0000-00000000dead";
    assert_eq!(
        refused(q.desk.call(&ta, &b, unknown, at(3, 0)).await),
        "notFound"
    );
    assert_eq!(
        refused(q.desk.cancel(&ta, &b, unknown, "x", at(3, 0)).await),
        "notFound"
    );

    db.add_branch("B2", "Asia/Bangkok").await;
    db.open_queue("B2").await;
    let theirs = db
        .issuer()
        .issue(&BranchId::new("B2"), IssueSource::Desk, "b2-1", at(3, 1))
        .await
        .expect("issue on B2");
    assert_eq!(
        refused(q.desk.call(&ta, &b, &theirs.id, at(3, 2)).await),
        "notFound",
        "a ticket of another branch is not found in this one"
    );
    assert_eq!(
        refused(q.desk.call_next(&ta, &b, at(3, 2)).await),
        "nothingWaiting",
        "another branch's waiting ticket is not MAIN's"
    );
    assert_eq!(stored(&db, &theirs.id).await, "WAITING W001");
    db.finish().await;
}

#[tokio::test]
async fn every_transition_writes_one_event_and_raises_the_branch_version() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let (ta, b) = (t(&a), main_branch());
    let w1 = walk_in(&db, "w1", at(3, 0)).await;
    let id = w1.id.clone();

    let mut version = branch_version(&db, "MAIN").await;
    let mut queue_v = queue_version(&db, "MAIN").await;
    let mut events_n = db.count("SELECT count(*) FROM queue_events").await;
    for (step, m) in ["call", "recall", "start", "finish"]
        .into_iter()
        .zip(10u32..)
    {
        let now = at(3, m);
        match step {
            "call" => q.desk.call(&ta, &b, &id, now).await,
            "recall" => q.desk.recall(&ta, &b, &id, now).await,
            "start" => q.desk.start(&ta, &b, &id, now).await,
            _ => q.desk.finish(&ta, &b, &id, now).await,
        }
        .unwrap_or_else(|e| panic!("{step}: {e:?}"));
        let (v, qv, n) = (
            branch_version(&db, "MAIN").await,
            queue_version(&db, "MAIN").await,
            db.count("SELECT count(*) FROM queue_events").await,
        );
        assert_eq!(v, version + 1, "{step} runs in one BranchTx (Q13)");
        assert!(
            qv > queue_v,
            "{step} raises branch_service_status.queue_version ({queue_v} → {qv})"
        );
        assert_eq!(
            n,
            events_n + 1,
            "{step} writes exactly one queue_events row"
        );
        (version, queue_v, events_n) = (v, qv, n);
    }

    assert_eq!(
        refused(q.desk.start(&ta, &b, &id, at(3, 30)).await),
        "staleVersion"
    );
    assert_eq!(
        branch_version(&db, "MAIN").await,
        version,
        "a refusal commits nothing"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM queue_events").await,
        events_n
    );
    db.finish().await;
}
