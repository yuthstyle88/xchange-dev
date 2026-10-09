//! Ticket #756 (RE-4b) — `CompatRate::per_one`: the one function that turns a rate per
//! `unit_size` into v2's rate per 1, for the surfaces that keep v2's shape.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md`
//! - §3.1: `unit_size` is a positive `numeric(20,4)`; the register seeds JPY 100, KRW 1000,
//!   every other currency 1;
//! - §3.3: a `Rate` holds up to 8 places;
//! - §4.9 R3: the v2-compat wire stays per 1. That rate is `rate ÷ unit_size`, exact,
//!   normalised, through one function, `CompatRate::per_one(rate, unit_size)`
//!   (`api/crates/xc-kernel/src/money/compat_rate.rs`, created by RE-4b);
//! - ticket #756 §7: it is exact because a `unit_size` is a power of 10, so the division only
//!   moves the decimal point; the result has at most `8 + log10(unit_size)` places; a `unit_size`
//!   that is not a power of 10 is refused (`Err`), so a future register entry can never
//!   silently round.
//!
//! Needs no database; nothing here skips.
//!
//! Signatures this test assumes:
//! - [required] `xc_kernel::money::CompatRate::per_one(rate: Rate, unit_size: UnitSize)
//!   -> Result<rust_decimal::Decimal, E>`, for some error type `E` (this file never reads it).

use std::str::FromStr;

use rust_decimal::Decimal;
use xc_kernel::money::{CompatRate, Rate, UnitSize};

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap_or_else(|e| panic!("{text:?} is a decimal: {e}"))
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).unwrap_or_else(|e| panic!("{text:?} is a rate: {e:?}"))
}

fn unit(text: &str) -> UnitSize {
    UnitSize::parse(text).unwrap_or_else(|e| panic!("{text:?} is a unit size: {e:?}"))
}

fn per_one(rate_text: &str, unit_text: &str) -> Decimal {
    match CompatRate::per_one(rate(rate_text), unit(unit_text)) {
        Ok(value) => value,
        Err(_) => panic!("{rate_text} per {unit_text} has a rate per 1"),
    }
}

/// R3, "exact": the answer times the unit is the rate again, digit for digit, for every unit
/// the register can hold that is a power of 10. A 16-digit rate keeps all 16 digits.
#[test]
fn per_one_is_exact_for_1_10_100_and_1000() {
    for (rate_text, unit_text, expected) in [
        ("32.1", "1", "32.1"),
        ("23.50000000", "100", "0.235"),
        ("23.1", "100", "0.231"),
        ("0.2195", "10", "0.02195"),
        ("8.5", "1000", "0.0085"),
        ("27.12345678", "1000", "0.02712345678"),
        ("0.00000001", "1000", "0.00000000001"),
        ("12345678.12345678", "1", "12345678.12345678"),
        ("12345678.12345678", "10", "1234567.812345678"),
        ("12345678.12345678", "100", "123456.7812345678"),
        ("12345678.12345678", "1000", "12345.67812345678"),
        ("4321.5", "10000", "0.43215"),
    ] {
        let got = per_one(rate_text, unit_text);
        assert_eq!(got, d(expected), "{rate_text} per {unit_text}");
        assert_eq!(
            got * unit(unit_text).value(),
            rate(rate_text).value(),
            "{rate_text} per {unit_text}: nothing was rounded away"
        );
    }
}

/// R3, "normalised": no trailing zeros, whatever scale the rate and the unit were held at. The
/// database hands both back at their column scale (`numeric(20,8)`, `numeric(20,4)`).
#[test]
fn per_one_is_normalised() {
    for (rate_text, unit_text, text, scale) in [
        ("23.50000000", "100", "0.235", 3),
        ("33", "1", "33", 0),
        ("33.00000000", "1.0000", "33", 0),
        ("100", "100", "1", 0),
        ("2300.00000000", "100.0000", "23", 0),
        ("32.10000000", "1", "32.1", 1),
        ("23.1", "100.0000", "0.231", 3),
    ] {
        let got = per_one(rate_text, unit_text);
        assert_eq!(got.to_string(), text, "{rate_text} per {unit_text}");
        assert_eq!(got.scale(), scale, "{rate_text} per {unit_text}");
    }
}

/// Ticket #756 §7: at most `8 + log10(unit_size)` places. A rate with all 8 places, per
/// 10^k, has exactly 8 + k.
#[test]
fn per_one_has_at_most_eight_plus_log10_of_the_unit_places() {
    for (unit_text, places) in [
        ("1", 8),
        ("10", 9),
        ("100", 10),
        ("1000", 11),
        ("10000", 12),
    ] {
        let got = per_one("0.12345678", unit_text);
        assert_eq!(got.scale(), places, "0.12345678 per {unit_text}");
        assert_eq!(got * unit(unit_text).value(), d("0.12345678"));
    }
}

/// A `unit_size` read from `numeric(20,4)` is `100.0000`; it is the same power of 10 as `100`.
#[test]
fn per_one_reads_a_unit_held_at_scale_four_as_its_power_of_ten() {
    let held = UnitSize::from_decimal(d("100.0000")).expect("100.0000 is a unit size");
    let got = match CompatRate::per_one(rate("23.5"), held) {
        Ok(value) => value,
        Err(_) => panic!("100.0000 is a power of 10"),
    };
    assert_eq!(got.to_string(), "0.235");
    let held = UnitSize::from_decimal(d("1000.0000")).expect("1000.0000 is a unit size");
    let got = match CompatRate::per_one(rate("8.5"), held) {
        Ok(value) => value,
        Err(_) => panic!("1000.0000 is a power of 10"),
    };
    assert_eq!(got.to_string(), "0.0085");
}

/// Ticket #756 §7, the invariant: a `unit_size` that is not a power of 10 is refused, because
/// dividing by it could need rounding. `UnitSize` itself accepts every one of these.
#[test]
fn per_one_refuses_a_unit_size_that_is_not_a_power_of_ten() {
    for unit_text in [
        "2", "3", "5", "25", "50", "99", "101", "200", "250", "999", "1001", "1.5", "0.5", "100.5",
        "12.3456",
    ] {
        assert!(
            CompatRate::per_one(rate("23.5"), unit(unit_text)).is_err(),
            "23.5 per {unit_text}: {unit_text} is not a power of 10"
        );
    }
    // the refusal is about the unit, not the rate: the same rate per 100 is answered
    assert_eq!(per_one("23.5", "100"), d("0.235"));
}
