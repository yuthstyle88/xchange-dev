//! RBAC F4a (#3152): DTO for the `admin_user_audit` view (§3.4, §4).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_user_audit.ts")]
pub struct AdminUserAuditRow {
    pub id: String,
    pub at: String,
    pub actor_id: String,
    pub actor_name: String,
    pub subject_id: String,
    pub subject_name: String,
    pub action: String,
    #[ts(type = "unknown")]
    pub before: Option<serde_json::Value>,
    #[ts(type = "unknown")]
    pub after: Option<serde_json::Value>,
    pub ip: Option<String>,
}
