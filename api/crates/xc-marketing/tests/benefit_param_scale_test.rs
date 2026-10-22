//! Ticket #4738: Benefit params refuse more decimals than their unit holds (THB 2, rate/percent 8).
//!
//! Behaviour table:
//! | Param | Value | Result |
//! |---|---|---|
//! | rate_improvement delta | "0.12345678" (8) | Ok |
//! | rate_improvement delta | "0.123456789" (9) | ParamError { field: "delta", reason: "parameter \"delta\" has more than 8 decimal places" } |
//! | rate_improvement percent | "1.123456789" | ParamError { field: "percent", reason: "parameter \"percent\" has more than 8 decimal places" } |
//! | fee_waiver amount | "10.25" | Ok |
//! | fee_waiver amount | "10.255" | ParamError { field: "amount", reason: "parameter \"amount\" has more than 2 decimal places" } |
//! | fee_waiver fixed | "5.001" | ParamError { field: "fixed", reason: "parameter \"fixed\" has more than 2 decimal places" } |
//! | fee_waiver amount | "50.0000" (trailing zeros) | Ok |
//! | fee_waiver percent | "0.0001" | Ok |

use serde_json::json;
use xc_marketing::domain::benefit::calculator::{BenefitCalculator, ParamError};
use xc_marketing::domain::benefit::kinds::fee_waiver::FeeWaiver;
use xc_marketing::domain::benefit::kinds::rate_improvement::RateImprovement;

#[test]
fn test_rate_improvement_delta_at_max_scale_accepted() {
    let calc = RateImprovement;
    let params = json!({ "delta": "0.12345678" });
    assert!(calc.validate_params(&params).is_ok());
}

#[test]
fn test_rate_improvement_delta_exceeding_scale_refused() {
    let calc = RateImprovement;
    let params = json!({ "delta": "0.123456789" });
    let err = calc.validate_params(&params).unwrap_err();
    assert_eq!(
        err,
        ParamError {
            field: "delta",
            reason: "parameter \"delta\" has more than 8 decimal places".into(),
        }
    );
}

#[test]
fn test_rate_improvement_percent_exceeding_scale_refused() {
    let calc = RateImprovement;
    let params = json!({ "percent": "1.123456789" });
    let err = calc.validate_params(&params).unwrap_err();
    assert_eq!(
        err,
        ParamError {
            field: "percent",
            reason: "parameter \"percent\" has more than 8 decimal places".into(),
        }
    );
}

#[test]
fn test_fee_waiver_amount_at_money_scale_accepted() {
    let calc = FeeWaiver;
    let params = json!({ "amount": "10.25" });
    assert!(calc.validate_params(&params).is_ok());
}

#[test]
fn test_fee_waiver_amount_exceeding_money_scale_refused() {
    let calc = FeeWaiver;
    let params = json!({ "amount": "10.255" });
    let err = calc.validate_params(&params).unwrap_err();
    assert_eq!(
        err,
        ParamError {
            field: "amount",
            reason: "parameter \"amount\" has more than 2 decimal places".into(),
        }
    );
}

#[test]
fn test_fee_waiver_fixed_exceeding_money_scale_refused() {
    let calc = FeeWaiver;
    let params = json!({ "fixed": "5.001" });
    let err = calc.validate_params(&params).unwrap_err();
    assert_eq!(
        err,
        ParamError {
            field: "fixed",
            reason: "parameter \"fixed\" has more than 2 decimal places".into(),
        }
    );
}

#[test]
fn test_fee_waiver_amount_trailing_zeros_accepted() {
    let calc = FeeWaiver;
    let params = json!({ "amount": "50.0000" });
    assert!(calc.validate_params(&params).is_ok());
}

#[test]
fn test_fee_waiver_percent_scale_accepted() {
    let calc = FeeWaiver;
    let params = json!({ "percent": "0.0001" });
    assert!(calc.validate_params(&params).is_ok());
}
