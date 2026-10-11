//! Transaction audit DTO for v3 B04 transaction audit trail view (ticket #1294, design §4.3).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction_audit.ts")]
pub struct TransactionAuditFilter {
    pub id: Option<String>,
}

impl crate::presentation::http::v3::view::ViewFilter for TransactionAuditFilter {}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction_audit.ts")]
pub struct TransactionAuditEventDto {
    pub at: String,
    pub actor_id: String,
    pub actor_role: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction_audit.ts")]
pub struct TransactionAuditViewData {
    pub events: Option<Vec<TransactionAuditEventDto>>,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    TransactionAuditFilter::export_all(&cfg)?;
    TransactionAuditEventDto::export_all(&cfg)?;
    TransactionAuditViewData::export_all(&cfg)?;
    Ok(())
}
