//! BK-2 #1379 — `BookingPolicy` (design `docs/design/blueprint/booking-slots.md` B11, §0 Q-B2–Q-B4; ticket §7
//! `[required]`: *"`confirm_expires_at` = made + `confirm_expiry_minutes`, never later than `slot.start_at −
//! cancel_cutoff_minutes`"*). Pure: no database. The values here are deliberately not the seeded 60 / 15 / 60, so
//! a constant in code cannot pass (ticket §4 item 2, B11: no number is in code).
//! Written before the implementation by the test writer (claude/dell-claude-4, on the BE lane controller
//! claude/ad05bf8d's order). Do not edit, delete or weaken; add tests in a file of your own.

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use uuid::Uuid;
use xc_booking::domain::policy::booking_policy::BookingPolicy;
use xc_booking::domain::slot::booking_slot::{BookingSlot, SlotStatus};
use xc_kernel::ids::BranchId;

fn start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 7, 3, 0, 0)
        .single()
        .expect("an instant")
}

fn slot() -> BookingSlot {
    BookingSlot {
        id: Uuid::new_v4(),
        organization_id: Uuid::nil(),
        branch_id: BranchId::new("MAIN"),
        service_date: NaiveDate::from_ymd_opt(2030, 1, 7).expect("a date"),
        start_at: start(),
        end_at: start() + Duration::minutes(30),
        capacity: 2,
        reserved: 0,
        status: SlotStatus::Open,
    }
}

fn policy(cutoff: i32, grace: i32, expiry: i32) -> BookingPolicy {
    BookingPolicy {
        cancel_cutoff_minutes: cutoff,
        no_show_grace_minutes: grace,
        confirm_expiry_minutes: expiry,
    }
}

#[test]
fn the_cutoff_is_the_slot_start_less_the_cutoff_minutes() {
    assert_eq!(
        policy(45, 10, 30).cutoff_at(&slot()),
        start() - Duration::minutes(45)
    );
    assert_eq!(
        policy(0, 10, 30).cutoff_at(&slot()),
        start(),
        "a cutoff of 0 is the start"
    );
}

#[test]
fn the_grace_ends_the_grace_minutes_after_the_slot_start() {
    assert_eq!(
        policy(45, 10, 30).grace_until(&slot()),
        start() + Duration::minutes(10)
    );
    assert_eq!(
        policy(45, 0, 30).grace_until(&slot()),
        start(),
        "a grace of 0 is the start"
    );
}

#[test]
fn a_confirmation_expires_the_expiry_minutes_after_it_was_made() {
    // Made 5 hours before the slot: 30 minutes later is well before the cutoff (45 minutes before).
    let now = start() - Duration::hours(5);
    assert_eq!(
        policy(45, 10, 30).confirm_expires_at(now, &slot()),
        now + Duration::minutes(30)
    );
}

#[test]
fn a_confirmation_never_expires_later_than_the_cutoff() {
    // Made 50 minutes before the slot: +30 would be 20 minutes before; the cutoff (45 before) comes first.
    let now = start() - Duration::minutes(50);
    let p = policy(45, 10, 30);
    assert_eq!(p.confirm_expires_at(now, &slot()), p.cutoff_at(&slot()));
    assert_eq!(
        p.confirm_expires_at(now, &slot()),
        start() - Duration::minutes(45)
    );
}

#[test]
fn a_confirmation_ending_exactly_at_the_cutoff_ends_there() {
    let now = start() - Duration::minutes(75);
    assert_eq!(
        policy(45, 10, 30).confirm_expires_at(now, &slot()),
        start() - Duration::minutes(45)
    );
}
