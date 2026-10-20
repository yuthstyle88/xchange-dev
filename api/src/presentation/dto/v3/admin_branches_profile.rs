//! OR-5a (#945): A04's branch profile (OR-1's columns), the `admin_branches_profile` view's
//! answer and what `branchSave` answers.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_branches_profile.ts")]
pub struct AdminBranchContactChannel {
    #[ts(type = "\"PHONE\" | \"LINE\" | \"EMAIL\" | \"FACEBOOK\" | \"WEBSITE\"")]
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_branches_profile.ts")]
pub struct AdminBranchProfileData {
    pub branch: String,
    pub name: String,
    pub code: Option<String>,
    pub address: Option<String>,
    pub timezone: String,
    #[ts(type = "Record<string, Array<{ open: string; close: string }>> | null")]
    pub opening_hours: Option<serde_json::Value>,
    pub booking_enabled: bool,
    pub contact_channels: Vec<AdminBranchContactChannel>,
    #[ts(type = "\"ACTIVE\" | \"INACTIVE\"")]
    pub status: String,
}
