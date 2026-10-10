//! APP-5 (#995): the public branch list, `GET /api/public/orgs/{org}/branches`.
//!
//! Lane controller's rulings on #995 (2026-10-10): a public route, not a v3 view; the
//! organisation code is in the path. The TS is committed at
//! `web/src/shared/api/public/public_branches.ts`, not under `generated/`, which holds only the
//! v3 views' files (`api/tests/v3_types_test.rs`).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One branch as anyone may see it. No customer field.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "public_branches.ts")]
pub struct PublicBranchDto {
    pub code: String,
    pub name: String,
    pub address: Option<String>,
    /// `branches.opening_hours`, as `BranchHours::new` accepts it.
    #[ts(type = "Record<string, Array<{ open: string; close: string }>> | null")]
    pub opening_hours: Option<serde_json::Value>,
    pub timezone: String,
    pub booking_enabled: bool,
    /// `branch_service_status.queue_open`; `false` when the branch has no row. The estimates are
    /// QU-7's public `branch_service_status` view (`queue-tickets.md:163`).
    pub queue_open: bool,
}

/// The organisation's branches, sorted by `code`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "public_branches.ts")]
pub struct PublicBranchesData {
    pub branches: Vec<PublicBranchDto>,
}

/// Writes `public_branches.ts` into `dir`.
pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PublicBranchDto::export_all(&cfg)?;
    PublicBranchesData::export_all(&cfg)?;
    Ok(())
}
