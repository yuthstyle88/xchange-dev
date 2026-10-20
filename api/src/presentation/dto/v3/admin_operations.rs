//! Admin operations view DTOs for v3 (§7, §12.7).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_operations.ts")]
pub struct AdminOperationsEntity {
    pub company: String,
    pub site: String,
}

/// Answer data for the `admin_operations` view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_operations.ts")]
pub struct AdminOperationsData {
    pub entity: Option<AdminOperationsEntity>,
}
