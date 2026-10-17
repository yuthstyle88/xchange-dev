//! Up-front tests: `campaignVersionSave` refuses malformed benefit-rule params (ticket A).
//!
//! Design `docs/design/blueprint/marketing-campaigns.md`:
//! - §4.5 `campaignVersionSave` (DRAFT only: dates, branches, budget, caps, stack, priority, **rules**, snapshot);
//! - §4.1 `benefit_rules.benefit_type` "validated against the registry", `calculation` "the calculator's
//!   params, decimals as strings";
//! - D-6 benefit types are one trait (`BenefitCalculator`), one file per type (♻️).
//!
//! Written before the fix by the test writer; the implementer must not edit, delete or weaken them.
//!
//! Request shape `[proposed]`: `rules` is an optional array on the save body. Each item is
//! `{ benefitType, eligibility, calculation, caps, requiresCoupon }` (the write half of the
//! `campaign` view's `CampaignRuleDetail`). `benefit_class` comes from the calculator kind,
//! not from the request. `rules` present ⇒ it replaces the draft's rules in order (`rule_no` 1..n).
//! `rules` absent ⇒ the stored rules are untouched.
//!
//! Field names `[proposed]`: `V3Error.field` is `&'static str`, so the field is the path
//! without the index (`rules.calculation.percent`). The message names the index (`rules[1]`).
//!
//! Behaviour rows:
//! | case | answer |
//! |---|---|
//! | FEE_WAIVER `percent` not a decimal string ("abc") | 400 `validationFailed`, `rules.calculation.percent`, nothing written |
//! | FEE_WAIVER `percent` not a string (`true`) | same, `rules.calculation.percent` |
//! | FEE_WAIVER `amount` not a decimal string | same, `rules.calculation.amount` |
//! | FEE_WAIVER `fixed` not a string (`{}`) | same, `rules.calculation.fixed` |
//! | FEE_WAIVER with none of `percent`/`amount`/`fixed` | same, `rules.calculation` |
//! | RATE_IMPROVEMENT `delta` "abc" / `true` | same, `rules.calculation.delta` |
//! | RATE_IMPROVEMENT `percent` "abc" / `true` | same, `rules.calculation.percent` |
//! | RATE_IMPROVEMENT with neither `delta` nor `percent` | same, `rules.calculation` |
//! | a param given as a JSON number, not a decimal string (ruling R3) | same, `rules.calculation.<key>` |
//! | `benefitType` the registry does not know | same, `rules.benefitType` |
//! | the 2nd rule malformed | the message names `rules[1]`; the 1st rule is not written either |
//! | each well-formed kind/key | 200, rule stored with that calculation and the kind's class |
//! | no `rules` in the body | 200, the stored rules untouched (as today) |

mod campaign_common;
mod common;
mod quote_common;

use std::str::FromStr;

use actix_web::http::StatusCode;
use campaign_common::*;
use quote_common::TestDb;
use rust_decimal::Decimal;
use serde_json::{Value, json};

/// Saves one malformed rule and expects 400 `validationFailed` on `field`, with nothing written:
/// the seeded rule is still the only one, and the version's budget is unchanged.
async fn refused(benefit_type: &str, calculation: Value, field: &str) {
    let room = TestDb::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        &body_with(Some(vec![rule(benefit_type, calculation.clone())])),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "{benefit_type} {calculation} must be refused with 400: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "{}", ans.text);
    assert_eq!(ans.field(), field, "{}", ans.text);
    assert_eq!(
        stored_rules(&room.pool, version_id).await,
        vec![(
            "RATE_IMPROVEMENT".to_owned(),
            "PRICE".to_owned(),
            serde_json::from_str::<Value>(SEEDED_CALC).expect("seed calc"),
            1
        )],
        "a refused save must not replace or add rules"
    );
    assert_eq!(
        stored_budget(&room.pool, version_id).await,
        Decimal::from_str(SEEDED_BUDGET).expect("decimal"),
        "a refused save must not write the version's terms either"
    );

    room.finish().await;
}

/// Saves one well-formed rule and expects 200 with exactly that rule stored as `rule_no` 1.
async fn stored(benefit_type: &str, calculation: Value) {
    let room = TestDb::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        &body_with(Some(vec![rule(benefit_type, calculation.clone())])),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "{benefit_type} {calculation} is well formed and must save: {}",
        ans.text
    );
    assert_eq!(
        stored_rules(&room.pool, version_id).await,
        vec![(benefit_type.to_owned(), "PRICE".to_owned(), calculation, 1)],
        "the saved rules replace the draft's rules, class taken from the calculator kind"
    );

    room.finish().await;
}

// ── FEE_WAIVER refusals (reader sites fee_waiver.rs:25, :29, :41, :45, :48) ──

#[actix_web::test]
async fn fee_waiver_percent_not_a_decimal_string_is_refused() {
    refused(
        "FEE_WAIVER",
        json!({ "percent": "abc" }),
        "rules.calculation.percent",
    )
    .await;
}

#[actix_web::test]
async fn fee_waiver_percent_not_a_string_is_refused() {
    refused(
        "FEE_WAIVER",
        json!({ "percent": true }),
        "rules.calculation.percent",
    )
    .await;
}

#[actix_web::test]
async fn fee_waiver_amount_not_a_decimal_string_is_refused() {
    refused(
        "FEE_WAIVER",
        json!({ "amount": "abc" }),
        "rules.calculation.amount",
    )
    .await;
}

#[actix_web::test]
async fn fee_waiver_fixed_not_a_string_is_refused() {
    refused(
        "FEE_WAIVER",
        json!({ "fixed": {} }),
        "rules.calculation.fixed",
    )
    .await;
}

#[actix_web::test]
async fn fee_waiver_without_percent_amount_or_fixed_is_refused() {
    refused("FEE_WAIVER", json!({ "pct": "0.5" }), "rules.calculation").await;
}

// ── RATE_IMPROVEMENT refusals (reader sites rate_improvement.rs:25, :29, :33, :37, :40) ──

#[actix_web::test]
async fn rate_improvement_delta_not_a_decimal_string_is_refused() {
    refused(
        "RATE_IMPROVEMENT",
        json!({ "delta": "abc" }),
        "rules.calculation.delta",
    )
    .await;
}

#[actix_web::test]
async fn rate_improvement_delta_not_a_string_is_refused() {
    refused(
        "RATE_IMPROVEMENT",
        json!({ "delta": true }),
        "rules.calculation.delta",
    )
    .await;
}

#[actix_web::test]
async fn rate_improvement_percent_not_a_decimal_string_is_refused() {
    refused(
        "RATE_IMPROVEMENT",
        json!({ "percent": "abc" }),
        "rules.calculation.percent",
    )
    .await;
}

#[actix_web::test]
async fn rate_improvement_percent_not_a_string_is_refused() {
    refused(
        "RATE_IMPROVEMENT",
        json!({ "percent": [] }),
        "rules.calculation.percent",
    )
    .await;
}

#[actix_web::test]
async fn rate_improvement_without_delta_or_percent_is_refused() {
    refused(
        "RATE_IMPROVEMENT",
        json!({ "type": "FIXED_RATE" }),
        "rules.calculation",
    )
    .await;
}

// ── JSON numbers are malformed (ruling R3: design §4.1 "decimals as strings") ──

#[actix_web::test]
async fn fee_waiver_percent_as_json_number_is_refused() {
    refused(
        "FEE_WAIVER",
        json!({ "percent": 0.5 }),
        "rules.calculation.percent",
    )
    .await;
}

#[actix_web::test]
async fn fee_waiver_amount_as_json_integer_is_refused() {
    refused(
        "FEE_WAIVER",
        json!({ "amount": 30 }),
        "rules.calculation.amount",
    )
    .await;
}

#[actix_web::test]
async fn fee_waiver_fixed_as_json_number_is_refused() {
    refused(
        "FEE_WAIVER",
        json!({ "fixed": 25.5 }),
        "rules.calculation.fixed",
    )
    .await;
}

#[actix_web::test]
async fn rate_improvement_delta_as_json_integer_is_refused() {
    refused(
        "RATE_IMPROVEMENT",
        json!({ "delta": 1 }),
        "rules.calculation.delta",
    )
    .await;
}

#[actix_web::test]
async fn rate_improvement_percent_as_json_number_is_refused() {
    refused(
        "RATE_IMPROVEMENT",
        json!({ "percent": 0.01 }),
        "rules.calculation.percent",
    )
    .await;
}

// ── registry and position ──

#[actix_web::test]
async fn unknown_benefit_type_is_refused() {
    refused("CASHBACK", json!({ "delta": "0.10" }), "rules.benefitType").await;
}

#[actix_web::test]
async fn second_rule_malformed_names_its_index_and_writes_neither_rule() {
    let room = TestDb::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        &body_with(Some(vec![
            rule("FEE_WAIVER", json!({ "percent": "0.5" })),
            rule("RATE_IMPROVEMENT", json!({ "delta": "abc" })),
        ])),
    )
    .await;

    assert_eq!(ans.status, StatusCode::BAD_REQUEST, "{}", ans.text);
    assert_eq!(ans.code(), "validationFailed", "{}", ans.text);
    assert_eq!(ans.field(), "rules.calculation.delta", "{}", ans.text);
    assert!(
        ans.message().contains("rules[1]"),
        "the message must name the failing rule's index: {}",
        ans.text
    );
    let rules = stored_rules(&room.pool, version_id).await;
    assert_eq!(rules.len(), 1, "only the seeded rule remains: {rules:?}");
    assert_eq!(rules[0].0, "RATE_IMPROVEMENT", "{rules:?}");

    room.finish().await;
}

// ── well-formed params are stored (new behaviour: on main the `rules` field is dropped) ──

#[actix_web::test]
async fn fee_waiver_percent_well_formed_is_stored() {
    stored("FEE_WAIVER", json!({ "percent": "0.5" })).await;
}

#[actix_web::test]
async fn fee_waiver_amount_well_formed_is_stored() {
    stored("FEE_WAIVER", json!({ "amount": "30.00" })).await;
}

#[actix_web::test]
async fn fee_waiver_fixed_well_formed_is_stored() {
    stored("FEE_WAIVER", json!({ "fixed": "30" })).await;
}

#[actix_web::test]
async fn rate_improvement_delta_well_formed_is_stored() {
    stored(
        "RATE_IMPROVEMENT",
        json!({ "type": "FIXED_RATE", "delta": "0.25" }),
    )
    .await;
}

#[actix_web::test]
async fn rate_improvement_percent_well_formed_is_stored() {
    stored("RATE_IMPROVEMENT", json!({ "percent": "0.01" })).await;
}

#[actix_web::test]
async fn two_well_formed_rules_are_stored_in_order() {
    let room = TestDb::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        &body_with(Some(vec![
            rule("FEE_WAIVER", json!({ "percent": "1" })),
            rule("RATE_IMPROVEMENT", json!({ "delta": "0.20" })),
        ])),
    )
    .await;

    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(
        stored_rules(&room.pool, version_id).await,
        vec![
            (
                "FEE_WAIVER".to_owned(),
                "PRICE".to_owned(),
                json!({ "percent": "1" }),
                1
            ),
            (
                "RATE_IMPROVEMENT".to_owned(),
                "PRICE".to_owned(),
                json!({ "delta": "0.20" }),
                2
            ),
        ]
    );

    room.finish().await;
}

// ── keep-pin: a save without `rules` behaves as today ──

#[actix_web::test]
async fn save_without_rules_keeps_the_stored_rules() {
    let room = TestDb::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(&room, campaign_id, &body_with(None)).await;

    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(ans.body["version"], 1, "{}", ans.text);
    assert_eq!(
        stored_rules(&room.pool, version_id).await,
        vec![(
            "RATE_IMPROVEMENT".to_owned(),
            "PRICE".to_owned(),
            serde_json::from_str::<Value>(SEEDED_CALC).expect("seed calc"),
            1
        )]
    );
    assert_eq!(
        stored_budget(&room.pool, version_id).await,
        Decimal::from_str("20000.00").expect("decimal"),
        "the terms are saved"
    );

    room.finish().await;
}
