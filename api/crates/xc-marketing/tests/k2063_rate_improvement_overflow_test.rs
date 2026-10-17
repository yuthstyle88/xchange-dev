//! Tests for checked money arithmetic in `RateImprovement::apply` (ticket #2854 / #2063 tail).
//!
//! Asserts that when intermediate THB amounts overflow `Decimal` or exceed `i64::MAX`
//! hundredths, `RateImprovement::apply` returns `Err(ParamError)` with field `"quantity"`,
//! rather than panicking or wrapping silently.

use std::str::FromStr;

use rust_decimal::Decimal;
use serde_json::json;

use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};
use xc_marketing::domain::benefit::calculator::BenefitCalculator;
use xc_marketing::domain::benefit::kinds::rate_improvement::RateImprovement;
use xc_marketing::domain::engine::engine::QuoteLineInput;

fn make_line(
    side: Side,
    quantity: ForeignAmount,
    base_rate: Rate,
    unit_size: UnitSize,
) -> QuoteLineInput {
    QuoteLineInput {
        currency: CurrencyCode::parse("USD").unwrap(),
        shop_side: side,
        quantity,
        unit_size,
        base_rate,
        published_buy_rate: base_rate,
        published_sell_rate: base_rate,
        fee_thb: Money::ZERO,
    }
}

// ── K-a: ExchangeArithmetic::try_thb overflow (Decimal overflow) ──

#[test]
fn test_overflow_ka_thb_new() {
    let calc = RateImprovement;
    // In Side::Buy: new_rate = base_rate + delta.
    // If base_rate = 0.0001 and delta = 99999999.9999, new_rate = 100000000 (1e8).
    // With quantity = 1e21 and unit_size = 1:
    // thb_new = 1e21 * 1e8 = 1e29 (overflows Decimal::MAX ~7.9e28).
    // thb_base = 1e21 * 0.0001 = 1e17 (fits in Decimal).
    // Only thb_new overflows.
    let extreme_qty =
        ForeignAmount::from_decimal(Decimal::from_str("1000000000000000000000").unwrap());
    let base_rate = Rate::parse("0.0001").unwrap();
    let unit = UnitSize::ONE;
    let line = make_line(Side::Buy, extreme_qty, base_rate, unit);

    let params = json!({ "delta": "99999999.9999" });
    let err = calc
        .apply(&line, &params)
        .expect_err("Decimal overflow in thb_new must return ParamError");
    assert_eq!(err.field, "quantity");
}

#[test]
fn test_overflow_ka_thb_base() {
    let calc = RateImprovement;
    // In Side::Sell: new_rate = base_rate - delta.
    // If base_rate = 1e8 and delta = 99999999.9999, new_rate = 0.0001.
    // With quantity = 1e21 and unit_size = 1:
    // thb_new = 1e21 * 0.0001 = 1e17 (fits in Decimal).
    // thb_base = 1e21 * 1e8 = 1e29 (overflows Decimal::MAX ~7.9e28).
    // Only thb_base overflows.
    let extreme_qty =
        ForeignAmount::from_decimal(Decimal::from_str("1000000000000000000000").unwrap());
    let base_rate = Rate::parse("100000000").unwrap();
    let unit = UnitSize::ONE;
    let line = make_line(Side::Sell, extreme_qty, base_rate, unit);

    let params = json!({ "delta": "99999999.9999" });
    let err = calc
        .apply(&line, &params)
        .expect_err("Decimal overflow in thb_base must return ParamError");
    assert_eq!(err.field, "quantity");
}

// ── K-b: try_minor() overflow (past i64::MAX hundredths, fits Decimal) ──
//
// i64::MAX hundredths ≈ 9.223e16 THB (9.223e18 satang).
// With quantity = 2,600,000,000,000,000 (2.6e15) and unit_size = 1:
// - at rate 35.00: 2.6e15 * 35 = 9.1e16 THB = 9.1e18 satang (< i64::MAX, fits)
// - at rate 36.00: 2.6e15 * 36 = 9.36e16 THB = 9.36e18 satang (> i64::MAX, overflows)

#[test]
fn test_overflow_kb_buy_new_minor() {
    let calc = RateImprovement;
    // Side::Buy with base 35.00 and delta 1.00:
    // thb_base is at 35.00 = 9.1e16 THB (fits in i64 hundredths).
    // thb_new is at 36.00 = 9.36e16 THB (overflows i64 hundredths).
    // Only thb_new.try_minor() (:89) overflows; thb_base fits.
    let qty = ForeignAmount::from_decimal(Decimal::from_str("2600000000000000").unwrap());
    let base_rate = Rate::parse("35.00").unwrap();
    let line = make_line(Side::Buy, qty, base_rate, UnitSize::ONE);

    let params = json!({ "delta": "1.00" });
    let err = calc
        .apply(&line, &params)
        .expect_err("i64 minor overflow in Buy thb_new must return ParamError");
    assert_eq!(err.field, "quantity");
}

#[test]
fn test_overflow_kb_sell_base_minor() {
    let calc = RateImprovement;
    // Side::Sell with base 36.00 and delta 1.00:
    // thb_base is at 36.00 = 9.36e16 THB (overflows i64 hundredths).
    // thb_new is at 35.00 = 9.1e16 THB (fits in i64 hundredths).
    // Only thb_base.try_minor() (:101) overflows; thb_new fits.
    let qty = ForeignAmount::from_decimal(Decimal::from_str("2600000000000000").unwrap());
    let base_rate = Rate::parse("36.00").unwrap();
    let line = make_line(Side::Sell, qty, base_rate, UnitSize::ONE);

    let params = json!({ "delta": "1.00" });
    let err = calc
        .apply(&line, &params)
        .expect_err("i64 minor overflow in Sell thb_base must return ParamError");
    assert_eq!(err.field, "quantity");
}

// ── Non-overflow keep-pins: ordinary figures calculate correctly ──

#[test]
fn test_ordinary_figures_buy_delta() {
    let calc = RateImprovement;
    let qty = ForeignAmount::from_decimal(Decimal::from(100));
    let base_rate = Rate::parse("35.00").unwrap();
    let line = make_line(Side::Buy, qty, base_rate, UnitSize::ONE);

    let params = json!({ "delta": "0.10" });
    let effect = calc
        .apply(&line, &params)
        .expect("apply ordinary buy delta");

    assert_eq!(effect.new_final_rate, Some(Rate::parse("35.10").unwrap()));
    assert_eq!(effect.new_fee_thb, None);
    // (100 * 35.10) - (100 * 35.00) = 3510 - 3500 = 10 THB = 1000 satang
    assert_eq!(effect.amount_thb, Money::from_minor(1000).unwrap());
}

#[test]
fn test_ordinary_figures_sell_delta() {
    let calc = RateImprovement;
    let qty = ForeignAmount::from_decimal(Decimal::from(100));
    let base_rate = Rate::parse("35.00").unwrap();
    let line = make_line(Side::Sell, qty, base_rate, UnitSize::ONE);

    let params = json!({ "delta": "0.10" });
    let effect = calc
        .apply(&line, &params)
        .expect("apply ordinary sell delta");

    assert_eq!(effect.new_final_rate, Some(Rate::parse("34.90").unwrap()));
    assert_eq!(effect.new_fee_thb, None);
    // (100 * 35.00) - (100 * 34.90) = 3500 - 3490 = 10 THB = 1000 satang
    assert_eq!(effect.amount_thb, Money::from_minor(1000).unwrap());
}

#[test]
fn test_ordinary_figures_percent() {
    let calc = RateImprovement;
    let qty = ForeignAmount::from_decimal(Decimal::from(100));
    let base_rate = Rate::parse("35.00").unwrap();
    let line = make_line(Side::Buy, qty, base_rate, UnitSize::ONE);

    // 1% of 35.00 is 0.35, new rate is 35.35
    let params = json!({ "percent": "0.01" });
    let effect = calc.apply(&line, &params).expect("apply ordinary percent");

    assert_eq!(effect.new_final_rate, Some(Rate::parse("35.35").unwrap()));
    assert_eq!(effect.new_fee_thb, None);
    // (100 * 35.35) - (100 * 35.00) = 3535 - 3500 = 35 THB = 3500 satang
    assert_eq!(effect.amount_thb, Money::from_minor(3500).unwrap());
}
