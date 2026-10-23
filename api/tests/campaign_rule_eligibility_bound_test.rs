//! Up-front tests for #4916: `campaignVersionSave` checks each rule's eligibility before the write.
//!
//! Written before the implementation by the ticket author's lane (claude/28619d18, for claude/7ca80c).
//! Do not edit, delete or weaken these tests; add your own in another file.
//!
//! Design `docs/design/blueprint/bounded-inputs.md` B-1 / B-7: a decimal goes through `Bound`, and a
//! value that names a row is checked to exist before the write, answering `400 validationFailed` with
//! the field. Today `campaign_version_save.rs:243-278` checks the calculation params only and stores
//! `eligibility` as sent.
//!
//! Field names: `V3Error.field` is `&'static str` (`v3/error.rs:11`), so the field is the path without
//! the index, as the save already answers for `rules.eligibility.requiresCoupon`
//! (`campaign_version_save.rs:254`); the message names the index (`rules[0]`).
//!
//! Behaviour rows (#4916 §3):
//! | # | Rule eligibility | Expect |
//! |---|---|---|
//! | CE1 | currencies `["USDD"]` | 400 `validationFailed`, field `rules.eligibility.currencies`, nothing stored |
//! | CE2 | minThb `"1.234"` | 400, field `rules.eligibility.minThb`, nothing stored |
//! | CE3 | minThb `"-1"` | 400, field `rules.eligibility.minThb`, nothing stored |
//! | CE4 | currencies `["USD"]`, minThb `"1000"` | 200, saved as today |

mod campaign_common;
mod common;
mod quote_common;

use std::str::FromStr;

use actix_web::http::StatusCode;
use campaign_common::*;
use quote_common::TestDb;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

/// One well-formed FEE_WAIVER rule carrying `eligibility`.
fn rule_with(eligibility: Value) -> Value {
    let mut r = rule("FEE_WAIVER", json!({ "percent": "0.5" }));
    r["eligibility"] = eligibility;
    r
}

async fn stored_eligibility(pool: &PgPool, version_id: Uuid) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT eligibility FROM benefit_rules WHERE campaign_version_id = $1 ORDER BY rule_no",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
    .expect("read stored eligibility")
}

/// Saves one rule with `eligibility` and expects 400 `validationFailed` on `field`, the message
/// naming `rules[0]`, and nothing written: the seeded rule is still the only one and the version's
/// budget is unchanged.
async fn refused(eligibility: Value, field: &str) {
    let room = TestDb::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        &body_with(Some(vec![rule_with(eligibility.clone())])),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "eligibility {eligibility} must be refused with 400: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "{}", ans.text);
    assert_eq!(ans.field(), field, "{}", ans.text);
    assert!(
        ans.message().contains("rules[0]"),
        "the message names the rule's index: {}",
        ans.text
    );
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

#[actix_web::test]
async fn ce1_unknown_eligibility_currency_is_refused() {
    refused(
        json!({ "currencies": ["USDD"] }),
        "rules.eligibility.currencies",
    )
    .await;
}

#[actix_web::test]
async fn ce2_min_thb_with_three_decimals_is_refused() {
    refused(
        json!({ "currencies": ["USD"], "minThb": "1.234" }),
        "rules.eligibility.minThb",
    )
    .await;
}

#[actix_web::test]
async fn ce3_negative_min_thb_is_refused() {
    refused(
        json!({ "currencies": ["USD"], "minThb": "-1" }),
        "rules.eligibility.minThb",
    )
    .await;
}

#[actix_web::test]
async fn ce4_known_currency_and_min_thb_save_as_today() {
    let room = TestDb::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;
    let eligibility = json!({ "currencies": ["USD"], "minThb": "1000" });

    let ans = save(
        &room,
        campaign_id,
        &body_with(Some(vec![rule_with(eligibility.clone())])),
    )
    .await;

    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(
        stored_rules(&room.pool, version_id).await,
        vec![(
            "FEE_WAIVER".to_owned(),
            "PRICE".to_owned(),
            json!({ "percent": "0.5" }),
            1
        )],
        "the saved rule replaces the draft's rule"
    );
    assert_eq!(
        stored_eligibility(&room.pool, version_id).await,
        vec![eligibility],
        "the eligibility is stored as sent"
    );

    room.finish().await;
}
