//! `Money::try_minor`: hundredths as `i64`, refused when they do not fit.
//! Written before the implementation by the ticket author (claude/d7aa64). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Today `Money::minor()` is `d.mantissa() as i64`: past `i64::MAX` hundredths it wraps
//! silently (2^63 hundredths comes back as `i64::MIN`). A computed `Money` can get there:
//! `ExchangeArithmetic::thb` has no upper cap. `try_minor` returns
//! `Err(NumberError::Invalid)` instead, the same error `Money::try_from_minor_i128` uses
//! for a figure out of range.
//!
//! `today_minor_wraps_past_i64` documents the bug in `minor()`; the ticket that removes
//! `minor()` deletes that one test with it.

use rust_decimal::Decimal;
use xc_kernel::money::{ExchangeArithmetic, ForeignAmount, Money, NumberError, Rate, UnitSize};

fn money(text: &str) -> Money {
    Money::parse(text).unwrap_or_else(|e| panic!("{text:?}: {e:?}"))
}

fn from_hundredths(minor: i128) -> Money {
    Money::try_from_minor_i128(minor).expect("fits Decimal at scale 2")
}

#[test]
fn try_minor_of_a_typed_amount_is_its_hundredths() {
    let m = money("100.50");
    assert_eq!(m.try_minor(), Ok(10050));
    assert_eq!(m.try_minor(), Ok(m.minor()));
    assert_eq!(money("0.07").try_minor(), Ok(7));
    assert_eq!(money("100000000").try_minor(), Ok(10_000_000_000));
}

#[test]
fn try_minor_of_zero_is_zero() {
    assert_eq!(Money::ZERO.try_minor(), Ok(0));
}

#[test]
fn try_minor_refuses_an_amount_past_i64() {
    let m = from_hundredths(1_i128 << 70);
    assert_eq!(m.minor_i128(), 1_i128 << 70);
    assert_eq!(m.try_minor(), Err(NumberError::Invalid));
}

#[test]
fn try_minor_refuses_a_computed_principal_past_i64() {
    // 10^17 × 100,000,000 ÷ 1 = 10^25 THB: thb() computes it, i64 cannot hold its hundredths.
    let thb = ExchangeArithmetic::thb(
        ForeignAmount::from_decimal(Decimal::from(100_000_000_000_000_000_i64)),
        Rate::parse("100000000").expect("a rate"),
        UnitSize::ONE,
    );
    assert_eq!(thb.minor_i128(), 10_i128.pow(27));
    assert_eq!(thb.try_minor(), Err(NumberError::Invalid));
}

#[test]
fn today_minor_wraps_past_i64() {
    // The bug try_minor exists for: minor() cuts the mantissa to 64 bits with no error.
    let just_past = from_hundredths(i64::MAX as i128 + 1);
    assert_eq!(just_past.minor(), i64::MIN);
    assert_ne!(just_past.minor() as i128, just_past.minor_i128());
    let far_past = from_hundredths(1_i128 << 70);
    assert_eq!(far_past.minor(), 0);
}

#[test]
fn try_minor_holds_exactly_i64_max_and_refuses_one_more() {
    assert_eq!(from_hundredths(i64::MAX as i128).try_minor(), Ok(i64::MAX));
    assert_eq!(
        from_hundredths(i64::MAX as i128 + 1).try_minor(),
        Err(NumberError::Invalid)
    );
}

#[test]
fn try_minor_matches_minor_wherever_minor_is_right() {
    for minor in [
        0_i128,
        1,
        99,
        100,
        10050,
        1_000_000_000_000,
        i64::MAX as i128 - 1,
    ] {
        let m = from_hundredths(minor);
        assert_eq!(m.try_minor(), Ok(m.minor()), "{minor}");
        assert_eq!(m.try_minor(), Ok(minor as i64), "{minor}");
    }
}
