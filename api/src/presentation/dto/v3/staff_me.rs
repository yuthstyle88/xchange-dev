//! Staff me view DTOs for v3 (§3.1).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_me.ts")]
pub struct StaffRoleDto {
    pub role: String,
    pub branch_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_me.ts")]
pub struct StaffMeView {
    pub user_id: String,
    pub name: String,
    pub roles: Vec<StaffRoleDto>,
    pub branch_id: String,
    pub branch_name: String,
    pub permissions: Vec<String>,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    StaffRoleDto::export_all(&cfg)?;
    StaffMeView::export_all(&cfg)?;
    Ok(())
}
