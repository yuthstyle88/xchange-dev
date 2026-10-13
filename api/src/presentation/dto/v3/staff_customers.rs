//! Staff customers view DTOs for v3 (§3.1).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::view::ViewFilter;

/// Row in the `staff_customers` view (§3.1, §8.1 row S-2).
/// Exactly `{ id, displayName }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_customers.ts")]
pub struct StaffCustomerRow {
    pub id: String,
    pub display_name: String,
}

/// Filter for the `staff_customers` view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "staff_customers.ts")]
pub struct StaffCustomersFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub q: Option<String>,
}

impl ViewFilter for StaffCustomersFilter {}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    StaffCustomerRow::export_all(&cfg)?;
    StaffCustomersFilter::export_all(&cfg)?;
    Ok(())
}
