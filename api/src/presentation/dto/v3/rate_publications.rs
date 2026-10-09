//! Rate publications DTO (§4.7).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_publications.ts")]
pub struct RatePublicationItemDto {
    pub id: String,
    pub branch_id: String,
    pub version: i32,
    pub published_at: String,
    pub published_by: String,
    pub approved_request_id: String,
    pub draft_id: Option<String>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_publications.ts")]
pub struct RatePublicationsData {
    pub publications: Vec<RatePublicationItemDto>,
}
