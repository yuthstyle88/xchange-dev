//! Desk configuration view DTOs for v3 (design §4.1).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "desk_config.ts")]
pub struct DeskPaymentMethod {
    pub id: String,
    pub enabled: bool,
    pub buy: bool,
    pub sell: bool,
    pub account: String,
    pub instruction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "desk_config.ts")]
pub struct DeskRate {
    pub code: String,
    pub unit_size: String,
    pub buy: Option<String>,
    pub sell: Option<String>,
    pub publication: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "desk_config.ts")]
pub struct DeskConfig {
    pub branch_id: String,
    pub branch_name: String,
    pub shift_open: bool,
    pub admin_url: String,
    pub rates: Vec<DeskRate>,
    pub payment_methods: Vec<DeskPaymentMethod>,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DeskPaymentMethod::export_all(&cfg)?;
    DeskRate::export_all(&cfg)?;
    DeskConfig::export_all(&cfg)?;
    Ok(())
}
