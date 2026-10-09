//! Tests for repeat booking redemptions running row 4 (form check) and row 5 (member check).
//!
//! Amended for #758 (RE-4d) by the ticket author (claude/e5ab6c), `docs/design/blueprint/rates.md`
//! §4.9 R1: a booking holds its `unit_size` and the line it was created at. The booking here is
//! USD, unit 1, created before RE-4 (no line), so the rate reads as before.

use chrono::{DateTime, Duration, TimeZone, Utc};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side, UnitSize};

use xc_exchange::domain::booking::commands::redeem_booking::RedeemBooking;
use xc_exchange::domain::booking::entities::booking::Booking;
use xc_exchange::domain::booking::entities::booking_machine::BookingMachine;
use xc_exchange::domain::booking::entities::member::Member;
use xc_exchange::domain::booking::value_objects::booking_error::BookingError;
use xc_exchange::domain::booking::value_objects::booking_status::BookingStatus;

const FORM_MISMATCH: &str = "ข้อมูลรายการไม่ตรงกับการจอง หรือยังไม่ยืนยันลูกค้า";
const MEMBER_MISMATCH: &str = "ลูกค้าไม่ตรงกับ Member ที่จอง";

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 8, 3, 0, 0).unwrap()
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).unwrap()
}

fn money(text: &str) -> Money {
    Money::parse(text).unwrap()
}

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).unwrap()
}

fn refused<T>(got: Result<T, BookingError>) -> (String, u16) {
    match got {
        Ok(_) => panic!("must be refused"),
        Err(e) => (e.to_string(), e.status()),
    }
}

fn says(sentence: &str) -> (String, u16) {
    (sentence.to_owned(), 400)
}

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
        published_rate_line_id: None,
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

fn used_by(submission: &str, member: Option<&str>, expires_at: DateTime<Utc>) -> Booking {
    Booking {
        submission_id: Some(submission.to_owned()),
        transaction_ref: Some("XC-20261008-000001".to_owned()),
        ..booking(BookingStatus::Used, member, Some(expires_at))
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

fn redeem_cmd() -> RedeemBooking {
    RedeemBooking {
        booking_id: "BK-1".to_owned(),
        submission_id: "S-1".to_owned(),
        live: true,
        side: Side::Buy,
        currency: code("USD"),
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

#[test]
fn repeat_with_mismatched_form_is_refused() {
    let b = used_by("S-1", None, tomorrow());
    let cmd = RedeemBooking {
        live: false,
        ..redeem_cmd()
    };
    assert_eq!(
        refused(BookingMachine::redeem(&b, None, &cmd, now())),
        says(FORM_MISMATCH)
    );
}

#[test]
fn repeat_of_member_booking_with_other_customer_is_refused() {
    let b = used_by("S-1", Some("MB-1"), tomorrow());
    let m = member("MB-1", "C-1");
    let cmd = RedeemBooking {
        customer_id: "C-OTHER".to_owned(),
        ..redeem_cmd()
    };
    assert_eq!(
        refused(BookingMachine::redeem(&b, Some(&m), &cmd, now())),
        says(MEMBER_MISMATCH)
    );
}
