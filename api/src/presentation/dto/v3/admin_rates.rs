//! Admin rates view DTOs for v3 (§7, §12.1).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_rates.ts")]
pub struct AdminRateRow {
    pub code: String,
    pub name: String,
    pub symbol: String,
    pub buy: Option<String>,
    pub sell: Option<String>,
    pub spread: Option<String>,
    pub updated: String,
    pub partner_name: String,
    pub partner_cost: Option<String>,
    pub partner_spread: Option<String>,
}

/// Answer data for the `admin_rates` view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_rates.ts")]
pub struct AdminRatesData {
    pub updated: String,
    pub rates: Vec<AdminRateRow>,
}
