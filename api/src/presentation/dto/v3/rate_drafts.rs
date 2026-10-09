//! Rate drafts DTO (§4.7).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_drafts.ts")]
pub struct RateDraftItemDto {
    pub id: String,
    pub branch_id: String,
    pub status: String,
    pub version: i32,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    #[ts(type = "any | null")]
    pub evaluation: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_drafts.ts")]
pub struct RateDraftsData {
    pub drafts: Vec<RateDraftItemDto>,
}

#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_draft.ts")]
pub struct RateDraftFilter {
    pub id: Option<String>,
}

impl crate::presentation::http::v3::view::ViewFilter for RateDraftFilter {}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_draft.ts")]
pub struct RateDraftLineDto {
    pub currency: String,
    pub buy_mode: String,
    pub sell_mode: String,
    pub manual_buy: Option<String>,
    pub manual_sell: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_draft.ts")]
pub struct RateDraftDetailItemDto {
    pub id: String,
    pub branch_id: String,
    pub status: String,
    pub version: i32,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    #[ts(type = "any | null")]
    pub evaluation: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_draft.ts")]
pub struct RateDraftDetailData {
    pub draft: Option<RateDraftDetailItemDto>,
    pub lines: Vec<RateDraftLineDto>,
}
