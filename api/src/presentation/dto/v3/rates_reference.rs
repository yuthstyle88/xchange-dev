//! Rates reference view DTOs for v3 (§7, §12.16).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::view::ViewFilter;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rates_reference.ts")]
pub struct RatesReferenceRow {
    pub code: String,
    pub date: String,
    pub unit: u32,
    pub buying_sight: String,
    pub buying_transfer: String,
    pub selling: String,
}

/// Answer data for the `rates_reference` view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rates_reference.ts")]
pub struct RatesReferenceData {
    pub rows: Vec<RatesReferenceRow>,
    pub fetched_at: String,
    pub error: Option<String>,
}

/// Filter for the `rates_reference` view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "rates_reference.ts")]
pub struct RatesReferenceFilter {
    pub fresh: Option<String>,
}

impl ViewFilter for RatesReferenceFilter {}

impl RatesReferenceFilter {
    pub fn is_fresh(&self) -> bool {
        self.fresh.as_deref() == Some("1")
    }
}
