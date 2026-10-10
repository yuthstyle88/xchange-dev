//! Consent texts DTO (§4.4).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "consent_texts.ts")]
pub struct ConsentTextViewItem {
    pub purpose: String,
    pub lang: String,
    pub version: i32,
    pub body: String,
    pub published_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "consent_texts.ts")]
pub struct ConsentTextsData {
    pub texts: Vec<ConsentTextViewItem>,
}
