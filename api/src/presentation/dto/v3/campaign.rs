//! MK-7 (#1406): the `campaign` view (M02, design `marketing-campaigns.md` §4.5), camelCase. Money is a decimal
//! string with two places.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::dto::v3::common::Money;

/// `?id=` names the campaign.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "campaign.ts")]
pub struct CampaignFilter {
    pub id: Option<String>,
}

/// `campaign` is `null` for an id that is not a campaign of the caller's organisation (the `transaction` view's
/// convention for an unknown id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign.ts")]
pub struct CampaignData {
    pub campaign: Option<CampaignDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign.ts")]
pub struct CampaignDetail {
    pub id: String,
    pub name: String,
    pub current_version_id: Option<String>,
    /// Newest first.
    pub versions: Vec<CampaignVersionDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign.ts")]
pub struct CampaignVersionDetail {
    pub id: String,
    #[ts(type = "number")]
    pub version: i64,
    /// The stored status, except an APPROVED version, which shows its derived SCHEDULED / ACTIVE / EXPIRED (D-2).
    pub status: String,
    pub starts_at: String,
    pub ends_at: String,
    pub budget_thb: Money,
    #[ts(type = "number | null")]
    pub max_uses: Option<i64>,
    pub stack_group: String,
    #[ts(type = "number")]
    pub priority: i64,
    /// Sorted.
    pub branches: Vec<String>,
    /// By `rule_no`.
    pub rules: Vec<CampaignRuleDetail>,
    /// Newest first.
    pub simulations: Vec<CampaignSimulationDetail>,
    /// The version's approval request (AP-1), when submitted.
    pub approval: Option<CampaignApprovalDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign.ts")]
pub struct CampaignRuleDetail {
    pub id: String,
    #[ts(type = "number")]
    pub rule_no: i64,
    pub benefit_type: String,
    pub benefit_class: String,
    pub requires_coupon: bool,
    /// The rule's stored `eligibility` JSON (§4.3).
    #[ts(type = "unknown")]
    pub eligibility: serde_json::Value,
    /// The calculator's stored params, decimals as strings.
    #[ts(type = "unknown")]
    pub calculation: serde_json::Value,
    #[ts(type = "unknown")]
    pub caps: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign.ts")]
pub struct CampaignSimulationDetail {
    pub id: String,
    /// `LEDGER` or `UNAVAILABLE` (§10 "input cost data ต้องระบุ").
    pub cost_source: String,
    pub simulated_at: String,
    #[ts(type = "unknown")]
    pub input: serde_json::Value,
    #[ts(type = "unknown")]
    pub result: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign.ts")]
pub struct CampaignApprovalDetail {
    pub id: String,
    /// `approval_requests.status`: PENDING, APPROVED or REJECTED.
    pub status: String,
}
