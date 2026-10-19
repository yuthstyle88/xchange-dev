//! V.9's Employee Register columns (`docs/blueprint/XCHANGE-RBAC-V9.md:107`, #3154).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_employee_register.ts")]
pub struct AdminEmployeeRegisterRow {
    pub user_id: String,
    pub employee_code: Option<String>,
    pub name: String,
    pub position: Option<String>,
    pub department: Option<String>,
    pub tell_code: Option<String>,
    pub role: Option<String>,
    pub branch: Option<String>,
    pub status: String,
}
