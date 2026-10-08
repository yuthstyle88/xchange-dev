//! Admin employees view DTOs for v3 (§7, §12.7).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_employees.ts")]
pub struct AdminEmployeeRow {
    pub id: String,
    pub username: String,
    pub name: String,
    pub role: String,
    pub branch_id: String,
    pub active: bool,
    pub must_change: bool,
}

/// Answer data for the `admin_employees` view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_employees.ts")]
pub struct AdminEmployeesData {
    pub employees: Vec<AdminEmployeeRow>,
}
