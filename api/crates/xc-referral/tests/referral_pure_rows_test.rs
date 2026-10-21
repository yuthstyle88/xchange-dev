//! Pure tests for `CaptureStatus`, `CaptureSource`, and `FeedValue` (ticket #4035, round 3 item A5).

use std::str::FromStr;

use rust_decimal::Decimal;
use xc_referral::domain::capture::{CaptureSource, CaptureStatus};
use xc_referral::domain::feed_value::FeedValue;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("valid decimal")
}

// Row 1: every CaptureStatus variant: parse(v.as_str()) == Some(v)
// Mutation it must catch: two arms swapped
#[test]
fn row_1_capture_status_variants_roundtrip() {
    assert_eq!(
        CaptureStatus::parse(CaptureStatus::Active.as_str()),
        Some(CaptureStatus::Active)
    );
    assert_eq!(
        CaptureStatus::parse(CaptureStatus::Superseded.as_str()),
        Some(CaptureStatus::Superseded)
    );
}

// Row 2: every CaptureSource variant (including "APP"): parse(v.as_str()) == Some(v)
// Mutation it must catch: two arms swapped
#[test]
fn row_2_capture_source_variants_roundtrip() {
    assert_eq!(
        CaptureSource::parse(CaptureSource::App.as_str()),
        Some(CaptureSource::App)
    );
    assert_eq!(
        CaptureSource::parse(CaptureSource::Booking.as_str()),
        Some(CaptureSource::Booking)
    );
    assert_eq!(
        CaptureSource::parse(CaptureSource::Desk.as_str()),
        Some(CaptureSource::Desk)
    );
}

// Row 3: parse("active"), parse("") for each -> None
// Mutation it must catch: case-insensitive match
#[test]
fn row_3_case_insensitive_and_empty_strings_refused() {
    assert_eq!(CaptureStatus::parse("active"), None);
    assert_eq!(CaptureStatus::parse("superseded"), None);
    assert_eq!(CaptureStatus::parse(""), None);

    assert_eq!(CaptureSource::parse("app"), None);
    assert_eq!(CaptureSource::parse("booking"), None);
    assert_eq!(CaptureSource::parse("desk"), None);
    assert_eq!(CaptureSource::parse(""), None);
}

// Row 4: FeedValue::of("12.5") -> 1250
// Mutation it must catch: the scale-1 multiplier (:22)
#[test]
fn row_4_feed_value_scale_1_twelve_point_five() {
    assert_eq!(FeedValue::of(dec("12.5")).expect("valid feed value"), 1250);
}

// Row 5: FeedValue::of("0.5") -> 50
// Mutation it must catch: the scale-1 multiplier (:22)
#[test]
fn row_5_feed_value_scale_1_zero_point_five() {
    assert_eq!(FeedValue::of(dec("0.5")).expect("valid feed value"), 50);
}
