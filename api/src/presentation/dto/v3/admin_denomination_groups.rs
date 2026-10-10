//! Banknote groups admin view DTOs (§10.2).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_denomination_groups.ts")]
pub struct GroupDto {
    pub group: String,
    pub instrument_key: Option<String>,
    pub removable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_denomination_groups.ts")]
pub struct GroupCurrencyDto {
    pub currency: String,
    pub groups: Vec<GroupDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_denomination_groups.ts")]
pub struct AdminDenominationGroupsData {
    pub provider: String,
    pub currencies: Vec<GroupCurrencyDto>,
}
