//! #2796: RATE_IMPROVEMENT rule range validation tests.
//!
//! Behaviour table (§8):
//! R1: delta = 0 -> refused (field "delta")
//! R2: delta < 0 -> refused (field "delta")
//! R3: percent = 0 -> refused (field "percent")
//! R4: percent < 0 -> refused (field "percent")
//! R5: percent = 1 -> refused (field "percent")
//! R6: percent > 1 -> refused (field "percent")
//! K1: valid delta -> accepted
//! K2: valid percent -> accepted
//! K3: malformed -> refused

use rust_decimal::Decimal;
use serde_json::json;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};
use xc_marketing::domain::benefit::calculator::BenefitCalculator;
use xc_marketing::domain::benefit::kinds::rate_improvement::RateImprovement;
use xc_marketing::domain::engine::engine::QuoteLineInput;

fn dummy_line(side: Side) -> QuoteLineInput {
    QuoteLineInput {
        currency: CurrencyCode::parse("USD").unwrap(),
        shop_side: side,
        quantity: ForeignAmount::from_decimal(Decimal::from(100)),
        unit_size: UnitSize::parse("1").unwrap(),
        base_rate: Rate::parse("35.00").unwrap(),
        published_buy_rate: Rate::parse("35.00").unwrap(),
        published_sell_rate: Rate::parse("36.00").unwrap(),
        fee_thb: Money::ZERO,
    }
}

#[test]
fn test_r1_delta_zero_refused() {
    let calc = RateImprovement;
    let params = json!({ "delta": "0" });
    let val_err = calc.validate_params(&params).unwrap_err();
    assert_eq!(val_err.field, "delta");

    let line = dummy_line(Side::Buy);
    let apply_err = calc.apply(&line, &params).unwrap_err();
    assert_eq!(apply_err.field, "delta");
}

#[test]
fn test_r2_delta_negative_refused() {
    let calc = RateImprovement;
    let params = json!({ "delta": "-0.10" });
    let val_err = calc.validate_params(&params).unwrap_err();
    assert_eq!(val_err.field, "delta");

    let line = dummy_line(Side::Buy);
    let apply_err = calc.apply(&line, &params).unwrap_err();
    assert_eq!(apply_err.field, "delta");
}

#[test]
fn test_r3_percent_zero_refused() {
    let calc = RateImprovement;
    let params = json!({ "percent": "0" });
    let val_err = calc.validate_params(&params).unwrap_err();
    assert_eq!(val_err.field, "percent");

    let line = dummy_line(Side::Buy);
    let apply_err = calc.apply(&line, &params).unwrap_err();
    assert_eq!(apply_err.field, "percent");
}

#[test]
fn test_r4_percent_negative_refused() {
    let calc = RateImprovement;
    let params = json!({ "percent": "-0.01" });
    let val_err = calc.validate_params(&params).unwrap_err();
    assert_eq!(val_err.field, "percent");

    let line = dummy_line(Side::Buy);
    let apply_err = calc.apply(&line, &params).unwrap_err();
    assert_eq!(apply_err.field, "percent");
}

#[test]
fn test_r5_percent_one_refused() {
    let calc = RateImprovement;
    let params = json!({ "percent": "1" });
    let val_err = calc.validate_params(&params).unwrap_err();
    assert_eq!(val_err.field, "percent");

    let line = dummy_line(Side::Buy);
    let apply_err = calc.apply(&line, &params).unwrap_err();
    assert_eq!(apply_err.field, "percent");

    let params_100 = json!({ "percent": "1.00" });
    let val_err_100 = calc.validate_params(&params_100).unwrap_err();
    assert_eq!(val_err_100.field, "percent");
}

#[test]
fn test_r6_percent_greater_than_one_refused() {
    let calc = RateImprovement;
    let params = json!({ "percent": "1.5" });
    let val_err = calc.validate_params(&params).unwrap_err();
    assert_eq!(val_err.field, "percent");

    let line = dummy_line(Side::Buy);
    let apply_err = calc.apply(&line, &params).unwrap_err();
    assert_eq!(apply_err.field, "percent");
}

#[test]
fn test_k1_valid_delta_accepted() {
    let calc = RateImprovement;
    let params = json!({ "delta": "0.10" });
    assert!(calc.validate_params(&params).is_ok());

    let buy_line = dummy_line(Side::Buy);
    let buy_effect = calc.apply(&buy_line, &params).unwrap();
    assert_eq!(
        buy_effect.new_final_rate,
        Some(Rate::parse("35.10").unwrap())
    );
    assert!(!buy_effect.amount_thb.is_zero());

    let sell_line = dummy_line(Side::Sell);
    let sell_effect = calc.apply(&sell_line, &params).unwrap();
    assert_eq!(
        sell_effect.new_final_rate,
        Some(Rate::parse("34.90").unwrap())
    );
    assert!(!sell_effect.amount_thb.is_zero());
}

#[test]
fn test_k2_valid_percent_accepted() {
    let calc = RateImprovement;
    let params = json!({ "percent": "0.01" });
    assert!(calc.validate_params(&params).is_ok());

    let line = dummy_line(Side::Buy);
    let effect = calc.apply(&line, &params).unwrap();
    assert!(effect.new_final_rate.is_some());
    assert!(!effect.amount_thb.is_zero());
}

#[test]
fn test_k3_malformed_params_refused() {
    let calc = RateImprovement;

    // String not a decimal
    let params_abc = json!({ "delta": "abc" });
    let err_abc = calc.validate_params(&params_abc).unwrap_err();
    assert_eq!(err_abc.field, "delta");

    // JSON number not string
    let params_num = json!({ "delta": 0.1 });
    let err_num = calc.validate_params(&params_num).unwrap_err();
    assert_eq!(err_num.field, "delta");

    // No known key
    let params_none = json!({ "type": "FIXED" });
    let err_none = calc.validate_params(&params_none).unwrap_err();
    assert_eq!(err_none.field, "calculation");
}

#[test]
fn test_edge_percent_just_below_one_accepted() {
    let calc = RateImprovement;
    let params = json!({ "percent": "0.99999999" });
    assert!(calc.validate_params(&params).is_ok());

    let line = dummy_line(Side::Buy);
    let effect = calc.apply(&line, &params).unwrap();
    assert!(effect.new_final_rate.is_some());
}

#[test]
fn test_edge_both_keys_delta_wins() {
    let calc = RateImprovement;
    // delta is valid ("0.10"), percent is invalid ("-1")
    let params = json!({ "delta": "0.10", "percent": "-1" });
    assert!(calc.validate_params(&params).is_ok());

    let line = dummy_line(Side::Buy);
    let effect = calc.apply(&line, &params).unwrap();
    assert_eq!(effect.new_final_rate, Some(Rate::parse("35.10").unwrap()));
}
