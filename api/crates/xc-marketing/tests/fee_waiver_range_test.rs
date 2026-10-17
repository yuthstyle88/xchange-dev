//! Calculator-level tests for FEE_WAIVER range validation (ticket #2795).
//!
//! Behaviour table rows R1–R6 and K1–K4:
//! | # | calculation | validate_params (save) | apply (quote) |
//! |---|---|---|---|
//! | R1 | {"percent":"0"} | Err, field percent | Err, field percent |
//! | R2 | {"percent":"-0.5"} | same | same |
//! | R3 | {"percent":"1.01"} | same | same |
//! | R4 | {"amount":"0"} | Err, field amount | Err, field amount |
//! | R5 | {"amount":"-20"} | same | same |
//! | R6 | {"fixed":"-1"} | Err, field fixed | Err, field fixed |
//! | K1 (kept) | {"percent":"1"} / "1.00" | Ok | whole fee waived |
//! | K2 (kept) | {"percent":"0.5"} on fee 50.00 | Ok | waives 25.00 |
//! | K3 (kept) | {"amount":"30.00"} on fee 20.00 | Ok | waives 20.00 (capped at the fee) |
//! | K4 (kept) | malformed ("abc", JSON number, no key) | as #2119 (Err) | as #2119 (Err) |

use std::str::FromStr;

use rust_decimal::Decimal;
use serde_json::json;

use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};
use xc_marketing::domain::benefit::calculator::BenefitCalculator;
use xc_marketing::domain::benefit::kinds::fee_waiver::FeeWaiver;
use xc_marketing::domain::engine::engine::QuoteLineInput;

fn dummy_line(fee_thb: &str) -> QuoteLineInput {
    QuoteLineInput {
        currency: CurrencyCode::parse("USD").unwrap(),
        shop_side: Side::Buy,
        quantity: ForeignAmount::from_decimal(Decimal::from_str("100").unwrap()),
        unit_size: UnitSize::ONE,
        base_rate: Rate::parse("35.00").unwrap(),
        published_buy_rate: Rate::parse("35.00").unwrap(),
        published_sell_rate: Rate::parse("36.00").unwrap(),
        fee_thb: Money::parse(fee_thb).unwrap(),
    }
}

// ── R1: percent 0 ──

#[test]
fn test_r1_percent_zero_refused() {
    let waiver = FeeWaiver;
    let line = dummy_line("50.00");

    for val in &["0", "0.00", "0.0000", "-0"] {
        let calc = json!({ "percent": val });
        let val_err = waiver
            .validate_params(&calc)
            .expect_err("percent 0 must fail validate_params");
        assert_eq!(val_err.field, "percent");
        assert_eq!(
            val_err.reason,
            "FEE_WAIVER percent must be greater than 0 and at most 1"
        );

        let app_err = waiver
            .apply(&line, &calc)
            .expect_err("percent 0 must fail apply");
        assert_eq!(app_err.field, "percent");
        assert_eq!(
            app_err.reason,
            "FEE_WAIVER percent must be greater than 0 and at most 1"
        );
    }
}

// ── R2: percent negative ──

#[test]
fn test_r2_percent_negative_refused() {
    let waiver = FeeWaiver;
    let line = dummy_line("50.00");

    for val in &["-0.5", "-1", "-0.01"] {
        let calc = json!({ "percent": val });
        let val_err = waiver
            .validate_params(&calc)
            .expect_err("negative percent must fail validate_params");
        assert_eq!(val_err.field, "percent");
        assert_eq!(
            val_err.reason,
            "FEE_WAIVER percent must be greater than 0 and at most 1"
        );

        let app_err = waiver
            .apply(&line, &calc)
            .expect_err("negative percent must fail apply");
        assert_eq!(app_err.field, "percent");
        assert_eq!(
            app_err.reason,
            "FEE_WAIVER percent must be greater than 0 and at most 1"
        );
    }
}

// ── R3: percent > 1 ──

#[test]
fn test_r3_percent_greater_than_one_refused() {
    let waiver = FeeWaiver;
    let line = dummy_line("50.00");

    for val in &["1.01", "7", "2.00"] {
        let calc = json!({ "percent": val });
        let val_err = waiver
            .validate_params(&calc)
            .expect_err("percent > 1 must fail validate_params");
        assert_eq!(val_err.field, "percent");
        assert_eq!(
            val_err.reason,
            "FEE_WAIVER percent must be greater than 0 and at most 1"
        );

        let app_err = waiver
            .apply(&line, &calc)
            .expect_err("percent > 1 must fail apply");
        assert_eq!(app_err.field, "percent");
        assert_eq!(
            app_err.reason,
            "FEE_WAIVER percent must be greater than 0 and at most 1"
        );
    }
}

// ── R4: amount 0 ──

#[test]
fn test_r4_amount_zero_refused() {
    let waiver = FeeWaiver;
    let line = dummy_line("50.00");

    for val in &["0", "0.00", "0.0000", "-0"] {
        let calc = json!({ "amount": val });
        let val_err = waiver
            .validate_params(&calc)
            .expect_err("amount 0 must fail validate_params");
        assert_eq!(val_err.field, "amount");
        assert_eq!(val_err.reason, "FEE_WAIVER amount must be greater than 0");

        let app_err = waiver
            .apply(&line, &calc)
            .expect_err("amount 0 must fail apply");
        assert_eq!(app_err.field, "amount");
        assert_eq!(app_err.reason, "FEE_WAIVER amount must be greater than 0");
    }
}

// ── R5: amount negative ──

#[test]
fn test_r5_amount_negative_refused() {
    let waiver = FeeWaiver;
    let line = dummy_line("50.00");

    for val in &["-20", "-0.01", "-1.00"] {
        let calc = json!({ "amount": val });
        let val_err = waiver
            .validate_params(&calc)
            .expect_err("negative amount must fail validate_params");
        assert_eq!(val_err.field, "amount");
        assert_eq!(val_err.reason, "FEE_WAIVER amount must be greater than 0");

        let app_err = waiver
            .apply(&line, &calc)
            .expect_err("negative amount must fail apply");
        assert_eq!(app_err.field, "amount");
        assert_eq!(app_err.reason, "FEE_WAIVER amount must be greater than 0");
    }
}

// ── R6: fixed negative or zero ──

#[test]
fn test_r6_fixed_negative_refused() {
    let waiver = FeeWaiver;
    let line = dummy_line("50.00");

    for val in &["-1", "0", "-20.00"] {
        let calc = json!({ "fixed": val });
        let val_err = waiver
            .validate_params(&calc)
            .expect_err("non-positive fixed must fail validate_params");
        assert_eq!(val_err.field, "fixed");
        assert_eq!(val_err.reason, "FEE_WAIVER amount must be greater than 0");

        let app_err = waiver
            .apply(&line, &calc)
            .expect_err("non-positive fixed must fail apply");
        assert_eq!(app_err.field, "fixed");
        assert_eq!(app_err.reason, "FEE_WAIVER amount must be greater than 0");
    }
}

// ── K1: percent 1 or 1.00 waives whole fee ──

#[test]
fn test_k1_percent_one_waives_whole_fee() {
    let waiver = FeeWaiver;
    let line = dummy_line("100.00");

    for val in &["1", "1.00"] {
        let calc = json!({ "percent": val });
        waiver.validate_params(&calc).expect("percent 1 is valid");
        let effect = waiver.apply(&line, &calc).expect("percent 1 applies");
        assert_eq!(effect.new_fee_thb, Some(Money::ZERO));
        assert_eq!(effect.amount_thb, Money::parse("100.00").unwrap());
    }
}

// ── K2: percent 0.5 on fee 50.00 waives 25.00 ──

#[test]
fn test_k2_percent_half_waives_half_fee() {
    let waiver = FeeWaiver;
    let line = dummy_line("50.00");

    let calc = json!({ "percent": "0.5" });
    waiver.validate_params(&calc).expect("percent 0.5 is valid");
    let effect = waiver.apply(&line, &calc).expect("percent 0.5 applies");
    assert_eq!(effect.new_fee_thb, Some(Money::parse("25.00").unwrap()));
    assert_eq!(effect.amount_thb, Money::parse("25.00").unwrap());
}

// ── K3: amount 30.00 on fee 20.00 waives 20.00 (capped at fee) ──

#[test]
fn test_k3_amount_capped_at_fee() {
    let waiver = FeeWaiver;
    let line = dummy_line("20.00");

    let calc = json!({ "amount": "30.00" });
    waiver
        .validate_params(&calc)
        .expect("amount 30.00 is valid");
    let effect = waiver.apply(&line, &calc).expect("amount 30.00 applies");
    assert_eq!(effect.new_fee_thb, Some(Money::ZERO));
    assert_eq!(effect.amount_thb, Money::parse("20.00").unwrap());
}

// ── K4: malformed params refused ──

#[test]
fn test_k4_malformed_params_refused() {
    let waiver = FeeWaiver;
    let line = dummy_line("50.00");

    // non-decimal string
    let calc = json!({ "percent": "abc" });
    let err = waiver
        .validate_params(&calc)
        .expect_err("abc percent must fail");
    assert_eq!(err.field, "percent");
    let err = waiver
        .apply(&line, &calc)
        .expect_err("abc percent must fail");
    assert_eq!(err.field, "percent");

    // JSON number
    let calc = json!({ "percent": 0.5 });
    let err = waiver
        .validate_params(&calc)
        .expect_err("json number percent must fail");
    assert_eq!(err.field, "percent");
    let err = waiver
        .apply(&line, &calc)
        .expect_err("json number percent must fail");
    assert_eq!(err.field, "percent");

    // missing key
    let calc = json!({});
    let err = waiver
        .validate_params(&calc)
        .expect_err("empty calc must fail");
    assert_eq!(err.field, "calculation");
    let err = waiver
        .apply(&line, &calc)
        .expect_err("empty calc must fail");
    assert_eq!(err.field, "calculation");
}

// ── Edge cases ──

#[test]
fn test_edge_case_small_positive_percent() {
    let waiver = FeeWaiver;
    let line = dummy_line("50.00");

    let calc = json!({ "percent": "0.0001" });
    waiver.validate_params(&calc).expect("0.0001 is valid");
    let effect = waiver.apply(&line, &calc).expect("0.0001 applies");
    // 50.00 * 0.0001 = 0.005 -> 0 satang
    assert_eq!(effect.amount_thb, Money::ZERO);
}

#[test]
fn test_edge_case_both_percent_and_amount_present() {
    let waiver = FeeWaiver;
    let line = dummy_line("50.00");

    // percent wins and is valid; invalid amount is ignored
    let calc = json!({ "percent": "0.5", "amount": "-20" });
    waiver.validate_params(&calc).expect("valid percent wins");
    let effect = waiver.apply(&line, &calc).expect("valid percent wins");
    assert_eq!(effect.amount_thb, Money::parse("25.00").unwrap());

    // percent wins and is invalid -> refused on percent
    let calc = json!({ "percent": "0", "amount": "30.00" });
    let val_err = waiver
        .validate_params(&calc)
        .expect_err("invalid percent refused");
    assert_eq!(val_err.field, "percent");
}
