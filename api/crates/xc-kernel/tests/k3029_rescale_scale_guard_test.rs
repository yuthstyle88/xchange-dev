//! #3029: a THB figure too wide to hold at 2 decimal places is refused, never returned ÷10 or ÷100.
//! Written before the implementation by the ticket author (claude/dell-claude-4). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! `Decimal` holds a 96-bit mantissa, so at scale 2 it holds at most (2^96 − 1) hundredths
//! (792,281,625,142,643,375,935,439,503.35). Past that, `rescale(2)` silently keeps a smaller
//! scale. Today `ExchangeArithmetic::thb` / `try_thb` return such a `Money`, and
//! `Money::minor_i128` reads its mantissa as hundredths, so the figure comes back ÷10 or ÷100.
//!
//! The rule (BE controller, #3029): the checked `try_thb` returns `Err(ArithmeticOverflow)`; the
//! unchecked `thb` and `minor_i128` panic with a message containing
//! `not representable at 2 decimal places`; `try_minor` keeps returning `Err(NumberError::Invalid)`
//! and never panics.

use std::str::FromStr;

use rust_decimal::Decimal;
use xc_kernel::money::{
    ArithmeticOverflow, ExchangeArithmetic, ForeignAmount, Money, NumberError, Quote, Rate, Side,
    UnitSize,
};

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

fn foreign(text: &str) -> ForeignAmount {
    ForeignAmount::from_decimal(d(text))
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).unwrap_or_else(|e| panic!("{text:?}: {e:?}"))
}

fn unit(text: &str) -> UnitSize {
    UnitSize::parse(text).unwrap_or_else(|e| panic!("{text:?}: {e:?}"))
}

/// The largest THB `Money` holds at 2 places: (2^96 − 1) hundredths.
const WIDEST_AT_TWO_PLACES: &str = "792281625142643375935439503.35";

/// A `Money` whose value is one baht past `WIDEST_AT_TWO_PLACES`, reached through public API
/// only: a SELL quote whose principal is the widest figure and whose fee is 1.00. Its `received`
/// cannot be held at 2 places.
fn money_one_baht_past_the_widest() -> Money {
    let q = Quote::per_unit(
        Side::Sell,
        foreign(WIDEST_AT_TWO_PLACES),
        rate("1"),
        UnitSize::ONE,
        Money::parse("1").expect("a fee"),
    )
    .expect("the principal itself still fits at 2 places");
    q.received
}

// ── try_thb refuses a figure too wide for 2 places ──────────────────────────

#[test]
fn try_thb_refuses_a_figure_too_wide_for_two_places() {
    // 10^19 × 100,000,000 ÷ 1 = 10^27 THB: fits Decimal, not at 2 places.
    let r = ExchangeArithmetic::try_thb(
        foreign("10000000000000000000"),
        rate("100000000"),
        UnitSize::ONE,
    );
    let _: ArithmeticOverflow = r.expect_err("10^27 THB cannot be held at 2 places");
}

#[test]
fn try_thb_refuses_the_transaction_row_worst_case() {
    // The #3029 figure: quantity numeric(20,4) max × the Rate::parse cap ÷ unit_size 0.0001.
    let r = ExchangeArithmetic::try_thb(
        foreign("9999999999999999.9999"),
        rate("100000000"),
        unit("0.0001"),
    );
    let _: ArithmeticOverflow = r.expect_err("≈10^28 THB cannot be held at 2 places");
}

// ── minor_i128 panics instead of returning a mis-scaled figure ──────────────

#[test]
#[should_panic(expected = "not representable at 2 decimal places")]
fn minor_i128_panics_rather_than_reading_a_mis_scaled_mantissa() {
    let _ = money_one_baht_past_the_widest().minor_i128();
}

// ── try_minor still refuses, and never panics ───────────────────────────────

#[test]
fn try_minor_of_a_figure_too_wide_for_two_places_is_invalid_and_never_panics() {
    let m = money_one_baht_past_the_widest();
    let r = std::panic::catch_unwind(|| m.try_minor());
    assert_eq!(
        r.expect("try_minor must not panic"),
        Err(NumberError::Invalid)
    );
}

// ── the widest figure that fits is unchanged ────────────────────────────────

#[test]
fn the_widest_figure_at_two_places_is_still_computed_exactly() {
    let widest = (1_i128 << 96) - 1;
    let checked =
        ExchangeArithmetic::try_thb(foreign(WIDEST_AT_TWO_PLACES), rate("1"), UnitSize::ONE)
            .expect("fits at 2 places");
    assert_eq!(checked.minor_i128(), widest);
    assert_eq!(checked.to_string(), WIDEST_AT_TWO_PLACES);
}
