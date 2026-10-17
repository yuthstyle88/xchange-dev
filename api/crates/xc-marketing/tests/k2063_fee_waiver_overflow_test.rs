//! Tests for K-b checked money arithmetic in `FeeWaiver::apply` (ticket #2853 / #2063 tail).
//!
//! Asserts that when `QuoteLineInput::fee_thb` exceeds `i64::MAX` hundredths, `FeeWaiver::apply`
//! returns `Err(ParamError)` with field `"fee_thb"`, rather than panicking or wrapping.

use std::str::FromStr;

use rust_decimal::Decimal;
use serde_json::json;

use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};
use xc_marketing::domain::benefit::calculator::BenefitCalculator;
use xc_marketing::domain::benefit::kinds::fee_waiver::FeeWaiver;
use xc_marketing::domain::engine::engine::QuoteLineInput;

fn make_line(fee_thb: Money) -> QuoteLineInput {
    QuoteLineInput {
        currency: CurrencyCode::parse("USD").unwrap(),
        shop_side: Side::Buy,
        quantity: ForeignAmount::from_decimal(Decimal::from_str("100").unwrap()),
        unit_size: UnitSize::ONE,
        base_rate: Rate::parse("35.00").unwrap(),
        published_buy_rate: Rate::parse("35.00").unwrap(),
        published_sell_rate: Rate::parse("36.00").unwrap(),
        fee_thb,
    }
}

// ── Overflow cases: fee_thb > i64::MAX hundredths ──

#[test]
fn test_overflow_fee_percent_one() {
    let waiver = FeeWaiver;
    let overflow_fee = Money::try_from_minor_i128(i64::MAX as i128 + 1).unwrap();
    let line = make_line(overflow_fee);

    let calc = json!({ "percent": "1.0" });
    let err = waiver
        .apply(&line, &calc)
        .expect_err("overflowing fee must return ParamError for percent 1.0");
    assert_eq!(err.field, "fee_thb");
}

#[test]
fn test_overflow_fee_percent_partial() {
    let waiver = FeeWaiver;
    let overflow_fee = Money::try_from_minor_i128(i64::MAX as i128 + 1).unwrap();
    let line = make_line(overflow_fee);

    let calc = json!({ "percent": "0.5" });
    let err = waiver
        .apply(&line, &calc)
        .expect_err("overflowing fee must return ParamError for percent 0.5");
    assert_eq!(err.field, "fee_thb");
}

#[test]
fn test_overflow_fee_fixed_amount() {
    let waiver = FeeWaiver;
    let overflow_fee = Money::try_from_minor_i128(i64::MAX as i128 + 1).unwrap();
    let line = make_line(overflow_fee);

    let calc = json!({ "amount": "50.00" });
    let err = waiver
        .apply(&line, &calc)
        .expect_err("overflowing fee must return ParamError for amount");
    assert_eq!(err.field, "fee_thb");
}

#[test]
fn test_overflow_fee_fixed() {
    let waiver = FeeWaiver;
    let overflow_fee = Money::try_from_minor_i128(i64::MAX as i128 + 1).unwrap();
    let line = make_line(overflow_fee);

    let calc = json!({ "fixed": "50.00" });
    let err = waiver
        .apply(&line, &calc)
        .expect_err("overflowing fee must return ParamError for fixed");
    assert_eq!(err.field, "fee_thb");
}

#[test]
fn test_overflow_fee_huge() {
    let waiver = FeeWaiver;
    let overflow_fee = Money::try_from_minor_i128(1_i128 << 70).unwrap();
    let line = make_line(overflow_fee);

    let calc = json!({ "percent": "1" });
    let err = waiver
        .apply(&line, &calc)
        .expect_err("huge overflowing fee must return ParamError");
    assert_eq!(err.field, "fee_thb");
}

// ── Non-overflow cases: ordinary figures must calculate correctly ──

#[test]
fn test_ordinary_figures_percent() {
    let waiver = FeeWaiver;
    let line = make_line(Money::parse("50.00").unwrap());

    let calc = json!({ "percent": "0.5" });
    let effect = waiver.apply(&line, &calc).expect("valid quote line");
    assert_eq!(effect.new_fee_thb, Some(Money::parse("25.00").unwrap()));
    assert_eq!(effect.amount_thb, Money::parse("25.00").unwrap());
}

#[test]
fn test_ordinary_figures_fixed() {
    let waiver = FeeWaiver;
    let line = make_line(Money::parse("50.00").unwrap());

    let calc = json!({ "amount": "20.00" });
    let effect = waiver.apply(&line, &calc).expect("valid quote line");
    assert_eq!(effect.new_fee_thb, Some(Money::parse("30.00").unwrap()));
    assert_eq!(effect.amount_thb, Money::parse("20.00").unwrap());
}
