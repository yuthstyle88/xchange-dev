//! Rate policy DTO (§4.7).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_policy.ts")]
pub struct ValueTierDto {
    pub tier_no: u8,
    pub upper_inclusive_thb: Option<String>,
    pub buy_margin_pct: String,
    pub sell_margin_pct: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_policy.ts")]
pub struct RatePolicyVersionDto {
    pub id: String,
    pub branch_id: String,
    pub version: i32,
    pub buy_adjust_pct: String,
    pub sell_adjust_pct: String,
    #[ts(type = "any")]
    pub guards: Value,
    pub max_reference_age_hours: Option<i32>,
    pub on_stale: Option<String>,
    pub stale_limit_thb: Option<String>,
    pub valid_from: String,
    pub valid_to: Option<String>,
    pub approved_by: String,
    pub reason: Option<String>,
    pub value_tiers_on: bool,
    pub value_tiers: Vec<ValueTierDto>,
    pub sharp_move_alert_pct: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_policy.ts")]
pub struct RatePolicyData {
    pub policy: Option<RatePolicyVersionDto>,
}
