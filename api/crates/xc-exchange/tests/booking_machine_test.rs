//! Written before the implementation (ticket T15-a, #407). Do not edit, delete or weaken.
//!
//! `BookingMachine`: every transition of a booking and the order of its refusals, as design
//! `docs/design/xchange-v3.md` §5.6.3 states them (*States*, the `create`, `confirm`, `cancel`
//! rows, *Using a booking at the desk*) and §5.6.8 names the tests (rows marked T15-a).
//!
//! - `create`: a non-member's booking is CONFIRMED for the rule's minutes, a member's is PENDING
//!   (owner, 2026-10-08, PR 395 issuecomment-6050139400); refusals 8, 9, 10, 11 in that order
//!   (1–6 are the payload's, 7 the service's stored-request check).
//! - `confirm`: refusals 3, 4 (double confirm), 5, 6 in v2's order (10-05 `server.mjs:7248-7255`).
//! - `cancel`: v2 `:7243-7247`.
//! - `redeem` (the desk's save): rows 2–5 (`validateBookingForExchange` `:6955-6962`, the member
//!   check `:7029-7032`); row 1, a booking of another branch, is the service's.
//!
//! The machine is pure: no database, no clock of its own (`now` is given).
//!
//! Amended for #758 (RE-4d) by the ticket author (claude/e5ab6c), `docs/design/blueprint/rates.md`
//! §4.9 R1, R4 and R6: the counter is the current published line (`CounterLine`: its id, its rate
//! per `unit_size`, its unit), not a bare rate; a booking holds the line it was created at and its
//! unit; confirm compares the stored line with the current one and never re-locks a rate. Every
//! booking here is USD, unit 1, on line A, so the rates read as before. The commands' expected
//! rate is the v2 wire's per-1 rate, a `Decimal` of up to 11 places (R3 and the ticket author's
//! ruling 4 on #758), no longer a `Rate`.

use std::str::FromStr;

use chrono::{DateTime, Duration, TimeZone, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side, UnitSize};

use xc_exchange::domain::booking::commands::confirm_booking::ConfirmBooking;
use xc_exchange::domain::booking::commands::create_booking::CreateBooking;
use xc_exchange::domain::booking::commands::redeem_booking::RedeemBooking;
use xc_exchange::domain::booking::entities::booking::Booking;
use xc_exchange::domain::booking::entities::booking_machine::BookingMachine;
use xc_exchange::domain::booking::entities::booking_steps::{
    CancelStep, ConfirmStep, CreateFacts, RedeemStep,
};
use xc_exchange::domain::booking::entities::member::Member;
use xc_exchange::domain::booking::repositories::booking_ports::CounterLine;
use xc_exchange::domain::booking::value_objects::booking_error::BookingError;
use xc_exchange::domain::booking::value_objects::booking_rules::BookingRules;
use xc_exchange::domain::booking::value_objects::booking_status::BookingStatus;

// ── the sentences (v2's, verbatim) ───────────────────────────────────────────

const NO_MEMBER: &str = "ไม่พบ Member ของลูกค้านี้ในสาขานี้";
const CLOSED: &str = "ปิดรับจองใหม่";
const NO_SHIFT: &str = "กรุณาเปิดกะจากหน้าแอดมินก่อน";
const RATE_MOVED: &str = "เรทเปลี่ยน กรุณาโหลดใหม่";
const EXPIRY_IN_FUTURE: &str = "วันหมดอายุต้องอยู่ในอนาคต";
const ONLY_PENDING_MEMBER: &str = "ยืนยันได้เฉพาะคำขอ Member ที่รอยืนยัน";
const ALREADY_USED: &str = "การจองถูกใช้ทำรายการแล้ว";
const USED_BY_ANOTHER: &str = "การจองนี้ถูกใช้ทำรายการแล้ว";
const NOT_USABLE: &str = "การจองยังไม่ยืนยัน หมดอายุ หรือถูกยกเลิกแล้ว";
const FORM_MISMATCH: &str = "ข้อมูลรายการไม่ตรงกับการจอง หรือยังไม่ยืนยันลูกค้า";
const MEMBER_MISMATCH: &str = "ลูกค้าไม่ตรงกับ Member ที่จอง";

// ── helpers ──────────────────────────────────────────────────────────────────

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 8, 3, 0, 0).unwrap()
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).unwrap()
}

/// The USD line a booking was created at (A) and another, later one (B).
const LINE_A: u128 = 0x0000_0000_0000_4000_8000_0000_0000_00a1;
const LINE_B: u128 = 0x0000_0000_0000_4000_8000_0000_0000_00b2;

/// The current USD line `id` at `r` per 1 (RE-4d: the counter is a published line).
fn line(id: u128, r: &str) -> CounterLine {
    CounterLine {
        line_id: Uuid::from_u128(id),
        rate: rate(r),
        unit_size: UnitSize::ONE,
    }
}

/// A per-1 rate as the v2 wire carries it (RE-4d ruling 4: up to 11 places, not a `Rate`).
fn wire(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn money(text: &str) -> Money {
    Money::parse(text).unwrap()
}

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).unwrap()
}

/// The sentence and the status of a refusal; panics when the step was taken.
fn refused<T>(got: Result<T, BookingError>) -> (String, u16) {
    match got {
        Ok(_) => panic!("must be refused"),
        Err(e) => (e.to_string(), e.status()),
    }
}

fn taken<T>(got: Result<T, BookingError>) -> T {
    match got {
        Ok(v) => v,
        Err(e) => panic!("must be taken, was refused: {e}"),
    }
}

fn says(sentence: &str) -> (String, u16) {
    (sentence.to_owned(), 400)
}

/// A BUY of USD 100.00 at 32.1000 in MAIN, made an hour ago.
fn booking(
    status: BookingStatus,
    member: Option<&str>,
    expires_at: Option<DateTime<Utc>>,
) -> Booking {
    Booking {
        id: "BK-1".to_owned(),
        branch_id: BranchId::new("MAIN"),
        name: "Somchai Jaidee".to_owned(),
        phone: "081-234-5678".to_owned(),
        member_id: member.map(str::to_owned),
        side: Side::Buy,
        currency: code("USD"),
        amount: money("100.00"),
        rate: rate("32.1000"),
        unit_size: UnitSize::ONE,
        published_rate_line_id: Some(Uuid::from_u128(LINE_A)),
        status,
        created_at: now() - Duration::hours(1),
        created_by: "owner".to_owned(),
        expires_at,
        confirmed_at: None,
        confirmed_by: None,
        submission_id: None,
        transaction_ref: None,
    }
}

fn used_by(submission: &str, expires_at: DateTime<Utc>) -> Booking {
    Booking {
        submission_id: Some(submission.to_owned()),
        transaction_ref: Some("XC-20261008-000001".to_owned()),
        ..booking(BookingStatus::Used, None, Some(expires_at))
    }
}

fn member(id: &str, customer: &str) -> Member {
    Member {
        id: id.to_owned(),
        branch_id: BranchId::new("MAIN"),
        customer_id: customer.to_owned(),
        name: "Somchai Jaidee".to_owned(),
        phone: "081-234-5678".to_owned(),
        token_hash: "7c15e2bdbcfc71016f5042c55625e9ece4cf7db1a4124bc5670ef388f8fc3d2a".to_owned(),
        verified_at: now() - Duration::days(3),
        verified_by: "owner".to_owned(),
    }
}

/// A `create` for USD 100.00 BUY at the expected rate 32.1, for `customer` when given.
fn create_cmd(customer: Option<&str>) -> CreateBooking {
    CreateBooking {
        request_id: "REQ-1".to_owned(),
        name: "Somchai Jaidee".to_owned(),
        phone: "081-234-5678".to_owned(),
        side: Side::Buy,
        currency: code("USD"),
        amount: money("100.00"),
        expected_rate: wire("32.1"),
        customer_id: customer.map(str::to_owned),
    }
}

/// Everything `create` needs is in place: rules on for 30 minutes, a shift open, the counter
/// at the expected rate (line A), and no member.
fn facts() -> CreateFacts {
    CreateFacts {
        rules: BookingRules {
            enabled: true,
            minutes: 30,
        },
        open_shift: true,
        counter: Some(line(LINE_A, "32.1")),
        member_id: None,
    }
}

fn confirm_cmd(expires_at: Option<DateTime<Utc>>, expected: Option<&str>) -> ConfirmBooking {
    ConfirmBooking {
        id: "BK-1".to_owned(),
        expires_at,
        expected_rate: expected.map(wire),
    }
}

/// A LIVE save of exactly the booking of `booking(..)`, by customer C-1, ticked.
fn redeem_cmd() -> RedeemBooking {
    RedeemBooking {
        booking_id: "BK-1".to_owned(),
        submission_id: "S-1".to_owned(),
        live: true,
        side: Side::Buy,
        currency: code("USD"),
        // the same amount and rate written another way: compared as numbers, not text
        amount: money("100"),
        rate: rate("32.1"),
        form_booking: "BK-1".to_owned(),
        customer_id: "C-1".to_owned(),
        customer_confirmed: true,
    }
}

fn tomorrow() -> DateTime<Utc> {
    now() + Duration::days(1)
}

// ── create ───────────────────────────────────────────────────────────────────

#[test]
fn create_for_a_non_member_is_confirmed_for_the_rules_minutes() {
    let made = taken(BookingMachine::create(&create_cmd(None), &facts(), now()));
    assert_eq!(made.status, BookingStatus::Confirmed);
    assert_eq!(made.rate, rate("32.1"));
    assert_eq!(made.expires_at, Some(now() + Duration::minutes(30)));
    assert_eq!(made.member_id, None);

    let longer = CreateFacts {
        rules: BookingRules {
            enabled: true,
            minutes: 45,
        },
        ..facts()
    };
    let made = taken(BookingMachine::create(&create_cmd(None), &longer, now()));
    assert_eq!(made.expires_at, Some(now() + Duration::minutes(45)));

    let shortest = CreateFacts {
        rules: BookingRules {
            enabled: true,
            minutes: 5,
        },
        ..facts()
    };
    let made = taken(BookingMachine::create(&create_cmd(None), &shortest, now()));
    assert_eq!(made.expires_at, Some(now() + Duration::minutes(5)));
}

#[test]
fn member_booking_waits() {
    let with_member = CreateFacts {
        member_id: Some("MB-1".to_owned()),
        ..facts()
    };
    let made = taken(BookingMachine::create(
        &create_cmd(Some("C-1")),
        &with_member,
        now(),
    ));
    assert_eq!(made.status, BookingStatus::Pending);
    assert_eq!(made.member_id.as_deref(), Some("MB-1"));
    // RE-4d R6: the counter rate now, the price of the booking; confirm never re-locks it
    assert_eq!(made.rate, rate("32.1"));
    assert_eq!(made.expires_at, None);
}

#[test]
fn create_refusal_order() {
    let cmd = create_cmd(None);

    // 9 before 10 before 11: rules off, no shift open, and the rate moved
    let all_three = CreateFacts {
        rules: BookingRules {
            enabled: false,
            minutes: 30,
        },
        open_shift: false,
        counter: Some(line(LINE_B, "32.2")),
        member_id: None,
    };
    assert_eq!(
        refused(BookingMachine::create(&cmd, &all_three, now())),
        says(CLOSED)
    );

    // 10 before 11
    let no_shift_and_moved = CreateFacts {
        open_shift: false,
        counter: Some(line(LINE_B, "32.2")),
        ..facts()
    };
    assert_eq!(
        refused(BookingMachine::create(&cmd, &no_shift_and_moved, now())),
        says(NO_SHIFT)
    );

    // 11: the counter moved, or has no rate for the currency and side
    let moved = CreateFacts {
        counter: Some(line(LINE_B, "32.2")),
        ..facts()
    };
    assert_eq!(
        refused(BookingMachine::create(&cmd, &moved, now())),
        says(RATE_MOVED)
    );
    let no_rate = CreateFacts {
        counter: None,
        ..facts()
    };
    assert_eq!(
        refused(BookingMachine::create(&cmd, &no_rate, now())),
        says(RATE_MOVED)
    );

    // 8 before 9: a customer named who is no member of the branch, with the rules off too
    let closed = CreateFacts {
        rules: BookingRules {
            enabled: false,
            minutes: 30,
        },
        ..facts()
    };
    assert_eq!(
        refused(BookingMachine::create(
            &create_cmd(Some("C-1")),
            &closed,
            now()
        )),
        says(NO_MEMBER)
    );

    // a member's booking passes the same three checks
    let member_closed = CreateFacts {
        member_id: Some("MB-1".to_owned()),
        ..all_three
    };
    assert_eq!(
        refused(BookingMachine::create(
            &create_cmd(Some("C-1")),
            &member_closed,
            now()
        )),
        says(CLOSED)
    );
    let member_moved = CreateFacts {
        member_id: Some("MB-1".to_owned()),
        counter: Some(line(LINE_B, "32.2")),
        ..facts()
    };
    assert_eq!(
        refused(BookingMachine::create(
            &create_cmd(Some("C-1")),
            &member_moved,
            now()
        )),
        says(RATE_MOVED)
    );
}

#[test]
fn create_request_hash_follows_the_command() {
    // refusal 7 compares this hash: the same parsed command answers the stored booking,
    // another one is CHANGED (§5.6.3)
    let a = create_cmd(None).request_hash();
    assert_eq!(a, create_cmd(None).request_hash());
    assert!(!a.is_empty());

    let changes = [
        CreateBooking {
            amount: money("100.01"),
            ..create_cmd(None)
        },
        CreateBooking {
            expected_rate: wire("32.2"),
            ..create_cmd(None)
        },
        CreateBooking {
            side: Side::Sell,
            ..create_cmd(None)
        },
        CreateBooking {
            currency: code("EUR"),
            ..create_cmd(None)
        },
        CreateBooking {
            name: "Somchai J.".to_owned(),
            ..create_cmd(None)
        },
        CreateBooking {
            phone: "081-234-5679".to_owned(),
            ..create_cmd(None)
        },
        create_cmd(Some("C-1")),
    ];
    for (i, changed) in changes.iter().enumerate() {
        assert_ne!(changed.request_hash(), a, "change #{i}");
    }
}

// ── confirm ──────────────────────────────────────────────────────────────────

/// Amended for #758 (RE-4d, rates §4.9 R6): this was `confirm_sets_the_counter_rate_and_the_sent_expiry`
/// (made PENDING at 32.0, the counter now 32.1, confirm locked 32.1). R6: confirm never re-locks;
/// a counter at another rate is another line and refuses (`confirm_rate_moved`). On the line it
/// was made at, confirm takes the sent expiry and names no new rate.
#[test]
fn confirm_on_the_stored_line_sets_the_sent_expiry() {
    // made PENDING at 32.0 on line A; line A is still current and the screen expects 32.0
    let pending = Booking {
        rate: rate("32.0"),
        ..booking(BookingStatus::Pending, Some("MB-1"), None)
    };
    let step = taken(BookingMachine::confirm(
        &pending,
        &confirm_cmd(Some(tomorrow()), Some("32.0")),
        Some(line(LINE_A, "32.0000")),
        now(),
    ));
    match step {
        ConfirmStep::Confirm { expires_at, .. } => assert_eq!(expires_at, tomorrow()),
        ConfirmStep::Unchanged { .. } => panic!("a PENDING member booking must be confirmed"),
    }
}

#[test]
fn confirm_needs_a_future_expiry() {
    let pending = booking(BookingStatus::Pending, Some("MB-1"), None);
    for expires_at in [
        None, // unreadable
        Some(now()),
        Some(now() - Duration::milliseconds(1)),
        Some(now() - Duration::days(1)),
    ] {
        assert_eq!(
            refused(BookingMachine::confirm(
                &pending,
                &confirm_cmd(expires_at, Some("32.1")),
                Some(line(LINE_A, "32.1")),
                now(),
            )),
            says(EXPIRY_IN_FUTURE),
            "{expires_at:?}"
        );
    }
    assert!(matches!(
        taken(BookingMachine::confirm(
            &pending,
            &confirm_cmd(Some(now() + Duration::milliseconds(1)), Some("32.1")),
            Some(line(LINE_A, "32.1")),
            now(),
        )),
        ConfirmStep::Confirm { .. }
    ));

    // rule 3 comes before the double confirm of rule 4: a CONFIRMED booking at its own
    // instant, sent again with that same instant, is refused, not answered as unchanged
    let at_its_instant = booking(BookingStatus::Confirmed, Some("MB-1"), Some(now()));
    assert_eq!(
        refused(BookingMachine::confirm(
            &at_its_instant,
            &confirm_cmd(Some(now()), Some("32.1")),
            Some(line(LINE_A, "32.1")),
            now(),
        )),
        says(EXPIRY_IN_FUTURE)
    );
}

#[test]
fn double_confirm_same_expiry_is_unchanged() {
    let confirmed = booking(BookingStatus::Confirmed, Some("MB-1"), Some(tomorrow()));
    // no rate check: the counter moved since (line B), or has no line, or the screen sent none
    for (counter, expected) in [
        (Some(line(LINE_A, "32.1")), Some("32.1")),
        (Some(line(LINE_B, "33.0")), Some("32.1")),
        (None, Some("32.1")),
        (Some(line(LINE_A, "32.1")), None),
    ] {
        assert!(
            matches!(
                taken(BookingMachine::confirm(
                    &confirmed,
                    &confirm_cmd(Some(tomorrow()), expected),
                    counter,
                    now(),
                )),
                ConfirmStep::Unchanged { .. }
            ),
            "counter {counter:?}, expected {expected:?}"
        );
    }
    // the same to the millisecond: what v2 compares (`toISOString()`)
    assert!(matches!(
        taken(BookingMachine::confirm(
            &confirmed,
            &confirm_cmd(Some(tomorrow() + Duration::microseconds(400)), Some("32.1")),
            Some(line(LINE_A, "32.1")),
            now(),
        )),
        ConfirmStep::Unchanged { .. }
    ));
    // v2 asks only "CONFIRMED with this expiry": a non-member's booking is answered the same
    let not_a_member = booking(BookingStatus::Confirmed, None, Some(tomorrow()));
    assert!(matches!(
        taken(BookingMachine::confirm(
            &not_a_member,
            &confirm_cmd(Some(tomorrow()), Some("32.1")),
            Some(line(LINE_A, "32.1")),
            now(),
        )),
        ConfirmStep::Unchanged { .. }
    ));
}

#[test]
fn reconfirm_other_expiry_refused() {
    let confirmed = booking(BookingStatus::Confirmed, Some("MB-1"), Some(tomorrow()));
    for other in [
        tomorrow() + Duration::milliseconds(1),
        tomorrow() - Duration::milliseconds(1),
        tomorrow() + Duration::days(1),
    ] {
        assert_eq!(
            refused(BookingMachine::confirm(
                &confirmed,
                &confirm_cmd(Some(other), Some("32.1")),
                Some(line(LINE_A, "32.1")),
                now(),
            )),
            says(ONLY_PENDING_MEMBER),
            "{other}"
        );
    }
}

#[test]
fn only_a_pending_member_booking_is_confirmed() {
    let expires = Some(tomorrow());
    for (label, b) in [
        (
            "cancelled",
            booking(BookingStatus::Cancelled, Some("MB-1"), None),
        ),
        ("used", used_by("S-9", tomorrow())),
        (
            "a non-member's confirmed booking, another expiry",
            booking(
                BookingStatus::Confirmed,
                None,
                Some(now() + Duration::hours(2)),
            ),
        ),
        // the table's CHECK forbids it (T15-b); the machine refuses it on its own
        (
            "pending without a member",
            booking(BookingStatus::Pending, None, None),
        ),
    ] {
        assert_eq!(
            refused(BookingMachine::confirm(
                &b,
                &confirm_cmd(expires, Some("32.1")),
                Some(line(LINE_A, "32.1")),
                now(),
            )),
            says(ONLY_PENDING_MEMBER),
            "{label}"
        );
    }
}

/// Amended for #758 (RE-4d, rates §4.9 R4/R6): the counter is the current line. A moved counter is
/// line B (at another rate, or at the very same rate); no line; or the screen sent no rate.
#[test]
fn confirm_rate_moved() {
    let pending = booking(BookingStatus::Pending, Some("MB-1"), None);
    for (counter, expected) in [
        (Some(line(LINE_B, "32.2")), Some("32.1")),
        (Some(line(LINE_B, "32.0999")), Some("32.1")),
        (Some(line(LINE_B, "32.1")), Some("32.1")),
        (None, Some("32.1")),
        (Some(line(LINE_A, "32.1")), None),
    ] {
        assert_eq!(
            refused(BookingMachine::confirm(
                &pending,
                &confirm_cmd(Some(tomorrow()), expected),
                counter,
                now(),
            )),
            says(RATE_MOVED),
            "counter {counter:?}, expected {expected:?}"
        );
    }
    // rule 5 comes before rule 6: a cancelled booking with the rate moved is not "rate moved"
    let cancelled = booking(BookingStatus::Cancelled, Some("MB-1"), None);
    assert_eq!(
        refused(BookingMachine::confirm(
            &cancelled,
            &confirm_cmd(Some(tomorrow()), Some("32.1")),
            Some(line(LINE_B, "33.0")),
            now(),
        )),
        says(ONLY_PENDING_MEMBER)
    );
}

// ── cancel ───────────────────────────────────────────────────────────────────

#[test]
fn pending_and_confirmed_are_cancelled() {
    for (label, b) in [
        (
            "pending",
            booking(BookingStatus::Pending, Some("MB-1"), None),
        ),
        (
            "confirmed",
            booking(BookingStatus::Confirmed, None, Some(tomorrow())),
        ),
        (
            "confirmed, shown expired",
            booking(
                BookingStatus::Confirmed,
                None,
                Some(now() - Duration::hours(1)),
            ),
        ),
    ] {
        assert!(
            matches!(taken(BookingMachine::cancel(&b)), CancelStep::Cancel { .. }),
            "{label}"
        );
    }
}

#[test]
fn cancel_twice_is_unchanged() {
    let cancelled = booking(BookingStatus::Cancelled, None, Some(tomorrow()));
    assert!(matches!(
        taken(BookingMachine::cancel(&cancelled)),
        CancelStep::Unchanged { .. }
    ));
}

#[test]
fn cancel_used_refused() {
    assert_eq!(
        refused(BookingMachine::cancel(&used_by("S-1", tomorrow()))),
        says(ALREADY_USED)
    );
}

// ── the shown status ─────────────────────────────────────────────────────────

#[test]
fn the_shown_status_is_expired_only_for_a_confirmed_booking_past_its_expiry() {
    let past = Some(now() - Duration::minutes(1));
    assert_eq!(
        booking(BookingStatus::Confirmed, None, past).shown_status(now()),
        "EXPIRED"
    );
    assert_eq!(
        booking(BookingStatus::Confirmed, None, Some(tomorrow())).shown_status(now()),
        "CONFIRMED"
    );
    assert_eq!(
        booking(BookingStatus::Pending, Some("MB-1"), None).shown_status(now()),
        "PENDING"
    );
    assert_eq!(
        booking(BookingStatus::Cancelled, None, past).shown_status(now()),
        "CANCELLED"
    );
    assert_eq!(
        used_by("S-1", now() - Duration::days(1)).shown_status(now()),
        "USED"
    );

    assert!(booking(BookingStatus::Pending, Some("MB-1"), None).is_member());
    assert!(!booking(BookingStatus::Confirmed, None, Some(tomorrow())).is_member());
}

#[test]
fn expiry_is_inclusive_of_the_instant() {
    // §5.6.8: a CONFIRMED booking at exactly `expiresAt`
    let at = now();
    let b = booking(BookingStatus::Confirmed, None, Some(at));

    // shown EXPIRED at the instant, CONFIRMED a millisecond before
    assert_eq!(b.shown_status(at), "EXPIRED");
    assert_eq!(b.shown_status(at - Duration::milliseconds(1)), "CONFIRMED");

    // cancel works
    assert!(matches!(
        taken(BookingMachine::cancel(&b)),
        CancelStep::Cancel { .. }
    ));

    // confirm (with a new, future expiry) refuses rule 5
    assert_eq!(
        refused(BookingMachine::confirm(
            &b,
            &confirm_cmd(Some(tomorrow()), Some("32.1")),
            Some(line(LINE_A, "32.1")),
            at,
        )),
        says(ONLY_PENDING_MEMBER)
    );

    // a save refuses row 3 at the instant, and is let through a millisecond before
    assert_eq!(
        refused(BookingMachine::redeem(&b, None, &redeem_cmd(), at)),
        says(NOT_USABLE)
    );
    assert!(matches!(
        taken(BookingMachine::redeem(
            &b,
            None,
            &redeem_cmd(),
            at - Duration::milliseconds(1)
        )),
        RedeemStep::Use { .. }
    ));
}

// ── redeem: using a booking at the desk ──────────────────────────────────────

#[test]
fn redeem_a_confirmed_booking_is_used() {
    let b = booking(
        BookingStatus::Confirmed,
        None,
        Some(now() + Duration::minutes(30)),
    );
    assert!(matches!(
        taken(BookingMachine::redeem(&b, None, &redeem_cmd(), now())),
        RedeemStep::Use { .. }
    ));
}

#[test]
fn redeem_used_by_another_submission_refused() {
    assert_eq!(
        refused(BookingMachine::redeem(
            &used_by("S-2", tomorrow()),
            None,
            &redeem_cmd(),
            now()
        )),
        says(USED_BY_ANOTHER)
    );
}

#[test]
fn redeem_repeat_by_the_same_submission() {
    // USED by this submission is a repeat and passes row 2 and row 3, even once expired
    for expires in [tomorrow(), now() - Duration::hours(1)] {
        assert!(
            matches!(
                taken(BookingMachine::redeem(
                    &used_by("S-1", expires),
                    None,
                    &redeem_cmd(),
                    now()
                )),
                RedeemStep::Repeat { .. }
            ),
            "{expires}"
        );
    }
}

#[test]
fn redeem_needs_a_confirmed_unexpired_booking() {
    for (label, b) in [
        (
            "pending",
            booking(BookingStatus::Pending, Some("MB-1"), None),
        ),
        (
            "cancelled",
            booking(BookingStatus::Cancelled, None, Some(tomorrow())),
        ),
        (
            "expired",
            booking(
                BookingStatus::Confirmed,
                None,
                Some(now() - Duration::seconds(1)),
            ),
        ),
    ] {
        assert_eq!(
            refused(BookingMachine::redeem(&b, None, &redeem_cmd(), now())),
            says(NOT_USABLE),
            "{label}"
        );
    }
    // row 3 comes before row 4: an expired booking with a form that does not match it
    let expired = booking(
        BookingStatus::Confirmed,
        None,
        Some(now() - Duration::seconds(1)),
    );
    let other_form = RedeemBooking {
        amount: money("999"),
        ..redeem_cmd()
    };
    assert_eq!(
        refused(BookingMachine::redeem(&expired, None, &other_form, now())),
        says(NOT_USABLE)
    );
}

#[test]
fn redeem_form_must_match_the_booking() {
    let b = booking(BookingStatus::Confirmed, None, Some(tomorrow()));
    let cases = [
        (
            "DEMO",
            RedeemBooking {
                live: false,
                ..redeem_cmd()
            },
        ),
        (
            "SELL",
            RedeemBooking {
                side: Side::Sell,
                ..redeem_cmd()
            },
        ),
        (
            "EUR",
            RedeemBooking {
                currency: code("EUR"),
                ..redeem_cmd()
            },
        ),
        (
            "amount",
            RedeemBooking {
                amount: money("100.01"),
                ..redeem_cmd()
            },
        ),
        (
            "rate",
            RedeemBooking {
                rate: rate("32.1001"),
                ..redeem_cmd()
            },
        ),
        (
            "the form's booking field",
            RedeemBooking {
                form_booking: "BK-2".to_owned(),
                ..redeem_cmd()
            },
        ),
        (
            "an empty booking field",
            RedeemBooking {
                form_booking: String::new(),
                ..redeem_cmd()
            },
        ),
        (
            "the customer not ticked",
            RedeemBooking {
                customer_confirmed: false,
                ..redeem_cmd()
            },
        ),
    ];
    for (label, cmd) in cases {
        assert_eq!(
            refused(BookingMachine::redeem(&b, None, &cmd, now())),
            says(FORM_MISMATCH),
            "{label}"
        );
    }
}

#[test]
fn redeem_member_customer_must_match() {
    let b = booking(BookingStatus::Confirmed, Some("MB-1"), Some(tomorrow()));
    let m = member("MB-1", "C-1");

    assert!(matches!(
        taken(BookingMachine::redeem(&b, Some(&m), &redeem_cmd(), now())),
        RedeemStep::Use { .. }
    ));

    let other_customer = RedeemBooking {
        customer_id: "C-2".to_owned(),
        ..redeem_cmd()
    };
    assert_eq!(
        refused(BookingMachine::redeem(&b, Some(&m), &other_customer, now())),
        says(MEMBER_MISMATCH)
    );
    // the member row is gone: v2 compares `undefined` with the customer and refuses
    assert_eq!(
        refused(BookingMachine::redeem(&b, None, &redeem_cmd(), now())),
        says(MEMBER_MISMATCH)
    );

    // row 4 comes before row 5
    let other_customer_unticked = RedeemBooking {
        customer_confirmed: false,
        ..other_customer
    };
    assert_eq!(
        refused(BookingMachine::redeem(
            &b,
            Some(&m),
            &other_customer_unticked,
            now()
        )),
        says(FORM_MISMATCH)
    );
}
