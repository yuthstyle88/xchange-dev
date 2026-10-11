//! Receipt document DTO for v3 B04 receipt reprint view (ticket #1294, design §4.2).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "receipt_document.ts")]
pub struct ReceiptDocumentFilter {
    pub number: Option<String>,
}

impl crate::presentation::http::v3::view::ViewFilter for ReceiptDocumentFilter {}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "receipt_document.ts")]
pub struct ReceiptDocumentDto {
    pub number: String,
    pub revision: i32,
    pub issued_at: String,
    pub html: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "receipt_document.ts")]
pub struct ReceiptDocumentViewData {
    pub receipt: Option<ReceiptDocumentDto>,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ReceiptDocumentFilter::export_all(&cfg)?;
    ReceiptDocumentDto::export_all(&cfg)?;
    ReceiptDocumentViewData::export_all(&cfg)?;
    Ok(())
}
