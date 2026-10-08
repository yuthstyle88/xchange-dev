//! Admin payments view DTOs for v3 (§7, §12.3).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_payments.ts")]
pub struct PaymentMethodItem {
    pub id: String,
    pub enabled: bool,
    pub buy: bool,
    pub sell: bool,
    #[serde(default)]
    pub account: String,
    #[serde(default)]
    pub instruction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_payments.ts")]
pub struct PaymentHistoryItem {
    #[ts(type = "number")]
    pub version: i64,
    pub actor: String,
    pub created_at: String,
    pub before_methods: Vec<PaymentMethodItem>,
    pub after_methods: Vec<PaymentMethodItem>,
}

/// Answer data for the `admin_payments` view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_payments.ts")]
pub struct AdminPaymentsData {
    pub branch_id: String,
    #[ts(type = "number")]
    pub settings_version: i64,
    pub methods: Vec<PaymentMethodItem>,
    pub updated_at: Option<String>,
    pub updated_by: Option<String>,
    pub can_edit: bool,
    pub history: Vec<PaymentHistoryItem>,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PaymentMethodItem::export_all(&cfg)?;
    PaymentHistoryItem::export_all(&cfg)?;
    AdminPaymentsData::export_all(&cfg)?;
    Ok(())
}
