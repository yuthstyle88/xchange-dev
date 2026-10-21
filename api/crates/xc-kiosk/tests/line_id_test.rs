//! Ticket #4034 (A3c) — `LineId` text round-trip and UUID parsing without a database.

use xc_kiosk::domain::kiosk::value_objects::counter_facts::LineId;

/// Row 1: `LineId(x).to_string()` then parse, x = 0, 1, `u128::MAX`,
/// `0x0123456789abcdef0123456789abcdef` -> `Some(LineId(x))`.
#[test]
fn row1_display_then_parse_round_trips_all_groups() {
    for x in [0, 1, u128::MAX, 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef] {
        let rendered = LineId(x).to_string();
        assert_eq!(LineId::parse_uuid(&rendered), Some(LineId(x)));
    }
}

/// Row 2: `LineId(1).to_string()` -> `"00000000-0000-0000-0000-000000000001"`.
#[test]
fn row2_display_pads_each_group_to_uuid_width() {
    assert_eq!(
        LineId(1).to_string(),
        "00000000-0000-0000-0000-000000000001"
    );
}

/// Row 3: the same uuid in upper case -> `Some`.
#[test]
fn row3_parse_uuid_accepts_upper_case_hex() {
    assert_eq!(
        LineId::parse_uuid("01234567-89AB-CDEF-0123-456789ABCDEF"),
        Some(LineId(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef))
    );
    assert_eq!(
        LineId::parse_uuid(&LineId(1).to_string().to_uppercase()),
        Some(LineId(1))
    );
}

/// Row 4: 31 or 33 hex digits -> `None`.
#[test]
fn row4_parse_uuid_rejects_31_or_33_hex_digits() {
    assert_eq!(
        LineId::parse_uuid("00000000-0000-0000-0000-00000000000"),
        None
    );
    assert_eq!(
        LineId::parse_uuid("00000000-0000-0000-0000-0000000000001"),
        None
    );
}

/// Row 5: contains `g`; `""` -> `None`.
#[test]
fn row5_parse_uuid_rejects_non_hex_and_empty() {
    assert_eq!(
        LineId::parse_uuid("00000000-0000-0000-0000-00000000000g"),
        None
    );
    assert_eq!(LineId::parse_uuid(""), None);
}

/// Row 6: a 32-digit value with a leading `+` -> `None`
/// (`u128::from_str_radix` accepts a leading `+`, so a 32-char string starting with `+`
/// and 31 hex digits passes `hex.len() == 32` and parses as `Some`).
#[test]
#[ignore = "bug: + accepted"]
fn row6_parse_uuid_rejects_leading_plus() {
    assert_eq!(
        LineId::parse_uuid("+0000000-0000-0000-0000-000000000001"),
        None
    );
}
