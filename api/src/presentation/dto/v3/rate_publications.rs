//! Rate publications DTO (§4.7).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_publications.ts")]
pub struct RateSides {
    pub buy: Option<String>,
    pub sell: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_publications.ts")]
pub struct RatePublicationLine {
    pub currency: String,
    pub group: String,
    pub value_tier: String,
    pub unit_size: String,
    pub before: Option<RateSides>,
    pub after: Option<RateSides>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_publications.ts")]
pub struct RatePublicationRow {
    pub id: String,
    pub branch_id: String,
    pub version: i32,
    pub published_at: String,
    pub published_by: String,
    pub approved_request_id: String,
    pub draft_id: Option<String>,
    pub state: String,
    pub lines: Vec<RatePublicationLine>,
}
