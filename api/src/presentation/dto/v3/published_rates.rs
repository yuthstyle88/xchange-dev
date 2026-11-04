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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "published_rates.ts")]
pub struct PublishedBandDto {
    pub tier: String,
    pub buy_rate: Option<String>,
    pub sell_rate: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "published_rates.ts")]
pub struct PublishedBandHeadingDto {
    pub tier: String,
    pub lower_exclusive_thb: Option<String>,
    pub upper_inclusive_thb: Option<String>,
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
    pub bands: Vec<PublishedBandDto>,
    /// `AUTO` · `MANUAL` · `FOLLOW` (pricing-tiers.md §15.7): from `source_snapshot.pricing.source`; `MAIN_PUSH` reads
    /// as `MANUAL`.
    pub pricing_source: String,
    /// The board and the public feed drop a hidden line; the kiosk lists it `quoteEnabled: false`. `MANUAL` and
    /// `AUTO` are never hidden.
    pub hidden: bool,
    /// `DISPLAY` · `STALE` · `BLOCKED` · `MISMATCH`, or `null`.
    pub hidden_reason: Option<String>,
}

/// What the storefront shows (pricing-tiers.md §15.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "published_rates.ts")]
pub struct PublishedDisplayDto {
    pub tiers: Vec<u8>,
    pub featured: Vec<String>,
    pub highlight: bool,
    /// The priced currencies: featured first in their set order, then Super Rich's market order, then by code.
    pub order: Vec<String>,
    /// With `allMarket`: the branch's `ASK` rows that have no shown line, by `sort_order` — "contact the counter",
    /// never a price.
    pub more: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "published_rates.ts")]
pub struct PublishedRatesData {
    pub publication: Option<PublishedPublicationDto>,
    pub source_date: Option<String>,
    pub stale: bool,
    pub lines: Vec<PublishedLineDto>,
    pub band_headings: Vec<PublishedBandHeadingDto>,
    pub display: PublishedDisplayDto,
}
