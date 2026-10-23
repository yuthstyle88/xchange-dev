//! Ticket #4993 (Q1 of #4961) — a signed foreign quantity, `QuantityDelta`, and checked
//! arithmetic on `ForeignAmount`, so a delta of `KWD 1.125` can be held. Written before the fix.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Rows U1–U8 of the ticket's §3 table, one test each. U9 (`MoneyDelta` and `Money` unchanged)
//! is the existing `xc-kernel` tests staying green.
//!
//! No database; these run anywhere.

use std::str::FromStr;

use rust_decimal::Decimal;
use xc_kernel::money::{
    CurrencyCode, CurrencyUnits, ForeignAmount, MinorUnit, NumberError, QuantityDelta, UnitSize,
};

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("test decimal")
}

fn units(code: &str, places: u8) -> CurrencyUnits {
    CurrencyUnits::new(
        CurrencyCode::parse(code).expect("currency code"),
        UnitSize::ONE,
        MinorUnit::new(places).expect("minor unit"),
    )
}

fn amount(s: &str) -> ForeignAmount {
    ForeignAmount::from_decimal(d(s))
}

fn delta(s: &str) -> QuantityDelta {
    QuantityDelta::from_decimal(d(s)).expect("test delta")
}

/// U1: up to four decimal places, either sign; a fifth is `TooManyDecimals`.
#[test]
fn u1_from_decimal_takes_up_to_four_places_of_either_sign() {
    assert_eq!(
        QuantityDelta::from_decimal(d("1.125")).map(QuantityDelta::value),
        Ok(d("1.125"))
    );
    assert_eq!(
        QuantityDelta::from_decimal(d("-0.5")).map(QuantityDelta::value),
        Ok(d("-0.5"))
    );
    assert_eq!(
        QuantityDelta::from_decimal(d("1.12345")).map(QuantityDelta::value),
        Err(NumberError::TooManyDecimals)
    );
}

/// U2: `ZERO + KWD 1.125 − KWD 0.125` is exactly 1.000; nothing is rounded on the way.
#[test]
fn u2_plus_then_minus_keeps_three_places_exactly() {
    let up = QuantityDelta::ZERO
        .checked_plus(amount("1.125"))
        .expect("plus");
    assert_eq!(up.value(), d("1.125"), "the plus step must not round");
    let back = up.checked_minus(amount("0.125")).expect("minus");
    assert_eq!(back.value(), d("1.000"));
    assert_eq!(back.value(), Decimal::ONE);
}

/// U3: a delta plus its opposite is zero.
#[test]
fn u3_checked_add_of_opposites_is_zero() {
    let sum = delta("1.125").checked_add(delta("-1.125")).expect("add");
    assert!(sum.is_zero(), "1.125 + -1.125 must be zero; got {sum}");
    assert!(!sum.is_negative());
    assert_eq!(sum.value(), Decimal::ZERO);
}

/// U4: a negative delta is not an amount; a positive one is.
#[test]
fn u4_to_amount_refuses_negative() {
    let neg = delta("-0.001");
    assert!(neg.is_negative());
    assert!(
        neg.to_amount().is_err(),
        "-0.001 must not become a ForeignAmount"
    );
    assert_eq!(delta("2.5").to_amount(), Ok(amount("2.5")));
}

/// U5: `fits` compares the normalized scale with the currency's minor unit.
#[test]
fn u5_fits_compares_normalized_scale_with_minor_unit() {
    let kwd = units("KWD", 3);
    let usd = units("USD", 2);
    let jpy = units("JPY", 0);
    assert!(delta("1.125").fits(&kwd), "1.125 fits KWD (3)");
    assert!(!delta("1.125").fits(&usd), "1.125 does not fit USD (2)");
    assert!(!delta("1.5").fits(&jpy), "1.5 does not fit JPY (0)");
    assert!(
        delta("1.000").fits(&jpy),
        "1.000 is 1 once normalized, fits JPY (0)"
    );

    assert!(
        amount("1.125").fits(&kwd),
        "ForeignAmount 1.125 fits KWD (3)"
    );
    assert!(
        !amount("1.125").fits(&usd),
        "ForeignAmount 1.125 does not fit USD (2)"
    );
    assert!(
        !amount("1.5").fits(&jpy),
        "ForeignAmount 1.5 does not fit JPY (0)"
    );
    assert!(
        amount("1.000").fits(&jpy),
        "ForeignAmount 1.000 fits JPY (0)"
    );
}

/// U6: `ForeignAmount::checked_minus` refuses a result below zero.
#[test]
fn u6_foreign_amount_checked_minus_refuses_negative_result() {
    assert!(
        amount("1.0").checked_minus(amount("1.125")).is_err(),
        "1.0 − 1.125 must be refused"
    );
    assert_eq!(
        amount("1.125").checked_minus(amount("0.125")),
        Ok(amount("1.000"))
    );
    assert_eq!(
        ForeignAmount::ZERO.checked_plus(amount("1.125")),
        Ok(amount("1.125"))
    );
    assert!(ForeignAmount::ZERO.is_zero());
    assert!(!amount("0.001").is_zero());
}

/// U7: `checked_from_decimal` refuses a negative value and a fifth place; four places are fine.
#[test]
fn u7_foreign_amount_checked_from_decimal() {
    assert!(
        ForeignAmount::checked_from_decimal(d("-1")).is_err(),
        "-1 must be refused"
    );
    assert!(
        ForeignAmount::checked_from_decimal(d("1.12345")).is_err(),
        "1.12345 must be refused"
    );
    assert_eq!(
        ForeignAmount::checked_from_decimal(d("1.1250")).map(ForeignAmount::value),
        Ok(d("1.1250"))
    );
}

/// U8: `Display` is the plain decimal as held, never an exponent.
#[test]
fn u8_display_is_plain_decimal() {
    assert_eq!(delta("1.1250").to_string(), "1.1250");
    assert_eq!(delta("-0.5").to_string(), "-0.5");
    assert_eq!(QuantityDelta::from(amount("1.125")).to_string(), "1.125");
}
