//! Bangkok's clock (ticket #88).
//!
//! STUB of the first commit: wrong values, so `tests/input_test.rs` fails on wrong answers.

use chrono::FixedOffset;

/// Bangkok's offset from UTC, in seconds (UTC+7, no daylight saving).
pub const BANGKOK_OFFSET_SECONDS: i32 = 25_200;

/// Bangkok's fixed offset.
pub fn bangkok() -> FixedOffset {
    FixedOffset::east_opt(BANGKOK_OFFSET_SECONDS).expect("Bangkok is UTC+7")
}
