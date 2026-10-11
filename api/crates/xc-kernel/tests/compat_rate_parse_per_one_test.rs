//! Ticket #770 — `CompatRate::parse_per_one`: the one parser of the per-1 rate *text* on the
//! v2-compat wire, so that the bookings route and the desk save's `TransactionForm` stop each
//! carrying a copy of `Rate::parse`.
//! Written before the implementation by the ticket author (claude/5d45b017, on the xchange lane
//! controller's order). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md`
//! - §4.9 R3: the v2-compat wire stays per 1. On the way in, a per-1 rate takes up to **11**
//!   places, and a 12th **non-zero** place is `TOO_MANY_DECIMALS`. Eleven is
//!   `8 + log10(1000)`, the register's largest `unit_size` (KRW);
//! - the value the function returns is what `CompatRate::per_one(rate, unit_size)` produces for
//!   that line, so every `per_one` result must read back as itself.
//!
//! Pinned by the ticket author (ticket #770 §4 left trailing zeros open):
//! - **zeros past the 11th place are not a 12th place**, the same rule `Rate::parse` keeps past
//!   its 8th place (`rate_units_test.rs`, "trailing zeros past the eighth place are not a ninth
//!   place") and the rule `bookings.rs`' `parse_wire_rate` and RE-4c's form already follow;
//! - the limit is 11 whatever the currency: the function takes no `unit_size`.
//!
//! Needs no database; nothing here skips.
//!
//! Signatures this test assumes:
//! - [required] `xc_kernel::money::CompatRate::parse_per_one(text: &str)
//!   -> Result<rust_decimal::Decimal, xc_kernel::money::NumberError>`;
//! - `CompatRate::per_one`, `Rate::parse`, `UnitSize::parse` and `Money::MAX_INPUT_UNITS` as on
//!   main `d53e084`.
//!
//! Values are compared as numbers (`Decimal`'s `==`), never by scale or by printed text: the
//! ticket asks for the exact value and leaves its scale to the implementer.

use std::str::FromStr;

use rust_decimal::Decimal;
use xc_kernel::money::{CompatRate, Money, NumberError, Rate, UnitSize};

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap_or_else(|e| panic!("{text:?} is a decimal: {e}"))
}

fn taken(text: &str) -> Decimal {
    CompatRate::parse_per_one(text)
        .unwrap_or_else(|e| panic!("{text:?} is a per-1 rate the wire can carry, got {e:?}"))
}

fn refused(text: &str) -> NumberError {
    match CompatRate::parse_per_one(text) {
        Err(e) => e,
        Ok(v) => panic!("{text:?} must be refused, got Ok({v})"),
    }
}

/// KRW 25.12345678 per 1000 is 0.02512345678 per 1: eleven places. Ten, nine, eight and fewer
/// are taken too, and the value is exact.
#[test]
fn up_to_eleven_places_are_taken_exactly() {
    for text in [
        "0.02512345678",
        "0.0251234567",
        "0.025123456",
        "0.23512345",
        "0.235",
        "32.1",
        "35",
        "99999999.99999999999",
    ] {
        assert_eq!(taken(text), d(text), "{text}");
    }
}

/// A 12th non-zero place is more than any register unit gives.
#[test]
fn a_twelfth_non_zero_place_is_too_many_decimals() {
    for text in [
        "0.025123456781",
        "32.100000000001",
        "0.000000000001",
        "1.0000000000005",
    ] {
        assert_eq!(refused(text), NumberError::TooManyDecimals, "{text}");
    }
}

/// Zeros past the 11th place are not a 12th place (pinned above): the value is the number
/// without them.
#[test]
fn zeros_past_the_eleventh_place_are_not_a_twelfth_place() {
    for (text, value) in [
        ("0.025123456780", "0.02512345678"),
        ("0.02512345678000", "0.02512345678"),
        ("32.1000000000000", "32.1"),
    ] {
        assert_eq!(taken(text), d(value), "{text}");
    }
}

/// Zero, a negative number, a bare leading or trailing dot, two dots, signs, exponents, spaces
/// and anything that is not a plain decimal are `Invalid`.
#[test]
fn what_is_not_a_positive_plain_decimal_is_invalid() {
    for text in [
        "",
        "0",
        "0.0",
        "0.00000000000",
        "-1",
        "-0.5",
        ".5",
        "5.",
        ".",
        "1.2.3",
        "1..2",
        "+5",
        "1e3",
        "1E-3",
        " 5",
        "5 ",
        "3,5",
        "abc",
        "NaN",
    ] {
        assert_eq!(refused(text), NumberError::Invalid, "{text:?}");
    }
}

/// `Money::MAX_INPUT_UNITS` is the ceiling: equal is taken, above is `Invalid`.
#[test]
fn above_max_input_units_is_invalid() {
    let max = Money::MAX_INPUT_UNITS.to_string();
    assert_eq!(taken(&max), Decimal::from(Money::MAX_INPUT_UNITS));
    for text in [
        format!("{}", Money::MAX_INPUT_UNITS + 1),
        format!("{max}.00000000001"),
        "1000000000".to_owned(),
    ] {
        assert_eq!(refused(&text), NumberError::Invalid, "{text}");
    }
}

/// A 12th place on a number above the ceiling is refused. Which of the two errors it gets is
/// not pinned.
#[test]
fn a_long_fraction_above_the_ceiling_is_refused() {
    let text = format!("{}.000000000001", Money::MAX_INPUT_UNITS + 1);
    assert!(CompatRate::parse_per_one(&text).is_err(), "{text}");
}

/// Every per-1 rate the server prints for a line reads back as itself: whatever `per_one`
/// gives for a rate of up to 8 places at `unit_size` 1, 10, 100 or 1000, the wire takes back
/// unchanged. This is the round trip the kept v2 desk and booking pages make.
#[test]
fn every_per_one_reads_back_as_itself() {
    let rates = [
        "0.00000001",
        "0.235",
        "23.5",
        "25.12345678",
        "32.1",
        "99999999.99999999",
        "100000000",
    ];
    for unit in ["1", "10", "100", "1000"] {
        let unit_size =
            UnitSize::parse(unit).unwrap_or_else(|e| panic!("{unit:?} is a unit size: {e:?}"));
        for r in rates {
            let rate = Rate::parse(r).unwrap_or_else(|e| panic!("{r:?} is a rate: {e:?}"));
            let per_one = CompatRate::per_one(rate, unit_size)
                .unwrap_or_else(|e| panic!("{r} per {unit} has a per-1 rate: {e:?}"));
            let printed = per_one.to_string();
            assert_eq!(
                taken(&printed),
                per_one,
                "{r} per {unit} printed as {printed}"
            );
        }
    }
}
