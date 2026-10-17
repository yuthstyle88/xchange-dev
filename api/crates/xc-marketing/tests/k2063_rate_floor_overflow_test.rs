use std::str::FromStr;

use rust_decimal::Decimal;
use xc_kernel::money::{
    ArithmeticOverflow, CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize,
};
use xc_marketing::domain::benefit::calculator::BenefitEffect;
use xc_marketing::domain::benefit::rule::BenefitClass;
use xc_marketing::domain::engine::engine::QuoteLineInput;
use xc_marketing::domain::engine::rate_floor::RateFloor;

fn line(
    qty: &str,
    base_rate: &str,
    published_buy: &str,
    published_sell: &str,
    fee: &str,
    unit: &str,
    side: Side,
) -> QuoteLineInput {
    QuoteLineInput {
        currency: CurrencyCode::parse("USD").unwrap(),
        shop_side: side,
        quantity: ForeignAmount::from_decimal(Decimal::from_str(qty).unwrap()),
        unit_size: UnitSize::parse(unit).unwrap(),
        base_rate: Rate::parse(base_rate).unwrap(),
        fee_thb: Money::parse(fee).unwrap(),
        published_buy_rate: Rate::parse(published_buy).unwrap(),
        published_sell_rate: Rate::parse(published_sell).unwrap(),
    }
}

/// K-a site 1 (rate_floor.rs:97): ExchangeArithmetic::try_thb for final_rate overflows Decimal (> ~7.9e28).
#[test]
fn test_overflow_try_thb_final_rate() {
    // 1e18 foreign * 1e8 rate / 0.0001 unit_size = 1e30 > Decimal::MAX
    let quote_line = line(
        "1000000000000000000",
        "1.0000",
        "1.0000",
        "100000000",
        "0.00",
        "0.0001",
        Side::Buy,
    );
    let effect = BenefitEffect {
        new_final_rate: Some(Rate::parse("100000000").unwrap()),
        new_fee_thb: None,
        amount_thb: Money::ZERO,
    };

    let result = RateFloor::apply(&quote_line, &effect, BenefitClass::Price, None);
    assert_eq!(result, Err(ArithmeticOverflow));
}

/// K-a site 2 (rate_floor.rs:99): ExchangeArithmetic::try_thb for base_rate overflows Decimal (> ~7.9e28).
#[test]
fn test_overflow_try_thb_base_rate() {
    // final_rate does not overflow (1e18 * 1 / 1 = 1e18), but base_rate does:
    // 1e18 foreign * 1e8 rate / 0.0001 unit_size = 1e30 > Decimal::MAX
    let quote_line = line(
        "1000000000000000000",
        "100000000",
        "1.0000",
        "100000000",
        "0.00",
        "0.0001",
        Side::Sell,
    );
    let effect = BenefitEffect {
        new_final_rate: Some(Rate::parse("1.0000").unwrap()),
        new_fee_thb: None,
        amount_thb: Money::ZERO,
    };

    let result = RateFloor::apply(&quote_line, &effect, BenefitClass::Price, None);
    assert_eq!(result, Err(ArithmeticOverflow));
}

/// K-b sites (rate_floor.rs:103/107): thb_final minor units exceed i64::MAX (> 9.22e18).
#[test]
fn test_overflow_try_minor_thb_final() {
    // 1e15 foreign * 100 rate / 1 unit_size = 1e17 THB = 1e19 minor units (> i64::MAX)
    let quote_line = line(
        "1000000000000000",
        "1.0000",
        "1.0000",
        "100.0000",
        "0.00",
        "1",
        Side::Buy,
    );
    let effect = BenefitEffect {
        new_final_rate: Some(Rate::parse("100.0000").unwrap()),
        new_fee_thb: None,
        amount_thb: Money::ZERO,
    };

    let result = RateFloor::apply(&quote_line, &effect, BenefitClass::Price, None);
    assert_eq!(result, Err(ArithmeticOverflow));
}

/// K-b sites (rate_floor.rs:103/107): thb_base minor units exceed i64::MAX (> 9.22e18).
#[test]
fn test_overflow_try_minor_thb_base() {
    // 1e15 foreign * 100 rate / 1 unit_size = 1e17 THB = 1e19 minor units (> i64::MAX)
    let quote_line = line(
        "1000000000000000",
        "100.0000",
        "1.0000",
        "100.0000",
        "0.00",
        "1",
        Side::Sell,
    );
    let effect = BenefitEffect {
        new_final_rate: Some(Rate::parse("1.0000").unwrap()),
        new_fee_thb: None,
        amount_thb: Money::ZERO,
    };

    let result = RateFloor::apply(&quote_line, &effect, BenefitClass::Price, None);
    assert_eq!(result, Err(ArithmeticOverflow));
}

/// K-b sites (rate_floor.rs:128/147): effect.new_fee_thb minor units exceed i64::MAX.
#[test]
fn test_overflow_try_minor_effect_fee() {
    let quote_line = line(
        "100",
        "35.0000",
        "34.0000",
        "36.0000",
        "10.00",
        "1",
        Side::Buy,
    );
    // 1e19 hundredths exceeds i64::MAX
    let huge_fee = Money::try_from_minor_i128(10_000_000_000_000_000_000i128).unwrap();
    let effect = BenefitEffect {
        new_final_rate: None,
        new_fee_thb: Some(huge_fee),
        amount_thb: Money::ZERO,
    };

    let result = RateFloor::apply(&quote_line, &effect, BenefitClass::Price, None);
    assert_eq!(result, Err(ArithmeticOverflow));
}

/// K-b sites (rate_floor.rs:128/142/153): line.fee_thb minor units exceed i64::MAX.
#[test]
fn test_overflow_try_minor_line_fee() {
    let mut quote_line = line(
        "100",
        "35.0000",
        "34.0000",
        "36.0000",
        "0.00",
        "1",
        Side::Buy,
    );
    // 1e19 hundredths exceeds i64::MAX
    quote_line.fee_thb = Money::try_from_minor_i128(10_000_000_000_000_000_000i128).unwrap();
    let effect = BenefitEffect {
        new_final_rate: None,
        new_fee_thb: Some(Money::ZERO),
        amount_thb: Money::ZERO,
    };

    let result = RateFloor::apply(&quote_line, &effect, BenefitClass::Price, None);
    assert_eq!(result, Err(ArithmeticOverflow));
}

/// K-b site (rate_floor.rs:137): fee.try_minor() with margin guard set.
/// Under M6 (fee.try_minor() -> fee.minor()), fee_val is rebuilt from line fee,
/// so a wrapped fee.minor() slips through uncapped_discount_minor and returns Ok.
#[test]
fn test_overflow_try_minor_effect_fee_with_margin_guard() {
    let quote_line = line(
        "100",
        "35.0000",
        "34.0000",
        "36.0000",
        "1000.00",
        "1",
        Side::Buy,
    );
    let huge_fee = Money::try_from_minor_i128(10_000_000_000_000_000_000i128).unwrap();
    let effect = BenefitEffect {
        new_final_rate: None,
        new_fee_thb: Some(huge_fee),
        amount_thb: Money::ZERO,
    };

    let result = RateFloor::apply(
        &quote_line,
        &effect,
        BenefitClass::Price,
        Some(Decimal::from_str("0.5").unwrap()),
    );
    assert_eq!(result, Err(ArithmeticOverflow));
}
