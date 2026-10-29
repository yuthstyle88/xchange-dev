//! Helpers of the pricing-formula API up-front tests, on `pricing_common::Room`. Written before the
//! implementation. Do not edit, delete or weaken.
#![allow(dead_code)]

use serde_json::{Value, json};
use uuid::Uuid;

use super::pricing_common::{ORG, Room, Who, command, unique};

pub const SAVE: &str = "/api/pricing/formula";
pub const PREVIEW: &str = "/api/pricing/formula/preview";
pub const POLICY: &str = "/api/rate-policies";
pub const CONFIGURE: &str = "/api/pricing/configure";
pub const PFB: &str = "XC-PFB";

pub fn org() -> Uuid {
    Uuid::parse_str(ORG).expect("the seeded organisation")
}

pub fn finance() -> Who {
    Who(format!("{}|FINANCE", Uuid::new_v4()))
}

pub fn branch_manager() -> Who {
    Who(format!("{}|BRANCH_MANAGER", Uuid::new_v4()))
}

pub fn guards(min: &str, max: &str) -> Value {
    let g = json!({ "min_pct": min, "max_pct": max });
    json!({
        "USD": { "buy": g, "sell": g },
        "EUR": { "buy": g, "sell": g },
        "GBP": { "buy": g, "sell": g },
    })
}

pub fn wide() -> Value {
    guards("-50.0000", "50.0000")
}

/// The current policy version of `branch` (version 1), with four zero-margin tiers when `tiers_on`.
pub async fn policy(room: &Room, branch: &str, buy: &str, sell: &str, guards: Value, tiers_on: bool) -> Uuid {
    room.create_branch(branch).await;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO rate_policy_versions \
           (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, valid_from, valid_to, \
            approved_by, reason, value_tiers_on) \
         VALUES (gen_random_uuid(), $1, 1, $2::numeric, $3::numeric, $4, now() - interval '1 day', NULL, \
                 'owner-1', 'seeded', $5) RETURNING id",
    )
    .bind(branch)
    .bind(buy)
    .bind(sell)
    .bind(guards)
    .bind(tiers_on)
    .fetch_one(&room.pool)
    .await
    .expect("insert a policy version");
    if tiers_on {
        for (no, upper) in [(1i16, Some("20000.00")), (2, Some("100000.00")), (3, Some("500000.00")), (4, None)] {
            sqlx::query(
                "INSERT INTO rate_policy_value_tiers (policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct) \
                 VALUES ($1, $2, $3::numeric, 0, 0)",
            )
            .bind(id)
            .bind(no)
            .bind(upper)
            .execute(&room.pool)
            .await
            .expect("insert a value tier");
        }
    }
    id
}

pub async fn set_branch_formula(room: &Room, version: Uuid, buy: &str, sell: &str) {
    sqlx::query("UPDATE rate_policy_versions SET buy_formula = $2, sell_formula = $3 WHERE id = $1")
        .bind(version)
        .bind(buy)
        .bind(sell)
        .execute(&room.pool)
        .await
        .expect("set the branch formula");
}

pub async fn org_formula(room: &Room, version: i32, buy: &str, sell: &str, current: bool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO pricing_formula_versions \
         (id, organization_id, version, buy_formula, sell_formula, valid_from, valid_to, approved_by, reason) \
         VALUES ($1, $2, $3, $4, $5, now() - interval '1 hour', CASE WHEN $6 THEN NULL ELSE now() END, 'owner-1', 'seeded')",
    )
    .bind(id)
    .bind(org())
    .bind(version)
    .bind(buy)
    .bind(sell)
    .bind(current)
    .execute(&room.pool)
    .await
    .expect("insert an organisation formula");
    id
}

pub async fn follow(room: &Room, currency: &str) {
    sqlx::query(
        "INSERT INTO pricing_modes (currency_code, mode, revision, updated_by, updated_at) VALUES ($1, 'FOLLOW', 1, 'owner-1', now()) \
         ON CONFLICT (currency_code) DO UPDATE SET mode = 'FOLLOW'",
    )
    .bind(currency)
    .execute(&room.pool)
    .await
    .expect("set FOLLOW");
}

/// The 10-28 BOT USD line and an EUR line.
pub async fn bot_1028(room: &Room) {
    room.bot_batch(&[("USD", "33.6482", "33.9671"), ("EUR", "38.0000", "38.6000")], 1).await;
}

/// `(version, buy, sell, current, source_version)` of every organisation formula, by version.
pub async fn org_versions(room: &Room) -> Vec<(i32, String, String, bool, Option<i32>)> {
    sqlx::query_as(
        "SELECT version, buy_formula, sell_formula, valid_to IS NULL, source_version FROM pricing_formula_versions \
         WHERE organization_id = $1 ORDER BY version",
    )
    .bind(org())
    .fetch_all(&room.pool)
    .await
    .expect("read the organisation formulas")
}

/// `(version, buy_formula, sell_formula)` of every policy version of `branch`, by version.
pub async fn branch_formulas(room: &Room, branch: &str) -> Vec<(i32, Option<String>, Option<String>)> {
    sqlx::query_as(
        "SELECT version, buy_formula, sell_formula FROM rate_policy_versions WHERE branch_id = $1 ORDER BY version",
    )
    .bind(branch)
    .fetch_all(&room.pool)
    .await
    .expect("read the policy versions")
}

pub fn save_body(buy: &str, sell: &str) -> Value {
    json!({ "buyFormula": buy, "sellFormula": sell, "reason": "owner formula" })
}

pub async fn save(room: &Room, who: &Who, buy: &str, sell: &str) -> super::pricing_common::Answer {
    command(room, who, SAVE, "MAIN", save_body(buy, sell), Some(&unique("key"))).await
}

/// The error's `field` and `message`.
pub fn error_of(a: &super::pricing_common::Answer) -> (Value, String) {
    (
        a.body["error"]["field"].clone(),
        a.body["error"]["message"].as_str().unwrap_or_default().to_owned(),
    )
}

/// A ratePolicySave body with today's required fields.
pub fn policy_body(buy: &str, sell: &str, guards: Value) -> Value {
    json!({
        "buyAdjustPct": buy,
        "sellAdjustPct": sell,
        "guards": guards,
        "maxReferenceAgeHours": null,
        "onStale": null,
        "staleLimitThb": null,
        "reason": "policy change",
    })
}

/// Refused as the policy refuses a role outside its row (`pricing_configure_test.rs:263-268`): `roleNotAllowedHere`
/// with its status.
pub fn refused_role(a: &super::pricing_common::Answer, what: &str) {
    use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
    use xchange::presentation::http::v3::error::V3Error;
    let err = PolicyError::RoleNotAllowedHere;
    assert_eq!(a.status.as_u16(), err.status(), "{what}: {}", a.text);
    assert_eq!(a.body["error"]["code"], json!(V3Error::from(PolicyError::RoleNotAllowedHere).code), "{what}: {}", a.text);
}
