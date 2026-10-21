//! RBAC F4b (#3153): DTO for the `admin_login_history` view (§3.4, §4).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_login_history.ts")]
pub struct AdminLoginHistoryRow {
    pub id: String,
    pub at: String,
    pub user_id: Option<String>,
    pub user_name: Option<String>,
    pub ip: Option<String>,
    #[ts(
        type = "'OK' | 'BAD_PASSWORD' | 'UNKNOWN_USER' | 'NOT_ACTIVE' | 'TEMP_EXPIRED' | 'RATE_LIMITED' | 'UNAVAILABLE'"
    )]
    pub result: String,
}
