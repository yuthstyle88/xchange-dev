//! OR-5a (#945): A05's users, the `admin_users` view's rows. No invite code and no
//! `auth_subject` ever leaves the server.

use std::path::Path;

use actix_web::http::StatusCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::ViewFilter;

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

/// Filter for the `admin_users` view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "admin_users.ts")]
pub struct AdminUsersFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub q: Option<String>,
}

impl ViewFilter for AdminUsersFilter {
    fn check(&self) -> Result<(), V3Error> {
        if let Some(ref q) = self.q
            && q.chars().count() > 100
        {
            return Err(V3Error::new(
                "invalidRequest",
                Some("q"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }
        Ok(())
    }
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminUserRole::export_all(&cfg)?;
    AdminUserRow::export_all(&cfg)?;
    AdminUsersFilter::export_all(&cfg)?;
    Ok(())
}
