//! DTOs for `profit_summary` compute action (§12.20.5).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::dto::v3::common::Money;

#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "profit_summary.ts")]
pub struct ProfitSummaryRequest {
    #[ts(optional)]
    pub realized: Option<String>,
    #[ts(optional)]
    pub foreign_value: Option<String>,
    #[ts(optional)]
    pub foreign_cost: Option<String>,
    #[ts(optional)]
    pub net_assets: Option<String>,
    #[ts(optional)]
    pub capital: Option<String>,
    #[ts(optional)]
    pub reserve: Option<String>,
    #[ts(optional)]
    pub cash: Option<String>,
    #[ts(optional)]
    pub cash_minimum: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "profit_summary.ts")]
pub struct ProfitSummary {
    pub realized: Money,
    pub unrealized: Money,
    pub withdrawable: Money,
}
