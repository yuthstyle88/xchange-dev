//! Route-level integration tests for #2796: RATE_IMPROVEMENT range validation.
//!
//! Done criteria:
//! - Q1: quote 500 for a stored `{"delta":"-0.10"}`, written with SQL as
//!   `campaign_benefit_params_reader_test.rs` does; no quote, no reservation written.
//! - S1: `PUT /api/campaigns/{id}/draft` answers 400 `validationFailed`, field `rules.calculation.delta`,
//!   nothing written (tested with `"delta": "-0.10"` and `"delta": "0"`).
//! - S2: `PUT /api/campaigns/{id}/draft` answers 400 `validationFailed`, field `rules.calculation.percent`,
//!   nothing written (tested with `"percent": "1"`, `"percent": "0"`, `"percent": "1.5"`, `"percent": "-0.01"`).

mod common;
mod quote_common;
mod campaign_common;

use std::str::FromStr;

use actix_web::http::StatusCode;
use campaign_common::*;
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};

async fn verify_save_refused(db: &TestDb, calculation: Value, expected_field: &str) {
    let (campaign_id, version_id) = seed_draft(&db.pool).await;

    let ans = save_draft(
        db,
        campaign_id,
        &body_with(Some(vec![rule("RATE_IMPROVEMENT", calculation.clone())])),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "RATE_IMPROVEMENT {calculation} must be refused with 400: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "{}", ans.text);
    assert_eq!(ans.field(), expected_field, "{}", ans.text);
    assert_eq!(
        stored_rules(&db.pool, version_id).await,
        vec![(
            "RATE_IMPROVEMENT".to_owned(),
            "PRICE".to_owned(),
            serde_json::from_str::<Value>(SEEDED_CALC).expect("seed calc"),
            1
        )],
        "a refused save must not replace or add rules"
    );
    assert_eq!(
        stored_budget(&db.pool, version_id).await,
        Decimal::from_str(SEEDED_BUDGET).expect("decimal"),
        "a refused save must not write the version's terms either"
    );
}


// ── Q1: quote 500 for stored negative delta ──────────────────────────────────

#[actix_web::test]
async fn test_q1_quote_refuses_stored_negative_delta() {
    let (db, _rule) = room_with_rule("RATE_IMPROVEMENT", json!({ "delta": "-0.10" })).await;

    let ans = quote(&db, "100.00").await;

    assert_eq!(
        ans.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a stored RATE_IMPROVEMENT rule with negative delta must answer 500 serverError: {}",
        ans.text
    );
    assert_eq!(ans.code(), "serverError", "{}", ans.text);
    assert_eq!(db.quote_rows().await, (0, 0), "no quote is written");
    assert_eq!(reservations(&db).await, 0, "no budget is reserved");

    db.finish().await;
}

// ── S1: save draft refuses delta <= 0 (rules.calculation.delta) ─────────────

#[actix_web::test]
async fn test_s1_save_draft_refuses_negative_delta() {
    common::install_test_pii_keys();
    let db = TestDb::empty().await;
    db.migrate_through(None).await;

    verify_save_refused(&db, json!({ "delta": "-0.10" }), "rules.calculation.delta").await;

    db.finish().await;
}

#[actix_web::test]
async fn test_s1_save_draft_refuses_zero_delta() {
    common::install_test_pii_keys();
    let db = TestDb::empty().await;
    db.migrate_through(None).await;

    verify_save_refused(&db, json!({ "delta": "0" }), "rules.calculation.delta").await;

    db.finish().await;
}

// ── S2: save draft refuses percent not in (0, 1) (rules.calculation.percent) ─

#[actix_web::test]
async fn test_s2_save_draft_refuses_percent_one() {
    common::install_test_pii_keys();
    let db = TestDb::empty().await;
    db.migrate_through(None).await;

    verify_save_refused(&db, json!({ "percent": "1" }), "rules.calculation.percent").await;

    db.finish().await;
}

#[actix_web::test]
async fn test_s2_save_draft_refuses_percent_zero() {
    common::install_test_pii_keys();
    let db = TestDb::empty().await;
    db.migrate_through(None).await;

    verify_save_refused(&db, json!({ "percent": "0" }), "rules.calculation.percent").await;

    db.finish().await;
}

#[actix_web::test]
async fn test_s2_save_draft_refuses_percent_greater_than_one() {
    common::install_test_pii_keys();
    let db = TestDb::empty().await;
    db.migrate_through(None).await;

    verify_save_refused(
        &db,
        json!({ "percent": "1.5" }),
        "rules.calculation.percent",
    )
    .await;

    db.finish().await;
}

#[actix_web::test]
async fn test_s2_save_draft_refuses_negative_percent() {
    common::install_test_pii_keys();
    let db = TestDb::empty().await;
    db.migrate_through(None).await;

    verify_save_refused(
        &db,
        json!({ "percent": "-0.01" }),
        "rules.calculation.percent",
    )
    .await;

    db.finish().await;
}
