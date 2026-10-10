//! FD-1 up-front tests — `FeedCursor` and `FeedValue` (design broker-seam.md §6.2, §6.4, §11
//! "FD-1"). No database. Written before the implementation by the ticket author. Do not edit,
//! delete or weaken; add tests in a file of your own.

use std::str::FromStr;

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_referral::domain::feed_cursor::{CursorError, FeedCursor};
use xc_referral::domain::feed_value::FeedValue;

const ID: &str = "3f0c1d2e-4b5a-4c6d-8e7f-0a1b2c3d4e5f";
const NIL: &str = "00000000-0000-0000-0000-000000000000";

fn cursor(micros: i64, id: &str) -> FeedCursor {
    FeedCursor {
        updated_at: Utc.timestamp_micros(micros).single().expect("an instant"),
        id: Uuid::parse_str(id).expect("a uuid"),
    }
}

// ── the cursor ──────────────────────────────────────────────────────────────

#[test]
fn v3_writes_microseconds_and_z_then_the_hyphenated_id() {
    // 2026-10-10T06:00:00.123456Z
    let c = cursor(1_791_612_000_123_456, ID);
    assert_eq!(c.encode(), format!("2026-10-10T06:00:00.123456Z_{ID}"));
}

#[test]
fn a_cursor_round_trips() {
    for c in [
        cursor(1_791_612_000_123_456, ID),
        cursor(1_791_612_000_000_000, NIL),
    ] {
        assert_eq!(FeedCursor::parse(&c.encode()), Ok(c));
    }
}

#[test]
fn brokers_lookback_cursor_with_the_nil_uuid_is_accepted() {
    // Broker's CpCursor::encode of `stored − 10 min` with the nil UUID (Broker §7).
    let text = format!("2026-10-10T05:50:00.123456Z_{NIL}");
    assert_eq!(
        FeedCursor::parse(&text),
        Ok(cursor(1_791_611_400_123_456, NIL))
    );
}

#[test]
fn any_rfc3339_precision_and_offset_reads_as_the_same_utc_instant() {
    let whole = cursor(1_791_612_000_000_000, NIL);
    assert_eq!(
        FeedCursor::parse(&format!("2026-10-10T06:00:00Z_{NIL}")),
        Ok(whole)
    );
    assert_eq!(
        FeedCursor::parse(&format!("2026-10-10T13:00:00+07:00_{NIL}")),
        Ok(whole)
    );
    // 9 digits truncate to microseconds (timestamptz's precision); 3 digits pad.
    assert_eq!(
        FeedCursor::parse(&format!("2026-10-10T06:00:00.123456789Z_{NIL}")),
        Ok(cursor(1_791_612_000_123_456, NIL))
    );
    assert_eq!(
        FeedCursor::parse(&format!("2026-10-10T06:00:00.120Z_{NIL}")),
        Ok(cursor(1_791_612_000_120_000, NIL))
    );
}

#[test]
fn garbage_is_refused() {
    for text in [
        "",
        "2026-10-10T06:00:00Z",                // no `_`
        &format!("_{ID}"),                     // no time
        &format!("2026-10-10 06:00:00_{ID}"),  // not RFC 3339
        &format!("2026-13-10T06:00:00Z_{ID}"), // no such month
        "2026-10-10T06:00:00Z_not-a-uuid",
        "2026-10-10T06:00:00Z_3f0c1d2e4b5a4c6d8e7f0a1b2c3d4e5f", // not hyphenated
        &format!("2026-10-10T06:00:00Z_{ID}_x"),                 // trailing junk in the id
    ] {
        assert_eq!(FeedCursor::parse(text), Err(CursorError), "{text:?}");
    }
}

// ── the value ───────────────────────────────────────────────────────────────

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal")
}

#[test]
fn the_value_is_the_principal_in_satang() {
    assert_eq!(FeedValue::of(dec("1234.56")).expect("valid"), 123_456);
    assert_eq!(FeedValue::of(dec("0.01")).expect("valid"), 1);
    assert_eq!(FeedValue::of(dec("32616.30")).expect("valid"), 3_261_630);
    assert_eq!(FeedValue::of(dec("100")).expect("valid"), 10_000);
}

#[test]
fn a_value_that_is_not_a_positive_two_place_amount_is_refused() {
    for s in ["0", "0.00", "-1.00", "1.005"] {
        assert!(
            matches!(
                FeedValue::of(dec(s)),
                Err(pos_error::AppError::Validation(_))
            ),
            "{s}"
        );
    }
    let too_big = Decimal::from(i64::MAX);
    assert!(matches!(
        FeedValue::of(too_big),
        Err(pos_error::AppError::Validation(_))
    ));
}
