//! The `documents` and `staff_documents` view DTOs for v3 (#1349, design `xchange-v3-wave3-report-documents.md` §A).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::view::ViewFilter;

/// The kind of one document of an exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "documents.ts")]
pub enum DocumentsLinkKind {
    Passport,
    PaymentProof,
    Receipt,
}

/// One document: its kind, its link, and the owner's download name (`None` on the staff view).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "documents.ts")]
pub struct DocumentsLink {
    pub kind: DocumentsLinkKind,
    pub href: String,
    pub download: Option<String>,
}

/// Filter for the documents views: the exchange's reference. An empty one is v2's not-found, not a refusal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "documents.ts")]
pub struct DocumentsFilter {
    #[serde(rename = "ref")]
    #[ts(rename = "ref")]
    pub r#ref: String,
}

impl ViewFilter for DocumentsFilter {}

/// Answer data for the documents views: one exchange's documents, in v2's order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "documents.ts")]
pub struct DocumentsData {
    #[serde(rename = "ref")]
    #[ts(rename = "ref")]
    pub r#ref: String,
    pub links: Vec<DocumentsLink>,
}
