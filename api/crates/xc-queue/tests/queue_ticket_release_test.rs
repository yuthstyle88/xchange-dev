//! #5541 (design `docs/design/blueprint/queue-tickets.md` §4.2, §4.6): the state machine's `Release`, SERVING →
//! CANCELLED with reason `NO_SALE` and a CANCEL event. No database. `Cancel` is unchanged: SERVING + `Cancel` is
//! still stale (`queue_ticket_machine_test.rs`).
//!
//! Written before the implementation by the ticket author (claude/2ca5874b). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! Assumes (skeleton on `test/5541`, [required]): `TicketCommand::Release` and
//! `xc_queue::domain::ticket::queue_ticket::NO_SALE`.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use xc_kernel::ids::BranchId;
use xc_queue::domain::ticket::queue_error::QueueError;
use xc_queue::domain::ticket::queue_event::EventKind;
use xc_queue::domain::ticket::queue_ticket::{NO_SALE, QueueTicket, TicketCommand, TicketStatus};
use xc_queue::domain::ticket::service_type::ServiceType;

fn at(m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 29, 12, m, 0).unwrap()
}

fn ticket(status: TicketStatus) -> QueueTicket {
    QueueTicket {
        id: "00000000-0000-0000-0000-0000000000a1".to_owned(),
        organization_id: "00000000-0000-0000-0000-000000000001".to_owned(),
        branch_id: BranchId::new("MAIN"),
        service_date: NaiveDate::from_ymd_opt(2026, 10, 29).unwrap(),
        service_type: ServiceType::WalkIn,
        sequence: 13,
        display_number: "W013".to_owned(),
        status,
        customer_id: None,
        booking_id: None,
        kiosk_request_id: None,
        counter_teller_id: Some("00000000-0000-0000-0000-0000000000b1".to_owned()),
        issued_at: at(0),
        called_at: Some(at(1)),
        served_at: Some(at(2)),
        completed_at: None,
        ended_reason: None,
        idempotency_key: "k".to_owned(),
    }
}

#[test]
fn the_reason_is_no_sale() {
    assert_eq!(NO_SALE, "noSale");
}

#[test]
fn release_ends_a_serving_ticket_as_cancelled_with_no_sale_and_a_cancel_event() {
    let t = ticket(TicketStatus::Serving)
        .apply(&TicketCommand::Release, at(9))
        .expect("SERVING → CANCELLED");

    assert_eq!(t.event, EventKind::Cancel);
    assert_eq!(t.ticket.status, TicketStatus::Cancelled);
    assert_eq!(t.ticket.ended_reason.as_deref(), Some(NO_SALE));
    assert_eq!(t.ticket.completed_at, None, "no sale was completed");
    assert_eq!(
        t.ticket.served_at,
        Some(at(2)),
        "when it was served is kept"
    );
}

#[test]
fn release_from_any_other_state_is_stale() {
    for status in [
        TicketStatus::Waiting,
        TicketStatus::Called,
        TicketStatus::Completed,
        TicketStatus::Cancelled,
        TicketStatus::NoShow,
    ] {
        assert_eq!(
            ticket(status).apply(&TicketCommand::Release, at(9)),
            Err(QueueError::StaleVersion),
            "{status:?}"
        );
    }
}

#[test]
fn ticket_has_sale_is_its_own_v3_code() {
    assert_eq!(QueueError::TicketHasSale.code(), "ticketHasSale");
}
