//! Transaction documents view DTOs for v3 (§7, §12.2).

use actix_web::http::StatusCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::ViewFilter;

/// The kind of document file for an exchange.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction_documents.ts")]
pub enum TransactionDocumentKind {
    #[serde(rename = "receipt")]
    #[ts(rename = "receipt")]
    Receipt,
    #[serde(rename = "receiptImage")]
    #[ts(rename = "receiptImage")]
    ReceiptImage,
}

/// One receipt document file of an exchange.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction_documents.ts")]
pub struct TransactionDocumentFile {
    pub kind: TransactionDocumentKind,
    pub path: String,
    pub download: Option<String>,
}

/// Filter for the `transaction_documents` view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "transaction_documents.ts")]
pub struct TransactionDocumentsFilter {
    #[serde(rename = "ref")]
    #[ts(rename = "ref")]
    pub r#ref: String,
}

impl ViewFilter for TransactionDocumentsFilter {
    fn check(&self) -> Result<(), V3Error> {
        if self.r#ref.trim().is_empty() {
            return Err(V3Error::new(
                "invalidRequest",
                Some("ref"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }
        Ok(())
    }
}

/// Answer data for the `transaction_documents` view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction_documents.ts")]
pub struct TransactionDocumentsData {
    #[serde(rename = "ref")]
    #[ts(rename = "ref")]
    pub r#ref: String,
    pub files: Vec<TransactionDocumentFile>,
}
