//! Staff holds view DTOs for v3 (§3.1).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_holds.ts")]
pub struct StaffHoldRow {
    pub id: String,
    pub customer_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_name: Option<String>,
    pub reason: String,
    pub status: String,
    pub at: String,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    StaffHoldRow::export_all(&cfg)?;
    Ok(())
}
