//! Integration route tests for FEE_WAIVER range validation (ticket #2795).
//!
//! Q1: quote 500 for a stored out-of-range rule, written with SQL as campaign_benefit_params_reader_test.rs does.
//! S1: PUT /api/campaigns/{id}/draft answers 400 validationFailed, field rules.calculation.percent, nothing written.
//! S2: the same for rules.calculation.amount.

mod campaign_common;
mod common;
mod quote_common;

use std::str::FromStr;

use actix_web::http::StatusCode;
use campaign_common::*;
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};

// ────────────────────────────────────────────────────────────────────────────
// Q1: Quote 500 for stored out-of-range rule
// ────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_q1_quote_stored_out_of_range_fee_waiver_refused_500() {
    let (db, _rule) = room_with_rule("FEE_WAIVER", json!({ "percent": "0" })).await;

    let ans = quote(&db, "100.00").await;

    assert_eq!(
        ans.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "stored FEE_WAIVER with percent 0 must return 500 serverError: {}",
        ans.text
    );
    assert_eq!(ans.code(), "serverError", "{}", ans.text);
    assert_eq!(db.quote_rows().await, (0, 0), "no quote is written");
    assert_eq!(reservations(&db).await, 0, "no budget is reserved");

    db.finish().await;
}

// ────────────────────────────────────────────────────────────────────────────
// S1 & S2: Draft save refuses out-of-range percent and amount
// ────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_s1_draft_save_out_of_range_percent_refused() {
    let db = TestDb::start().await;
    let (campaign_id, version_id) = seed_draft(&db.pool).await;

    let ans = save_draft(
        &db,
        campaign_id,
        &body_with(Some(vec![rule("FEE_WAIVER", json!({ "percent": "0" }))])),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "percent 0 must be refused with 400: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "{}", ans.text);
    assert_eq!(ans.field(), "rules.calculation.percent", "{}", ans.text);
    assert_eq!(
        stored_rules(&db.pool, version_id).await,
        vec![(
            "RATE_IMPROVEMENT".to_owned(),
            "PRICE".to_owned(),
            serde_json::from_str::<Value>(SEEDED_CALC).expect("seed calc"),
            1
        )],
        "refused save must not alter rules"
    );
    assert_eq!(
        stored_budget(&db.pool, version_id).await,
        Decimal::from_str(SEEDED_BUDGET).expect("decimal"),
        "refused save must not alter budget"
    );

    db.finish().await;
}

#[tokio::test]
async fn test_s2_draft_save_out_of_range_amount_refused() {
    let db = TestDb::start().await;
    let (campaign_id, version_id) = seed_draft(&db.pool).await;

    let ans = save_draft(
        &db,
        campaign_id,
        &body_with(Some(vec![rule("FEE_WAIVER", json!({ "amount": "0" }))])),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "amount 0 must be refused with 400: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "{}", ans.text);
    assert_eq!(ans.field(), "rules.calculation.amount", "{}", ans.text);
    assert_eq!(
        stored_rules(&db.pool, version_id).await,
        vec![(
            "RATE_IMPROVEMENT".to_owned(),
            "PRICE".to_owned(),
            serde_json::from_str::<Value>(SEEDED_CALC).expect("seed calc"),
            1
        )],
        "refused save must not alter rules"
    );
    assert_eq!(
        stored_budget(&db.pool, version_id).await,
        Decimal::from_str(SEEDED_BUDGET).expect("decimal"),
        "refused save must not alter budget"
    );

    db.finish().await;
}
