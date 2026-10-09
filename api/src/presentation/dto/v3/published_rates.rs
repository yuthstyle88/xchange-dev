//! Published rates DTO (§4.8).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "published_rates.ts")]
pub struct PublishedPublicationDto {
    pub id: String,
    pub version: i32,
    pub published_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "published_rates.ts")]
pub struct PublishedLineDto {
    pub currency: String,
    pub group: String,
    pub unit_size: String,
    pub buy_rate: Option<String>,
    pub sell_rate: Option<String>,
    pub line_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "published_rates.ts")]
pub struct PublishedRatesData {
    pub publication: Option<PublishedPublicationDto>,
    pub source_date: Option<String>,
    pub stale: bool,
    pub lines: Vec<PublishedLineDto>,
}
