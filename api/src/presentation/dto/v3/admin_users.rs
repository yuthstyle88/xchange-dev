//! OR-5a (#945): A05's users, the `admin_users` view's rows. No invite code and no
//! `auth_subject` ever leaves the server.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_users.ts")]
pub struct AdminUserRole {
    pub role: String,
    /// `null` for an organisation-wide role.
    pub branch: Option<String>,
    pub valid_from: String,
    pub valid_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_users.ts")]
pub struct AdminUserRow {
    pub user_id: String,
    pub name: String,
    #[ts(optional = nullable)]
    pub employee_code: Option<String>,
    #[ts(optional = nullable)]
    pub department: Option<String>,
    #[ts(optional = nullable)]
    pub position: Option<String>,
    #[ts(optional = nullable)]
    pub tell_code: Option<String>,
    /// `INVITED`, `ACTIVE` or `DISABLED`.
    pub status: String,
    pub version: i32,
    /// Every role row of the user, ended ones included, oldest first.
    pub roles: Vec<AdminUserRole>,
}
