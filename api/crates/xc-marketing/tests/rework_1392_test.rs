//! Rework tests for MK-2 must-fixes (#1392, PR 1466):
//! 1. Margin guard must limit FEE_WAIVER benefits (all PRICE benefits).
//! 2. Guard-capped or percent-improved rates with >8 decimals must be rounded before Rate::parse,
//!    never falling back to base_rate with amountThb 0.00.

use std::str::FromStr;

use rust_decimal::Decimal;
use serde_json::json;

use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};
use xc_marketing::domain::benefit::calculator::BenefitCalculator;
use xc_marketing::domain::benefit::kinds::fee_waiver::FeeWaiver;
use xc_marketing::domain::benefit::kinds::rate_improvement::RateImprovement;
use xc_marketing::domain::benefit::rule::BenefitClass;
use xc_marketing::domain::engine::engine::QuoteLineInput;
use xc_marketing::domain::engine::rate_floor::RateFloor;

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
}

fn rate(s: &str) -> Rate {
    Rate::parse(s).unwrap()
}

fn money(s: &str) -> Money {
    Money::parse(s).unwrap()
}

fn fa(s: &str) -> ForeignAmount {
    ForeignAmount::from_decimal(Decimal::from_str(s).unwrap())
}

/// Must-fix 1: Margin guard limits FEE_WAIVER benefits.
/// BUY 100 USD, unit 1, base 35.00, published 35.00/36.00 (allowance = share * 100 THB), fee 200.00 THB.
/// 100% fee waiver with max_spread_share = 0 must give away 0 THB (fee_after = 200.00 THB).
/// With max_spread_share = 0.5, it must give away at most 50.00 THB (fee_after = 150.00 THB).
#[test]
fn test_margin_guard_limits_fee_waiver() {
    let line = QuoteLineInput {
        currency: usd(),
        shop_side: Side::Buy,
        quantity: fa("100"),
        unit_size: UnitSize::ONE,
        base_rate: rate("35.00"),
        published_buy_rate: rate("35.00"),
        published_sell_rate: rate("36.00"),
        fee_thb: money("200.00"),
    };

    let waiver = FeeWaiver;
    let effect = waiver
        .apply(&line, &json!({ "percent": "1" }))
        .expect("valid params");
    assert_eq!(effect.new_fee_thb, Some(Money::ZERO));

    // Case A: share = 0 -> allowance = 0 THB. fee_after must stay 200.00 THB, amount_thb = 0.00 THB.
    let res_zero = RateFloor::apply(&line, &effect, BenefitClass::Price, Some(Decimal::ZERO))
        .expect("in range");
    assert_eq!(
        res_zero.fee_thb,
        money("200.00"),
        "with share=0, fee waiver must not give away any fee"
    );
    assert_eq!(
        res_zero.amount_thb,
        money("0.00"),
        "with share=0, amount given away must be 0"
    );
    assert!(
        res_zero.guard_capped,
        "guard_capped must be true when waiver is restricted by margin guard"
    );

    // Case B: share = 0.5 -> allowance = 50.00 THB. fee_after must be 150.00 THB, amount_thb = 50.00 THB.
    let res_half = RateFloor::apply(
        &line,
        &effect,
        BenefitClass::Price,
        Some(Decimal::from_str("0.5").unwrap()),
    )
    .expect("in range");
    assert_eq!(
        res_half.fee_thb,
        money("150.00"),
        "with share=0.5, fee after waiver must be 150.00 THB"
    );
    assert_eq!(
        res_half.amount_thb,
        money("50.00"),
        "with share=0.5, amount given away must be capped at 50.00 THB"
    );
    assert!(
        res_half.guard_capped,
        "guard_capped must be true when waiver is restricted by margin guard"
    );
}

/// Must-fix 2: Guard-capped and percent-improved rates with >8 decimals must be rounded before Rate::parse.
#[test]
fn test_rate_improvement_8_decimals_not_dropped() {
    // Case A: Guard-capped rate from 8-decimal published rate:
    // base 35.12345678, published 35.12345678 / 36.12345679, delta 0.8, share 0.5
    // spread = 1.00000001, spread * 0.5 = 0.500000005, guard = 35.623456785 (9 decimals)
    let line = QuoteLineInput {
        currency: usd(),
        shop_side: Side::Buy,
        quantity: fa("100"),
        unit_size: UnitSize::ONE,
        base_rate: rate("35.12345678"),
        published_buy_rate: rate("35.12345678"),
        published_sell_rate: rate("36.12345679"),
        fee_thb: money("0.00"),
    };

    let imp = RateImprovement;
    let effect = imp
        .apply(&line, &json!({ "delta": "0.8" }))
        .expect("valid params");
    assert!(effect.new_final_rate.is_some());

    let res = RateFloor::apply(
        &line,
        &effect,
        BenefitClass::Price,
        Some(Decimal::from_str("0.5").unwrap()),
    )
    .expect("in range");
    assert_ne!(
        res.final_rate, line.base_rate,
        "guard-capped rate must not fall back to base rate"
    );
    assert_eq!(
        res.final_rate,
        rate("35.62345679"),
        "effective rate must round to 8 decimals with half away from zero"
    );
    assert!(
        !res.amount_thb.is_zero(),
        "amount_thb must not be zero when rule is applied"
    );
    assert!(res.guard_capped, "guard_capped must be true");

    // Case B: Percentage rate improvement from 8-decimal published rate:
    // base 35.12345678, percent = "0.01" (1%) -> delta = 0.3512345678 (10 decimals)
    let effect_pct = imp
        .apply(&line, &json!({ "percent": "0.01" }))
        .expect("valid params");
    assert!(
        effect_pct.new_final_rate.is_some(),
        "percent improvement on 8-decimal rate must not return None"
    );
    assert_eq!(
        effect_pct.new_final_rate.unwrap(),
        rate("35.47469135"),
        "percent rate must be rounded to 8 decimals half away from zero"
    );
    assert!(
        !effect_pct.amount_thb.is_zero(),
        "amount_thb must be positive for valid percent improvement"
    );
}
