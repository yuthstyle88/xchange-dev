//! Admin rate history view DTOs for v3 (§7, §12.14).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_rate_history.ts")]
pub struct BotReferenceRow {
    pub code: String,
    pub date: String,
    pub unit: u32,
    pub buying_sight: String,
    pub buying_transfer: String,
    pub selling: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_rate_history.ts")]
pub struct BotReference {
    pub fetched_at: String,
    pub rows: Vec<BotReferenceRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_rate_history.ts")]
pub struct AdminRateHistoryRow {
    pub id: String,
    pub code: String,
    pub buy: String,
    pub sell: String,
    pub updated: String,
    pub actor: String,
    pub partner_name: String,
    pub partner_cost: Option<String>,
    pub bot_reference: Option<BotReference>,
}
