//! MK-7 (#1406): the `campaign_performance` view (M04, design `marketing-campaigns.md` D-15, §6.1), camelCase. Money
//! is a decimal string with two places. Attributed, never "lift": no field is named profit or lift.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::dto::v3::common::Money;

/// M04's query keys: the period, inclusive `YYYY-MM-DD` bounds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "campaign_performance.ts")]
pub struct CampaignPerformanceFilter {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_performance.ts")]
pub struct CampaignPerformanceData {
    pub versions: Vec<VersionPerformanceRow>,
}

/// One campaign version over the period (D-15). REVERSED redemptions add nothing (ruling 2 on #1406).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_performance.ts")]
pub struct VersionPerformanceRow {
    pub campaign_id: String,
    pub campaign_name: String,
    pub version_id: String,
    #[ts(type = "number")]
    pub version: i64,
    #[ts(type = "number")]
    pub redemptions: i64,
    pub price_cost_thb: Money,
    pub reward_expense_thb: Money,
    pub volume_thb: Money,
    pub realized_margin_thb: PerformanceFigure,
    /// realized margin − REWARD expense − commission; PRICE cost is never subtracted (D-15).
    pub attributed_contribution_thb: PerformanceFigure,
}

/// A figure, or the reason it has none (`awaitsLedger`, `awaitsBroker`): exactly one of the two is set, and an
/// unavailable figure is never shown as zero (§6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_performance.ts")]
pub struct PerformanceFigure {
    pub value: Option<Money>,
    pub unavailable: Option<String>,
}
