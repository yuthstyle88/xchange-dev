//! Admin branches view DTOs for v3 (§7, §12.4).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_branches.ts")]
pub struct AdminBranchItem {
    pub id: String,
    pub name: String,
}

/// Answer data for the `admin_branches` view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_branches.ts")]
pub struct AdminBranchesData {
    pub branches: Vec<AdminBranchItem>,
}
