//! Up-front tests: a stored benefit rule with malformed params is refused when applied (ticket B).
//!
//! Design `docs/design/blueprint/marketing-campaigns.md` §4.1 (`calculation` = "the calculator's
//! params, decimals as strings"), D-6 (`BenefitCalculator::apply(line, params)`, one file per kind).
//!
//! Written before the fix by the test writer; the implementer must not edit, delete or weaken them.
//!
//! Today `FeeWaiver::apply` and `RateImprovement::apply` read a malformed or missing param as 0
//! (`fee_waiver.rs:25,29,41,45,48`, `rate_improvement.rs:25,29,33,37,40`), so a quote silently
//! carries a 0 % / 0 THB / 0-rate benefit as if the campaign applied.
//!
//! Refusal `[proposed]`: the bad value is stored data, not the caller's input, so `POST /api/quotes`
//! answers **500 `serverError`** and writes no quote and no reservation. This is the same answer the
//! quote already gives for a stored rule whose `eligibility` or `caps` do not decode
//! (`pg_campaign_repository.rs` `AppError::Internal("failed to decode …")` → `quotes.rs` `serverError`).
//! A 400 would tell the teller their request was wrong; it was not.
//!
//! The rows are written with SQL on purpose: ticket A refuses them at save, and these tests pin the
//! reader for rows that bypassed it (older rows, direct writes).
//!
//! Behaviour rows (one test per reader site, so a restored fallback turns exactly one test red):
//! | stored `calculation` | site | answer |
//! |---|---|---|
//! | FEE_WAIVER `{"percent":"abc"}` | fee_waiver.rs:25 | 500 `serverError`, no quote, no reservation |
//! | FEE_WAIVER `{"percent":true}` | fee_waiver.rs:29 | same |
//! | FEE_WAIVER `{"amount":"abc"}` | fee_waiver.rs:41 | same |
//! | FEE_WAIVER `{"fixed":{}}` | fee_waiver.rs:45 | same |
//! | FEE_WAIVER `{"pct":"0.5"}` (no known key) | fee_waiver.rs:48-49 | same |
//! | RATE_IMPROVEMENT `{"delta":"abc"}` | rate_improvement.rs:25 | same |
//! | RATE_IMPROVEMENT `{"delta":true}` | rate_improvement.rs:29 | same |
//! | RATE_IMPROVEMENT `{"percent":"abc"}` | rate_improvement.rs:33 | same |
//! | RATE_IMPROVEMENT `{"percent":[]}` | rate_improvement.rs:37 | same |
//! | RATE_IMPROVEMENT `{"type":"FIXED_RATE"}` (no known key) | rate_improvement.rs:40-41 | same |
//! | FEE_WAIVER `{"percent":0.5}` (JSON number) | fee_waiver.rs:29 (`to_string` path) | same |
//! | FEE_WAIVER `{"amount":30}` (JSON integer) | fee_waiver.rs:42-43 (`as_i64` path) | same |
//! | FEE_WAIVER `{"fixed":25.5}` (JSON number) | fee_waiver.rs:45 (`to_string` path) | same |
//! | RATE_IMPROVEMENT `{"delta":1}` (JSON integer) | rate_improvement.rs:26-27 (`as_i64` path) | same |
//! | RATE_IMPROVEMENT `{"percent":0.01}` (JSON number) | rate_improvement.rs:37 (`to_string` path) | same |
//! | `benefit_type` the registry does not know (`CASHBACK`) | engine.rs:175 (`if let Some(calc)` skips it) | same |
//! | each kind/key well formed (decimal strings only) | — | 200, the benefit applied with the right amount (keep-pin) |
//!
//! Controller rulings 2026-10-16: R3 a JSON number is malformed (design §4.1 "decimals as strings");
//! R4 a stored rule whose `benefit_type` is not in the registry is refused, not skipped.

mod campaign_common;
mod common;
mod quote_common;

use actix_web::http::StatusCode;
use campaign_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};

/// The stored rule's params are malformed: the quote must be refused, not priced with a 0 benefit.
async fn refused(benefit_type: &str, calculation: Value) {
    let (db, _rule) = room_with_rule(benefit_type, calculation.clone()).await;

    let ans = quote(&db, "100.00").await;

    assert_eq!(
        ans.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a stored {benefit_type} rule with calculation {calculation} must not be applied as 0: {}",
        ans.text
    );
    assert_eq!(ans.code(), "serverError", "{}", ans.text);
    assert_eq!(db.quote_rows().await, (0, 0), "no quote is written");
    assert_eq!(reservations(&db).await, 0, "no budget is reserved");
}

/// The stored rule's params are well formed: the quote applies exactly that benefit.
async fn applied(
    benefit_type: &str,
    calculation: Value,
    final_rate: &str,
    fee_thb: &str,
    amount_thb: &str,
) {
    let (db, rule_id) = room_with_rule(benefit_type, calculation.clone()).await;

    let ans = quote(&db, "100.00").await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "{benefit_type} {calculation}: {}",
        ans.text
    );
    let line = &ans.body["lines"][0];
    assert_eq!(line["baseRate"], "32.0000", "{}", ans.text);
    assert_eq!(line["finalRate"], final_rate, "{}", ans.text);
    assert_eq!(line["feeThb"], fee_thb, "{}", ans.text);
    let benefits = ans.body["benefits"].as_array().expect("benefits array");
    assert_eq!(benefits.len(), 1, "{}", ans.text);
    assert_eq!(
        benefits[0]["ruleId"],
        json!(rule_id.to_string()),
        "{}",
        ans.text
    );
    let amount = benefits[0]["amountThb"]
        .as_str()
        .map(|s| s.parse::<Decimal>().expect("decimal amountThb"));
    assert_eq!(
        amount,
        Some(amount_thb.parse::<Decimal>().expect("decimal")),
        "{}",
        ans.text
    );
    assert_eq!(db.quote_rows().await, (1, 1));
    assert_eq!(reservations(&db).await, 1);
}

// ── FEE_WAIVER refusals ──

#[tokio::test]
async fn fee_waiver_percent_not_a_decimal_string_refuses_the_quote() {
    refused("FEE_WAIVER", json!({ "percent": "abc" })).await;
}

#[tokio::test]
async fn fee_waiver_percent_not_a_string_refuses_the_quote() {
    refused("FEE_WAIVER", json!({ "percent": true })).await;
}

#[tokio::test]
async fn fee_waiver_amount_not_a_decimal_string_refuses_the_quote() {
    refused("FEE_WAIVER", json!({ "amount": "abc" })).await;
}

#[tokio::test]
async fn fee_waiver_fixed_not_a_string_refuses_the_quote() {
    refused("FEE_WAIVER", json!({ "fixed": {} })).await;
}

#[tokio::test]
async fn fee_waiver_without_a_known_key_refuses_the_quote() {
    refused("FEE_WAIVER", json!({ "pct": "0.5" })).await;
}

// ── RATE_IMPROVEMENT refusals ──

#[tokio::test]
async fn rate_improvement_delta_not_a_decimal_string_refuses_the_quote() {
    refused("RATE_IMPROVEMENT", json!({ "delta": "abc" })).await;
}

#[tokio::test]
async fn rate_improvement_delta_not_a_string_refuses_the_quote() {
    refused("RATE_IMPROVEMENT", json!({ "delta": true })).await;
}

#[tokio::test]
async fn rate_improvement_percent_not_a_decimal_string_refuses_the_quote() {
    refused("RATE_IMPROVEMENT", json!({ "percent": "abc" })).await;
}

#[tokio::test]
async fn rate_improvement_percent_not_a_string_refuses_the_quote() {
    refused("RATE_IMPROVEMENT", json!({ "percent": [] })).await;
}

#[tokio::test]
async fn rate_improvement_without_a_known_key_refuses_the_quote() {
    refused("RATE_IMPROVEMENT", json!({ "type": "FIXED_RATE" })).await;
}

// ── JSON numbers are malformed (R3: decimals are stored as strings) ──

#[tokio::test]
async fn fee_waiver_percent_as_json_number_refuses_the_quote() {
    refused("FEE_WAIVER", json!({ "percent": 0.5 })).await;
}

#[tokio::test]
async fn fee_waiver_amount_as_json_integer_refuses_the_quote() {
    refused("FEE_WAIVER", json!({ "amount": 30 })).await;
}

#[tokio::test]
async fn fee_waiver_fixed_as_json_number_refuses_the_quote() {
    refused("FEE_WAIVER", json!({ "fixed": 25.5 })).await;
}

#[tokio::test]
async fn rate_improvement_delta_as_json_integer_refuses_the_quote() {
    refused("RATE_IMPROVEMENT", json!({ "delta": 1 })).await;
}

#[tokio::test]
async fn rate_improvement_percent_as_json_number_refuses_the_quote() {
    refused("RATE_IMPROVEMENT", json!({ "percent": 0.01 })).await;
}

// ── a stored kind the registry does not know (R4) ──

#[tokio::test]
async fn unknown_stored_benefit_type_refuses_the_quote() {
    refused("CASHBACK", json!({ "delta": "0.50" })).await;
}

// ── keep-pins: well-formed params still apply the right amount ──

#[tokio::test]
async fn fee_waiver_percent_well_formed_waives_that_share_of_the_fee() {
    applied(
        "FEE_WAIVER",
        json!({ "percent": "0.5" }),
        "32.0000",
        "50.00",
        "50.00",
    )
    .await;
}

#[tokio::test]
async fn fee_waiver_amount_well_formed_waives_that_amount() {
    applied(
        "FEE_WAIVER",
        json!({ "amount": "30.00" }),
        "32.0000",
        "70.00",
        "30.00",
    )
    .await;
}

#[tokio::test]
async fn fee_waiver_fixed_well_formed_waives_that_amount() {
    applied(
        "FEE_WAIVER",
        json!({ "fixed": "25" }),
        "32.0000",
        "75.00",
        "25.00",
    )
    .await;
}

#[tokio::test]
async fn rate_improvement_delta_well_formed_moves_the_rate() {
    applied(
        "RATE_IMPROVEMENT",
        json!({ "type": "FIXED_RATE", "delta": "0.50" }),
        "32.5000",
        "100.00",
        "50.00",
    )
    .await;
}

#[tokio::test]
async fn rate_improvement_percent_well_formed_moves_the_rate_by_that_share() {
    applied(
        "RATE_IMPROVEMENT",
        json!({ "percent": "0.01" }),
        "32.3200",
        "100.00",
        "32.00",
    )
    .await;
}
