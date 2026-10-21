use chrono::{DateTime, TimeZone, Utc};
use serde_json::json;
use uuid::Uuid;
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel, QuoteDisplayStatus, QuoteStatus};
use xc_exchange::domain::reversal::reversal::ReversalStatus;
use xc_exchange::domain::transaction::transaction::TransactionStatus;
use xc_kernel::ids::BranchId;

fn at(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 21, 12, 0, second).unwrap()
}

fn pub_a() -> Uuid {
    Uuid::from_u128(1)
}

fn pub_b() -> Uuid {
    Uuid::from_u128(2)
}

fn quote(status: QuoteStatus, expires_at: DateTime<Utc>, publication: Uuid) -> Quote {
    let created = at(0);
    Quote {
        id: Uuid::from_u128(10),
        organization_id: Uuid::from_u128(20),
        branch_id: BranchId::new("MAIN"),
        customer_id: None,
        quote_type: "INDICATIVE".to_string(),
        channel: QuoteChannel::Desk,
        status,
        publication_id: publication,
        expires_at,
        benefit_snapshot: json!({}),
        created_by: "teller-1".to_string(),
        accepted_by_transaction_id: None,
        created_at: created,
        updated_at: created,
        lines: vec![],
        stale: false,
    }
}

#[test]
fn row_1_active_quote_one_second_before_expiry_same_publication_is_valid_and_active() {
    let expires_at = at(30);
    let now = at(29);
    let q = quote(QuoteStatus::Active, expires_at, pub_a());

    assert!(q.is_valid(now, pub_a()));
    assert_eq!(q.status_at(now, pub_a()), QuoteDisplayStatus::Active);
}

#[test]
fn row_2_now_equals_expires_at_exactly_same_publication_is_invalid_and_expired() {
    let expires_at = at(30);
    let now = at(30);
    let q = quote(QuoteStatus::Active, expires_at, pub_a());

    assert!(!q.is_valid(now, pub_a()));
    assert_eq!(q.status_at(now, pub_a()), QuoteDisplayStatus::Expired);
}

#[test]
fn row_3_now_one_second_after_expires_at_is_expired() {
    let expires_at = at(30);
    let now = at(31);
    let q = quote(QuoteStatus::Active, expires_at, pub_a());

    assert!(!q.is_valid(now, pub_a()));
    assert_eq!(q.status_at(now, pub_a()), QuoteDisplayStatus::Expired);
}

#[test]
fn row_4_before_expiry_but_current_publication_is_another_uuid_is_invalid_and_expired() {
    let expires_at = at(30);
    let now = at(29);
    let q = quote(QuoteStatus::Active, expires_at, pub_a());

    assert!(!q.is_valid(now, pub_b()));
    assert_eq!(q.status_at(now, pub_b()), QuoteDisplayStatus::Expired);
}

#[test]
fn row_5_accepted_quote_after_expiry_and_another_publication_stays_accepted() {
    let expires_at = at(30);
    let now = at(31);
    let q = quote(QuoteStatus::Accepted, expires_at, pub_a());

    assert_eq!(q.status_at(now, pub_b()), QuoteDisplayStatus::Accepted);
}

#[test]
fn row_6_cancelled_quote_before_expiry_same_publication_stays_cancelled() {
    let expires_at = at(30);
    let now = at(29);
    let q = quote(QuoteStatus::Cancelled, expires_at, pub_a());

    assert_eq!(q.status_at(now, pub_a()), QuoteDisplayStatus::Cancelled);
}

#[test]
fn row_7_quote_status_round_trip_and_unknown_inputs() {
    for (s, code) in [
        (QuoteStatus::Active, "ACTIVE"),
        (QuoteStatus::Accepted, "ACCEPTED"),
        (QuoteStatus::Cancelled, "CANCELLED"),
    ] {
        assert_eq!(s.as_str(), code);
        assert_eq!(QuoteStatus::parse_db(s.as_str()), Some(s));
    }
    assert_eq!(QuoteStatus::parse_db("active"), None);
    assert_eq!(QuoteStatus::parse_db(""), None);
}

#[test]
fn row_8_quote_channel_round_trip_and_case_sensitive_rejection() {
    for (ch, code) in [
        (QuoteChannel::Desk, "DESK"),
        (QuoteChannel::Kiosk, "KIOSK"),
        (QuoteChannel::Booking, "BOOKING"),
        (QuoteChannel::App, "APP"),
    ] {
        assert_eq!(ch.as_str(), code);
        assert_eq!(QuoteChannel::parse_db(ch.as_str()), Some(ch));
    }
    assert_eq!(QuoteChannel::parse_db("Desk"), None);
}

#[test]
fn row_9_quote_display_status_as_str_exact_codes() {
    assert_eq!(QuoteDisplayStatus::Active.as_str(), "ACTIVE");
    assert_eq!(QuoteDisplayStatus::Accepted.as_str(), "ACCEPTED");
    assert_eq!(QuoteDisplayStatus::Cancelled.as_str(), "CANCELLED");
    assert_eq!(QuoteDisplayStatus::Expired.as_str(), "EXPIRED");
}

#[test]
fn row_10_transaction_status_round_trip_and_unknown_inputs() {
    for (s, code) in [
        (TransactionStatus::Draft, "DRAFT"),
        (TransactionStatus::PendingApproval, "PENDING_APPROVAL"),
        (TransactionStatus::Confirmed, "CONFIRMED"),
        (TransactionStatus::Completed, "COMPLETED"),
        (TransactionStatus::Cancelled, "CANCELLED"),
    ] {
        assert_eq!(s.as_str(), code);
        assert_eq!(TransactionStatus::parse(s.as_str()), Some(s));
    }
    assert_eq!(TransactionStatus::parse("draft"), None);
    assert_eq!(TransactionStatus::parse(""), None);
    assert_eq!(TransactionStatus::parse("UNKNOWN"), None);
}

#[test]
fn row_11_reversal_status_round_trip_and_unknown_inputs() {
    for (s, code) in [
        (ReversalStatus::Requested, "REQUESTED"),
        (ReversalStatus::Approved, "APPROVED"),
        (ReversalStatus::Posted, "POSTED"),
        (ReversalStatus::Rejected, "REJECTED"),
    ] {
        assert_eq!(s.as_str(), code);
        assert_eq!(ReversalStatus::parse(s.as_str()), Some(s));
    }
    assert_eq!(ReversalStatus::parse("requested"), None);
    assert_eq!(ReversalStatus::parse(""), None);
    assert_eq!(ReversalStatus::parse("UNKNOWN"), None);
}
