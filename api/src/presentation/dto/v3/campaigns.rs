//! MK-7 (#1406): the `campaigns` view (M01, design `marketing-campaigns.md` §4.5, D-2), camelCase. Money is a
//! decimal string with two places.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::dto::v3::common::Money;

/// M01's query keys: the period of the period totals, inclusive `YYYY-MM-DD` bounds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "campaigns.ts")]
pub struct CampaignsFilter {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaigns.ts")]
pub struct CampaignsData {
    pub campaigns: Vec<CampaignListItem>,
}

/// One campaign of the caller's organisation, shown through its current version (`campaigns.current_version_id`),
/// or its newest version while none is approved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaigns.ts")]
pub struct CampaignListItem {
    pub id: String,
    pub name: String,
    pub version_id: String,
    #[ts(type = "number")]
    pub version: i64,
    /// The stored status, except an APPROVED version, which shows its derived SCHEDULED / ACTIVE / EXPIRED (D-2).
    pub status: String,
    pub starts_at: String,
    pub ends_at: String,
    pub budget_thb: Money,
    /// `campaign_budgets.consumed_thb`.
    pub budget_used_thb: Money,
    /// budget − consumed − active (RESERVED, unexpired) reservations: the "available" of design §4.1.
    pub budget_left_thb: Money,
    /// `campaign_budgets.used_count`.
    #[ts(type = "number")]
    pub uses: i64,
    #[ts(type = "number | null")]
    pub max_uses: Option<i64>,
    pub period: CampaignPeriodTotals,
}

/// The campaign's REDEEMED redemptions in the period and scope; REVERSED ones add nothing (ruling 2 on #1406).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaigns.ts")]
pub struct CampaignPeriodTotals {
    #[ts(type = "number")]
    pub redemptions: i64,
    pub price_cost_thb: Money,
    pub reward_expense_thb: Money,
}
