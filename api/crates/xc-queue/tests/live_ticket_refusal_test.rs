//! `ticketStillServing` names the blocking tickets (design `docs/design/blueprint/queue-tickets.md` §4.7, "The refusal
//! names the blocking tickets", design PR #5992 with the lane controller's follow-up 2).
//!
//! Pinned here, at the crate level:
//! - `LiveTicket::refusal` / `LiveTicket::from_refusal` carry the tickets through `AppError`'s message after a marker,
//!   the way `StockShortfall` does: a round trip gives the same tickets, and a message without the marker gives none.
//! - `AppError::from(QueueError::TicketStillServing(tickets))` is `Validation(message)`, whose message starts with the
//!   code (so the cash routes' prefix match still routes it) and reads back the same tickets.
//! - `CounterService::on_shift_closed` answers every live ticket of the closing teller, CALLED and SERVING, ordered by
//!   `called_at`, then number, with the teller's name.
//!
//! Written before the implementation by the ticket author claude/b091f5e0. Do not edit, delete or weaken; add tests in
//! a file of your own. `[required]` names: `xc_queue::domain::ticket::live_ticket::LiveTicket { number, status,
//! teller_id, teller_name: Option<String> }` (Debug, Clone, PartialEq, Eq), `LiveTicket::refusal(&[LiveTicket]) -> String`,
//! `LiveTicket::from_refusal(&str) -> Vec<LiveTicket>`, and `QueueError::TicketStillServing(Vec<LiveTicket>)`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;
mod queue_desk_common;

use chrono::{DateTime, Utc};
use pos_error::AppError;
use queue_common::*;
use queue_desk_common::*;
use xc_kernel::ids::BranchId;
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::live_ticket::LiveTicket;
use xc_queue::domain::ticket::queue_error::QueueError;
use xc_queue::domain::ticket::queue_ticket::QueueTicket;

fn ticket(number: &str, status: &str, teller: &str, name: Option<&str>) -> LiveTicket {
    LiveTicket {
        number: number.to_owned(),
        status: status.to_owned(),
        teller_id: teller.to_owned(),
        teller_name: name.map(str::to_owned),
    }
}

/// F1: a round trip through the refusal message gives back the same tickets, in order.
#[test]
fn f1_the_refusal_message_round_trips_the_tickets() {
    let tickets = vec![
        ticket(
            "W015",
            "SERVING",
            "00000000-0000-0000-0000-0000000000a1",
            Some("Teller One"),
        ),
        ticket(
            "W016",
            "CALLED",
            "00000000-0000-0000-0000-0000000000a1",
            None,
        ),
    ];
    let message = LiveTicket::refusal(&tickets);
    assert!(
        message.starts_with("ticketStillServing"),
        "the message starts with the code: {message}"
    );
    assert_eq!(LiveTicket::from_refusal(&message), tickets);
}

/// F2: a message without the marker (the bare code, or any other refusal) carries no tickets.
#[test]
fn f2_a_message_without_the_marker_gives_no_tickets() {
    assert_eq!(LiveTicket::from_refusal("ticketStillServing"), vec![]);
    assert_eq!(LiveTicket::from_refusal("staleVersion"), vec![]);
    assert_eq!(LiveTicket::from_refusal(""), vec![]);
}

/// F3: `AppError::from(TicketStillServing(tickets))` is `Validation`, starts with the code, and reads back the tickets.
#[test]
fn f3_the_app_error_carries_the_tickets_and_starts_with_the_code() {
    let tickets = vec![ticket(
        "W015",
        "SERVING",
        "00000000-0000-0000-0000-0000000000a1",
        Some("Teller One"),
    )];
    match AppError::from(QueueError::TicketStillServing(tickets.clone())) {
        AppError::Validation(message) => {
            assert!(message.starts_with("ticketStillServing"), "{message}");
            assert_eq!(LiveTicket::from_refusal(&message), tickets);
        }
        other => panic!("expected Validation, got {other:?}"),
    }
}

async fn walk_in(db: &TestDb, key: &str, now: DateTime<Utc>) -> QueueTicket {
    db.issuer()
        .issue(&main_branch(), IssueSource::Desk, key, now)
        .await
        .unwrap_or_else(|e| panic!("issue {key}: {e:?}"))
}

/// F4: the guard answers every live ticket of the closing teller, CALLED and SERVING, ordered by `called_at`, with the
/// teller's name; another teller's live ticket and WAITING tickets are not listed.
#[tokio::test]
async fn f4_the_guard_lists_every_live_ticket_of_the_closing_teller() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let q = queue(&db);
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let b = on_duty(&db, &q, "MAIN", "Teller B", at(2, 0)).await;
    let first = walk_in(&db, "f4-1", at(2, 1)).await;
    let second = walk_in(&db, "f4-2", at(2, 1)).await;
    let other = walk_in(&db, "f4-3", at(2, 1)).await;
    let _waiting = walk_in(&db, "f4-4", at(2, 1)).await;
    let me = teller(&a, "MAIN");
    q.desk
        .call(&me, &main_branch(), &first.id, at(2, 2))
        .await
        .expect("call the first");
    q.desk
        .start(&me, &main_branch(), &first.id, at(2, 3))
        .await
        .expect("serve the first");
    // The desk lets a teller hold one live ticket at a time (`alreadyServing`); a second can exist only through data
    // written outside it, such as a row from before #5675. The guard must list it all the same.
    db.text(&format!(
        "UPDATE queue_tickets SET status = 'CALLED', counter_teller_id = '{a}'::uuid, \
             called_at = '{}'::timestamptz WHERE id = '{}'::uuid RETURNING id::text",
        at(2, 4).to_rfc3339(),
        second.id
    ))
    .await
    .expect("a second live ticket at A's counter");
    q.desk
        .call(&teller(&b, "MAIN"), &main_branch(), &other.id, at(2, 4))
        .await
        .expect("teller B calls another");

    let counters = q.counters.clone();
    let t = a.clone();
    let refusal = db
        .branch_tx()
        .run(&main_branch(), move |tx| {
            Box::pin(async move {
                counters
                    .on_shift_closed(tx, &BranchId::new("MAIN"), &t, at(3, 0))
                    .await
            })
        })
        .await
        .expect_err("refused while two tickets are live");
    let message = match refusal {
        AppError::Validation(m) => m,
        other => panic!("expected Validation, got {other:?}"),
    };
    assert_eq!(
        LiveTicket::from_refusal(&message),
        vec![
            ticket(&first.display_number, "SERVING", &a, Some("Teller A")),
            ticket(&second.display_number, "CALLED", &a, Some("Teller A")),
        ],
        "both of A's live tickets, by called_at; not B's, not the WAITING one"
    );
    db.finish().await;
}
