//! BK-13 (design `docs/design/blueprint/booking-slots.md` B12, §5.2): `effective_status`, the reader half of B12.
//! Written before the implementation by the ticket author (claude/c4d72c). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! A reader shows an overdue row as the state the sweeper would write next, with the writers' own predicates:
//! `PENDING_CUSTOMER_CONFIRMATION` → `EXPIRED` when `confirm_expires_at` is `None` or `now > confirm_expires_at`
//! (`BookingMachine::expire`); `CONFIRMED` → `NO_SHOW` when `now > grace_until` (`BookingMachine::no_show`,
//! `BookingPolicy::grace_until`); `CHECKED_IN` → `CANCELLED` when `service_date < branch_today` and no active ticket
//! (the sweeper's day-ended query). A boundary instant is not overdue. Every other status is shown as stored.

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use xc_booking::domain::booking::booking_status::BookingStatus;
use xc_booking::domain::booking::effective_status::{OverdueFacts, effective_status};

fn at(h: u32, m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 16, h, m, 0).unwrap()
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

/// A booking whose slot is today (16 October) at 10:00 with a 15-minute grace; confirm expiry at 09:00; no ticket.
fn facts() -> OverdueFacts {
    OverdueFacts {
        confirm_expires_at: Some(at(9, 0)),
        grace_until: at(10, 15),
        service_date: day(16),
        branch_today: day(16),
        has_active_ticket: false,
    }
}

const ALL: [BookingStatus; 8] = [
    BookingStatus::PendingCustomerConfirmation,
    BookingStatus::Confirmed,
    BookingStatus::CheckedIn,
    BookingStatus::InService,
    BookingStatus::Completed,
    BookingStatus::Cancelled,
    BookingStatus::NoShow,
    BookingStatus::Expired,
];

#[test]
fn a_pending_booking_expires_strictly_after_its_confirm_expiry() {
    let f = facts();
    let pending = BookingStatus::PendingCustomerConfirmation;
    assert_eq!(effective_status(pending, &f, at(8, 59)), pending);
    assert_eq!(
        effective_status(pending, &f, at(9, 0)),
        pending,
        "the boundary is not overdue"
    );
    assert_eq!(
        effective_status(pending, &f, at(9, 0) + Duration::milliseconds(1)),
        BookingStatus::Expired
    );
}

#[test]
fn a_pending_booking_with_no_confirm_expiry_is_expired_as_the_sweeper_expires_it() {
    let f = OverdueFacts {
        confirm_expires_at: None,
        ..facts()
    };
    assert_eq!(
        effective_status(BookingStatus::PendingCustomerConfirmation, &f, at(8, 0)),
        BookingStatus::Expired
    );
}

#[test]
fn a_confirmed_booking_is_a_no_show_strictly_after_start_plus_grace() {
    let f = facts();
    let confirmed = BookingStatus::Confirmed;
    assert_eq!(effective_status(confirmed, &f, at(10, 0)), confirmed);
    assert_eq!(
        effective_status(confirmed, &f, at(10, 15)),
        confirmed,
        "the boundary is not overdue"
    );
    assert_eq!(
        effective_status(confirmed, &f, at(10, 15) + Duration::milliseconds(1)),
        BookingStatus::NoShow
    );
    assert_eq!(
        effective_status(confirmed, &f, at(23, 0)),
        BookingStatus::NoShow
    );
}

#[test]
fn a_checked_in_booking_ends_with_its_service_day_only_when_no_ticket_is_active() {
    let checked_in = BookingStatus::CheckedIn;
    // the same service day, however late: still checked in
    assert_eq!(
        effective_status(checked_in, &facts(), at(23, 59)),
        checked_in
    );
    // the branch's next day, no active ticket: the day ended
    let next_day = OverdueFacts {
        branch_today: day(17),
        ..facts()
    };
    assert_eq!(
        effective_status(checked_in, &next_day, at(23, 59)),
        BookingStatus::Cancelled
    );
    // the branch's next day, a ticket still WAITING/CALLED/SERVING: not ended
    let with_ticket = OverdueFacts {
        has_active_ticket: true,
        ..next_day
    };
    assert_eq!(
        effective_status(checked_in, &with_ticket, at(23, 59)),
        checked_in
    );
}

#[test]
fn the_branch_date_decides_the_day_not_the_utc_instant() {
    // 17:30Z on the 16th is 00:30 on the 17th in Bangkok: the reader passes the branch's date, and only it decides
    let late = at(17, 30);
    let same_branch_day = facts();
    assert_eq!(
        effective_status(BookingStatus::CheckedIn, &same_branch_day, late),
        BookingStatus::CheckedIn
    );
    let next_branch_day = OverdueFacts {
        branch_today: day(17),
        ..facts()
    };
    assert_eq!(
        effective_status(BookingStatus::CheckedIn, &next_branch_day, late),
        BookingStatus::Cancelled
    );
}

#[test]
fn every_other_status_is_shown_as_stored_however_overdue() {
    // long after every boundary, on a later branch day, with no ticket
    let f = OverdueFacts {
        branch_today: day(20),
        ..facts()
    };
    let now = at(23, 59);
    for status in ALL {
        let moved = matches!(
            status,
            BookingStatus::PendingCustomerConfirmation
                | BookingStatus::Confirmed
                | BookingStatus::CheckedIn
        );
        if !moved {
            assert_eq!(effective_status(status, &f, now), status, "{status:?}");
        }
    }
    // IN_SERVICE in particular: a ticket is being served; the sweeper never touches it
    assert_eq!(
        effective_status(BookingStatus::InService, &f, now),
        BookingStatus::InService
    );
}
