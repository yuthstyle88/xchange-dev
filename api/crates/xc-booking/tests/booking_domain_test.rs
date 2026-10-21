//! BK-3 #1380 — pure domain unit tests (design §4.1, §4.2, §4.3, §5, §7).
//!
//! Tests for pure value objects and state machine transitions without a database.

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_booking::domain::booking::booking::Booking;
use xc_booking::domain::booking::booking_error::BookingError;
use xc_booking::domain::booking::booking_machine::BookingMachine;
use xc_booking::domain::booking::booking_no::BookingNo;
use xc_booking::domain::booking::booking_status::BookingStatus;
use xc_booking::domain::booking::qr_token::QrToken;
use xc_booking::domain::booking::source_channel::SourceChannel;
use xc_booking::domain::policy::booking_policy::BookingPolicy;
use xc_booking::domain::slot::booking_slot::{BookingSlot, SlotStatus};
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;

fn dummy_slot(start: DateTime<Utc>) -> BookingSlot {
    BookingSlot {
        id: Uuid::new_v4(),
        organization_id: Uuid::nil(),
        branch_id: BranchId::new("MAIN"),
        service_date: NaiveDate::from_ymd_opt(2030, 1, 7).expect("a date"),
        start_at: start,
        end_at: start + Duration::minutes(30),
        capacity: 2,
        reserved: 1,
        status: SlotStatus::Open,
    }
}

fn dummy_booking(status: BookingStatus, expires_at: Option<DateTime<Utc>>) -> Booking {
    Booking {
        id: format!("BK-{}", Uuid::new_v4()),
        organization_id: Uuid::nil(),
        branch_id: BranchId::new("MAIN"),
        booking_no: "B300107-0001".to_string(),
        customer_id: "CX-00000001".to_string(),
        slot_id: Uuid::new_v4(),
        quote_id: None,
        shop_side: Side::Buy,
        requested_currency: "USD".to_string(),
        requested_quantity: Decimal::new(100, 0),
        source_channel: SourceChannel::App,
        status,
        qr_token_hash: vec![0u8; 32],
        qr_version: 1,
        confirm_expires_at: expires_at,
        checked_in_at: None,
        transaction_id: None,
        partner_attribution_id: None,
        ended_reason: None,
        created_by: "test".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn holding_states_hold_place_others_do_not() {
    assert!(BookingStatus::PendingCustomerConfirmation.holds_place());
    assert!(BookingStatus::Confirmed.holds_place());
    assert!(BookingStatus::CheckedIn.holds_place());
    assert!(BookingStatus::InService.holds_place());
    assert!(BookingStatus::Completed.holds_place());

    assert!(!BookingStatus::Cancelled.holds_place());
    assert!(!BookingStatus::NoShow.holds_place());
    assert!(!BookingStatus::Expired.holds_place());
}

#[test]
fn source_channels_start_in_specified_initial_state() {
    assert_eq!(SourceChannel::App.start_status(), BookingStatus::Confirmed);
    assert_eq!(SourceChannel::Desk.start_status(), BookingStatus::Confirmed);
    assert_eq!(
        SourceChannel::Partner.start_status(),
        BookingStatus::PendingCustomerConfirmation
    );
}

#[test]
fn booking_no_formats_date_and_sequence() {
    let date = NaiveDate::from_ymd_opt(2026, 10, 12).expect("valid date");
    assert_eq!(BookingNo::format(date, 1), "B261012-0001");
    assert_eq!(BookingNo::format(date, 42), "B261012-0042");
    assert_eq!(BookingNo::format(date, 9999), "B261012-9999");
}

#[test]
fn qr_token_derives_deterministic_token_and_hashes_it() {
    // Design §2 B7: token = base64url(HMAC-SHA256(qr_key, booking_id.as_bytes() || qr_version.to_be_bytes()))
    // Fixed vector computed with python3:
    // key = b"01234567890123456789012345678901"
    // booking_id = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap()
    // qr_version = 1
    // msg = booking_id.as_bytes() || qr_version.to_be_bytes() (20 bytes: 16 bytes UUID + 4 bytes big-endian)
    // token = "WfANZeMV-_4bEOEZ8s1hf1rAtGMqTBWOHYSXzOannog"
    // sha256(token) hex = "77a93c8d6383b91baaacddab2be6d6a1371099979f2018f37cf217e3d1410d62"
    let key = b"01234567890123456789012345678901";
    let booking_id = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();

    let token1 = QrToken::derive(key, &booking_id, 1);
    let token2 = QrToken::derive(key, &booking_id, 1);
    assert_eq!(token1, token2);
    assert_eq!(
        token1, "WfANZeMV-_4bEOEZ8s1hf1rAtGMqTBWOHYSXzOannog",
        "pinned against independent Python HMAC-SHA256 vector"
    );

    let token_v2 = QrToken::derive(key, &booking_id, 2);
    assert_ne!(token1, token_v2);
    assert_eq!(
        token_v2, "h0kejeMSDRk7i3Qpz9RhbIRMCwN0rftCNDjJiBTB8Vg",
        "pinned against independent Python vector for version 2"
    );

    let other_key = b"abcdefghijklmnopqrstuvwxyz012345";
    let token_other_key = QrToken::derive(other_key, &booking_id, 1);
    assert_ne!(token1, token_other_key);
    assert_eq!(
        token_other_key, "wF1d4TDuz-XpB4Y_Yt3eZvGyivtVlNk4XGEFdhSBQFs",
        "another key gives another token"
    );

    let hash1 = QrToken::hash(&token1);
    let hash2 = QrToken::hash(&token1);
    assert_eq!(hash1, hash2);
    assert_eq!(hash1.len(), 32);

    let mut hash_hex = String::with_capacity(64);
    for b in &hash1 {
        use std::fmt::Write;
        let _ = write!(&mut hash_hex, "{:02x}", b);
    }
    assert_eq!(
        hash_hex, "77a93c8d6383b91baaacddab2be6d6a1371099979f2018f37cf217e3d1410d62",
        "hash is exact SHA-256 of token"
    );
}

#[test]
fn confirm_before_expiry_transitions_to_confirmed() {
    let now = Utc.with_ymd_and_hms(2030, 1, 7, 2, 0, 0).unwrap();
    let expiry = now + Duration::minutes(30);
    let booking = dummy_booking(BookingStatus::PendingCustomerConfirmation, Some(expiry));
    let policy = BookingPolicy {
        cancel_cutoff_minutes: 60,
        no_show_grace_minutes: 15,
        confirm_expiry_minutes: 60,
    };

    let result = BookingMachine::confirm(&booking, now, &policy);
    assert_eq!(result.unwrap(), BookingStatus::Confirmed);
}

#[test]
fn confirm_after_expiry_refuses_with_booking_expired() {
    let now = Utc.with_ymd_and_hms(2030, 1, 7, 2, 31, 0).unwrap();
    let expiry = Utc.with_ymd_and_hms(2030, 1, 7, 2, 30, 0).unwrap();
    let booking = dummy_booking(BookingStatus::PendingCustomerConfirmation, Some(expiry));
    let policy = BookingPolicy {
        cancel_cutoff_minutes: 60,
        no_show_grace_minutes: 15,
        confirm_expiry_minutes: 60,
    };

    let result = BookingMachine::confirm(&booking, now, &policy);
    assert!(matches!(result, Err(BookingError::BookingExpired)));
}

#[test]
fn customer_cancel_before_cutoff_succeeds() {
    let slot_start = Utc.with_ymd_and_hms(2030, 1, 7, 10, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2030, 1, 7, 8, 30, 0).unwrap(); // 90 min before, cutoff is 60 min
    let slot = dummy_slot(slot_start);
    let booking = dummy_booking(BookingStatus::Confirmed, None);
    let policy = BookingPolicy {
        cancel_cutoff_minutes: 60,
        no_show_grace_minutes: 15,
        confirm_expiry_minutes: 60,
    };

    let result =
        BookingMachine::cancel(&booking, "customer:CX-1", false, None, now, &policy, &slot);
    assert_eq!(result.unwrap(), BookingStatus::Cancelled);
}

#[test]
fn customer_cancel_exactly_at_cutoff_boundary_succeeds() {
    let slot_start = Utc.with_ymd_and_hms(2030, 1, 7, 10, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2030, 1, 7, 9, 0, 0).unwrap(); // exactly 60 min before slot start
    let slot = dummy_slot(slot_start);
    let booking = dummy_booking(BookingStatus::Confirmed, None);
    let policy = BookingPolicy {
        cancel_cutoff_minutes: 60,
        no_show_grace_minutes: 15,
        confirm_expiry_minutes: 60,
    };

    let result =
        BookingMachine::cancel(&booking, "customer:CX-1", false, None, now, &policy, &slot);
    assert_eq!(result.unwrap(), BookingStatus::Cancelled);
}

/// #4203 row 1, restated by the ticket author (claude/studio-claude-6) on the owner's ruling of 2026-10-21:
/// "ยกเลิกการจอง ยกเลิกได้เลย" · "ไม่ต้อง มี การอนุมัติ เพราะไม่มีอะไรเกี่ยวกับเงิน". A customer may cancel at any time; the cutoff limits reschedule only (design `booking-slots.md` Q-B2,
/// revised). Before #4203 this test asserted `CancelCutoffPassed`.
#[test]
fn customer_cancel_after_cutoff_succeeds() {
    let slot_start = Utc.with_ymd_and_hms(2030, 1, 7, 10, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2030, 1, 7, 9, 50, 0).unwrap(); // 10 min before slot start, cutoff is 60 min
    let slot = dummy_slot(slot_start);
    let booking = dummy_booking(BookingStatus::Confirmed, None);
    let policy = BookingPolicy {
        cancel_cutoff_minutes: 60,
        no_show_grace_minutes: 15,
        confirm_expiry_minutes: 60,
    };

    let result =
        BookingMachine::cancel(&booking, "customer:CX-1", false, None, now, &policy, &slot);
    assert_eq!(result.unwrap(), BookingStatus::Cancelled);
}

/// #4203 row 2 (owner, 2026-10-21): a CONFIRMED booking not yet checked in can be cancelled by its customer even
/// after its slot has started.
#[test]
fn customer_cancel_after_the_slot_started_succeeds() {
    let slot_start = Utc.with_ymd_and_hms(2030, 1, 7, 10, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2030, 1, 7, 10, 10, 0).unwrap(); // 10 min after slot start
    let slot = dummy_slot(slot_start);
    let booking = dummy_booking(BookingStatus::Confirmed, None);
    let policy = BookingPolicy {
        cancel_cutoff_minutes: 60,
        no_show_grace_minutes: 15,
        confirm_expiry_minutes: 60,
    };

    let result =
        BookingMachine::cancel(&booking, "customer:CX-1", false, None, now, &policy, &slot);
    assert_eq!(result.unwrap(), BookingStatus::Cancelled);
}

/// #4203 row 3 (unchanged rule): a customer still cannot cancel a CHECKED_IN booking; only staff can, with a reason.
#[test]
fn customer_cancel_of_a_checked_in_booking_is_refused_stale_version() {
    let slot_start = Utc.with_ymd_and_hms(2030, 1, 7, 10, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2030, 1, 7, 8, 0, 0).unwrap();
    let slot = dummy_slot(slot_start);
    let booking = dummy_booking(BookingStatus::CheckedIn, None);
    let policy = BookingPolicy {
        cancel_cutoff_minutes: 60,
        no_show_grace_minutes: 15,
        confirm_expiry_minutes: 60,
    };

    let result =
        BookingMachine::cancel(&booking, "customer:CX-1", false, None, now, &policy, &slot);
    assert!(matches!(result, Err(BookingError::StaleVersion)));
}

#[test]
fn teller_cancel_without_reason_is_refused() {
    let slot_start = Utc.with_ymd_and_hms(2030, 1, 7, 10, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2030, 1, 7, 8, 0, 0).unwrap();
    let slot = dummy_slot(slot_start);
    let booking = dummy_booking(BookingStatus::Confirmed, None);
    let policy = BookingPolicy {
        cancel_cutoff_minutes: 60,
        no_show_grace_minutes: 15,
        confirm_expiry_minutes: 60,
    };

    let result = BookingMachine::cancel(&booking, "teller:user-1", true, None, now, &policy, &slot);
    assert!(matches!(
        result,
        Err(BookingError::TellerCancelReasonRequired)
    ));
}

#[test]
fn teller_cancel_with_reason_succeeds_even_after_cutoff() {
    let slot_start = Utc.with_ymd_and_hms(2030, 1, 7, 10, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2030, 1, 7, 9, 45, 0).unwrap(); // 15 min before slot
    let slot = dummy_slot(slot_start);
    let booking = dummy_booking(BookingStatus::Confirmed, None);
    let policy = BookingPolicy {
        cancel_cutoff_minutes: 60,
        no_show_grace_minutes: 15,
        confirm_expiry_minutes: 60,
    };

    let result = BookingMachine::cancel(
        &booking,
        "teller:user-1",
        true,
        Some("Customer called on phone"),
        now,
        &policy,
        &slot,
    );
    assert_eq!(result.unwrap(), BookingStatus::Cancelled);
}

#[test]
fn reschedule_to_same_slot_is_unchanged() {
    let slot_start = Utc.with_ymd_and_hms(2030, 1, 7, 10, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2030, 1, 7, 8, 0, 0).unwrap();
    let slot = dummy_slot(slot_start);
    let booking = dummy_booking(BookingStatus::Confirmed, None);
    let policy = BookingPolicy {
        cancel_cutoff_minutes: 60,
        no_show_grace_minutes: 15,
        confirm_expiry_minutes: 60,
    };

    let result =
        BookingMachine::reschedule(&booking, "customer:CX-1", false, &slot, now, &policy, &slot);
    assert_eq!(result.unwrap(), BookingStatus::Confirmed);
}

#[test]
fn customer_reschedule_after_cutoff_is_refused() {
    let slot_start = Utc.with_ymd_and_hms(2030, 1, 7, 10, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2030, 1, 7, 9, 30, 0).unwrap(); // 30 min before
    let current_slot = dummy_slot(slot_start);
    let new_slot = dummy_slot(slot_start + Duration::hours(2));
    let booking = dummy_booking(BookingStatus::Confirmed, None);
    let policy = BookingPolicy {
        cancel_cutoff_minutes: 60,
        no_show_grace_minutes: 15,
        confirm_expiry_minutes: 60,
    };

    let result = BookingMachine::reschedule(
        &booking,
        "customer:CX-1",
        false,
        &new_slot,
        now,
        &policy,
        &current_slot,
    );
    assert!(matches!(result, Err(BookingError::CancelCutoffPassed)));
}
