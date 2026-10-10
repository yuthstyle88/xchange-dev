//! QU-A1 (#556 design rows QU-2/QU-3) — the ticket's own rules, no database: the service types,
//! the display number, the issue source, the refusal codes, and `QueueTicket::apply`, the state
//! machine of design §4.2. (The desk that sends these commands, and who may send them, is QU-A2.)
//! Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/queue-tickets.md` Q2, Q3, Q5, §4.2, §9.1 rows QU-2 (`W001`/
//! `B001`) and QU-3 (recall keeps CALLED and adds an event; skip gives NO_SHOW and nothing moves it
//! again; start/finish only from the right state, else `STALE_VERSION`). Owner Q-A3 (skip is
//! NO_SHOW, final).

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use pos_error::AppError;
use xc_kernel::ids::BranchId;
use xc_kernel::queue_tickets::QueueTicketSource;
use xc_queue::domain::ticket::display_number::DisplayNumber;
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::queue_error::QueueError;
use xc_queue::domain::ticket::queue_event::EventKind;
use xc_queue::domain::ticket::queue_ticket::{QueueTicket, TicketCommand, TicketStatus};
use xc_queue::domain::ticket::service_type::ServiceType;

fn at(m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 2, 3, m, 0).unwrap()
}

fn waiting() -> QueueTicket {
    QueueTicket {
        id: "00000000-0000-0000-0000-0000000000a1".to_owned(),
        organization_id: "00000000-0000-0000-0000-000000000001".to_owned(),
        branch_id: BranchId::new("MAIN"),
        service_date: NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
        service_type: ServiceType::WalkIn,
        sequence: 7,
        display_number: "W007".to_owned(),
        status: TicketStatus::Waiting,
        customer_id: None,
        booking_id: None,
        kiosk_request_id: None,
        counter_teller_id: None,
        issued_at: at(0),
        called_at: None,
        served_at: None,
        completed_at: None,
        ended_reason: None,
        idempotency_key: "k".to_owned(),
    }
}

const TELLER: &str = "00000000-0000-0000-0000-0000000000b1";

fn call() -> TicketCommand {
    TicketCommand::Call {
        teller_id: TELLER.to_owned(),
    }
}

fn cancel() -> TicketCommand {
    TicketCommand::Cancel {
        reason: "customerLeft".to_owned(),
    }
}

fn step(ticket: &QueueTicket, cmd: TicketCommand, m: u32) -> QueueTicket {
    let t = ticket
        .apply(&cmd, at(m))
        .unwrap_or_else(|e| panic!("{cmd:?} from {:?}: {e:?}", ticket.status));
    t.ticket
}

fn called() -> QueueTicket {
    step(&waiting(), call(), 1)
}

fn serving() -> QueueTicket {
    step(&called(), TicketCommand::Start, 2)
}

fn all_commands() -> Vec<TicketCommand> {
    vec![
        call(),
        TicketCommand::Recall,
        TicketCommand::Skip,
        TicketCommand::Start,
        TicketCommand::Finish,
        cancel(),
    ]
}

#[test]
fn the_service_types_and_their_letters() {
    assert_eq!(ServiceType::WalkIn.as_str(), "WALK_IN");
    assert_eq!(ServiceType::Booking.as_str(), "BOOKING");
    assert_eq!(ServiceType::parse("WALK_IN"), Some(ServiceType::WalkIn));
    assert_eq!(ServiceType::parse("BOOKING"), Some(ServiceType::Booking));
    assert_eq!(ServiceType::parse("walk_in"), None);
    assert_eq!(ServiceType::WalkIn.letter(), 'W');
    assert_eq!(ServiceType::Booking.letter(), 'B');
}

#[test]
fn the_display_number_is_the_letter_and_the_sequence_padded_to_three() {
    assert_eq!(DisplayNumber::format(ServiceType::WalkIn, 1), "W001");
    assert_eq!(DisplayNumber::format(ServiceType::Booking, 1), "B001");
    assert_eq!(DisplayNumber::format(ServiceType::WalkIn, 42), "W042");
    assert_eq!(DisplayNumber::format(ServiceType::Booking, 999), "B999");
    assert_eq!(
        DisplayNumber::format(ServiceType::WalkIn, 1000),
        "W1000",
        "a long day keeps every digit"
    );
}

#[test]
fn the_source_decides_the_line() {
    assert_eq!(
        IssueSource::Kiosk {
            request_id: "KR-1".to_owned()
        }
        .service_type(),
        ServiceType::WalkIn
    );
    assert_eq!(IssueSource::Desk.service_type(), ServiceType::WalkIn);
    assert_eq!(
        IssueSource::Booking {
            booking_id: "BK-1".to_owned()
        }
        .service_type(),
        ServiceType::Booking
    );
    assert_eq!(
        IssueSource::from(&QueueTicketSource::Kiosk {
            request_id: "KR-1".to_owned()
        }),
        IssueSource::Kiosk {
            request_id: "KR-1".to_owned()
        }
    );
    assert_eq!(
        IssueSource::from(&QueueTicketSource::Booking {
            booking_id: "BK-1".to_owned()
        }),
        IssueSource::Booking {
            booking_id: "BK-1".to_owned()
        }
    );
}

#[test]
fn the_statuses_and_event_kinds_as_stored() {
    let statuses = [
        (TicketStatus::Waiting, "WAITING"),
        (TicketStatus::Called, "CALLED"),
        (TicketStatus::Serving, "SERVING"),
        (TicketStatus::Completed, "COMPLETED"),
        (TicketStatus::Cancelled, "CANCELLED"),
        (TicketStatus::NoShow, "NO_SHOW"),
    ];
    for (status, text) in statuses {
        assert_eq!(status.as_str(), text);
        assert_eq!(TicketStatus::parse(text), Some(status));
    }
    assert_eq!(TicketStatus::parse("PAUSED"), None);
    let kinds = [
        (EventKind::Issue, "ISSUE"),
        (EventKind::Call, "CALL"),
        (EventKind::Recall, "RECALL"),
        (EventKind::Start, "START"),
        (EventKind::Finish, "FINISH"),
        (EventKind::Skip, "SKIP"),
        (EventKind::Cancel, "CANCEL"),
    ];
    for (kind, text) in kinds {
        assert_eq!(kind.as_str(), text);
    }
}

#[test]
fn the_refusal_codes_and_how_they_reach_app_error() {
    assert_eq!(QueueError::QueueClosed.code(), "queueClosed");
    assert_eq!(QueueError::StaleVersion.code(), "staleVersion");
    assert_eq!(QueueError::CounterPaused.code(), "counterPaused");
    assert_eq!(QueueError::AlreadyServing.code(), "alreadyServing");
    assert_eq!(QueueError::NoOpenShift.code(), "noOpenShift");
    assert_eq!(QueueError::NotYourTicket.code(), "notYourTicket");
    assert_eq!(QueueError::NothingWaiting.code(), "nothingWaiting");
    assert_eq!(QueueError::NotFound.code(), "notFound");
    assert!(matches!(
        AppError::from(QueueError::QueueClosed),
        AppError::Validation(code) if code == "queueClosed"
    ));
    assert!(matches!(
        AppError::from(QueueError::StaleVersion),
        AppError::Conflict(code) if code == "staleVersion"
    ));
}

#[test]
fn call_start_finish_walk_the_ticket_to_completed() {
    let t = waiting().apply(&call(), at(1)).expect("WAITING → CALLED");
    assert_eq!(t.event, EventKind::Call);
    assert_eq!(t.ticket.status, TicketStatus::Called);
    assert_eq!(t.ticket.called_at, Some(at(1)));
    assert_eq!(t.ticket.counter_teller_id.as_deref(), Some(TELLER));

    let s = t
        .ticket
        .apply(&TicketCommand::Start, at(2))
        .expect("CALLED → SERVING");
    assert_eq!(s.event, EventKind::Start);
    assert_eq!(s.ticket.status, TicketStatus::Serving);
    assert_eq!(s.ticket.served_at, Some(at(2)));
    assert_eq!(s.ticket.called_at, Some(at(1)));

    let f = s
        .ticket
        .apply(&TicketCommand::Finish, at(9))
        .expect("SERVING → COMPLETED");
    assert_eq!(f.event, EventKind::Finish);
    assert_eq!(f.ticket.status, TicketStatus::Completed);
    assert_eq!(f.ticket.completed_at, Some(at(9)));
    assert_eq!(f.ticket.served_at, Some(at(2)));
    let unchanged = QueueTicket {
        status: TicketStatus::Completed,
        called_at: Some(at(1)),
        served_at: Some(at(2)),
        completed_at: Some(at(9)),
        counter_teller_id: Some(TELLER.to_owned()),
        ..waiting()
    };
    assert_eq!(f.ticket, unchanged, "nothing else on the ticket moves");
}

#[test]
fn recall_keeps_called_and_its_time_and_adds_an_event() {
    let before = called();
    let r = before
        .apply(&TicketCommand::Recall, at(5))
        .expect("CALLED → CALLED");
    assert_eq!(r.event, EventKind::Recall);
    assert_eq!(
        r.ticket, before,
        "recall changes nothing on the ticket, not even called_at"
    );
}

#[test]
fn skip_gives_no_show_and_nothing_moves_it_again() {
    let s = called()
        .apply(&TicketCommand::Skip, at(5))
        .expect("CALLED → NO_SHOW");
    assert_eq!(s.event, EventKind::Skip);
    assert_eq!(s.ticket.status, TicketStatus::NoShow);
    for cmd in all_commands() {
        assert_eq!(
            s.ticket.apply(&cmd, at(6)),
            Err(QueueError::StaleVersion),
            "{cmd:?} on a NO_SHOW ticket: final (owner Q-A3)"
        );
    }
}

#[test]
fn cancel_from_waiting_or_called_keeps_the_reason() {
    for from in [waiting(), called()] {
        let c = from.apply(&cancel(), at(5)).expect("cancel");
        assert_eq!(c.event, EventKind::Cancel);
        assert_eq!(c.ticket.status, TicketStatus::Cancelled);
        assert_eq!(c.ticket.ended_reason.as_deref(), Some("customerLeft"));
    }
    assert_eq!(
        serving().apply(&cancel(), at(5)),
        Err(QueueError::StaleVersion),
        "SERVING ends only in COMPLETED (Q5)"
    );
}

#[test]
fn every_command_outside_the_table_is_stale() {
    let completed = step(&serving(), TicketCommand::Finish, 3);
    let cancelled = step(&waiting(), cancel(), 3);
    // (state, the commands §4.2 allows from it)
    let allowed: [(QueueTicket, &[&str]); 5] = [
        (waiting(), &["Call", "Cancel"]),
        (called(), &["Recall", "Skip", "Start", "Cancel"]),
        (serving(), &["Finish"]),
        (completed, &[]),
        (cancelled, &[]),
    ];
    for (ticket, ok) in allowed {
        for cmd in all_commands() {
            let name = format!("{cmd:?}");
            let name = name.split([' ', '{']).next().unwrap_or("").to_owned();
            let result = ticket.apply(&cmd, at(7));
            if ok.contains(&name.as_str()) {
                assert!(result.is_ok(), "{name} from {:?} is allowed", ticket.status);
            } else {
                assert_eq!(
                    result,
                    Err(QueueError::StaleVersion),
                    "{name} from {:?} answers staleVersion",
                    ticket.status
                );
            }
        }
    }
}
