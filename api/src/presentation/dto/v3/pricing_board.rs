//! Pricing board DTOs (pricing-tiers.md §6, §9). Decimals are strings end to end (`rate-display.md` B7).
//!
//! Written in the skeleton (PT-0): the FE half (`feat/fe-pricing-board`) codes against these generated types.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `GET /api/v3/views/pricing_board?branch=`: one branch's board.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_board.ts")]
pub struct PricingBoardData {
    pub branch: String,
    /// The policy version the board was built on; `pricingApprove` / `pricingConfigure` send it back (lost update).
    pub policy_version_id: Option<String>,
    pub value_tiers_on: bool,
    /// The branch's AUTO anchor: `BOT` · `MARKET_TT` · `MARKET_SR` (§3.1).
    pub anchor_source: String,
    pub bands: Vec<PricingBandDto>,
    pub sources: Vec<PricingSourceDto>,
    pub rows: Vec<PricingRowDto>,
    /// V.9's `recommend` (pricing-tiers.md §5.2).
    pub recommendations: Vec<PricingRecommendationDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_board.ts")]
pub struct PricingBandDto {
    pub tier_no: u8,
    pub lower_exclusive_thb: Option<String>,
    pub upper_inclusive_thb: Option<String>,
}

/// One market source's state: `TT` · `SR` · `BOT`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_board.ts")]
pub struct PricingSourceDto {
    pub source: String,
    pub fetched_at: Option<String>,
    pub stale: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_board.ts")]
pub struct PricingRowDto {
    pub currency: String,
    pub unit_size: String,
    pub enabled: bool,
    /// The effective anchor for this currency (§3.1).
    pub anchor_source: String,
    pub quotes: Vec<PricingQuoteDto>,
    /// The current publication's tier rates (one row per tier; one row `tierNo: 0` when tiers are off).
    pub published: Vec<PricingTierRateDto>,
    /// The effective margins (the currency's override, else the branch's).
    pub margins: Vec<PricingTierMarginDto>,
    pub overridden: bool,
    pub min_spread_pct: Option<String>,
    /// The 4 effective `better` % values the suggestion uses (pricing-tiers.md §5.2), decimal strings.
    pub better_pct: Vec<String>,
    /// Where `betterPct` comes from: `currency` · `branch` · `default` (V.9's).
    pub better_pct_from: String,
    /// V.9's suggested tier prices at the unit size (`suggestTiers`); `null` when there is no anchor.
    pub suggestion: Option<Vec<PricingTierRateDto>>,
    /// `NoAnchor` · `NoTtQuote` · `StaleSource` · `MinSpread` · `BelowCost` · `LargeMove` · `Precision`.
    pub flags: Vec<String>,
    /// True when no flag keeps the row out of bulk approve.
    pub selectable: bool,
    /// `Current` · `Pending` · `Unpriced` · `Tiers` · `Review` · `Manual` · `Reference`.
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_board.ts")]
pub struct PricingQuoteDto {
    pub source: String,
    pub buy: Option<String>,
    pub sell: Option<String>,
    pub fetched_at: String,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_board.ts")]
pub struct PricingTierRateDto {
    pub tier_no: u8,
    pub buy: Option<String>,
    pub sell: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_board.ts")]
pub struct PricingTierMarginDto {
    pub tier_no: u8,
    pub buy_margin_pct: String,
    pub sell_margin_pct: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_board.ts")]
pub struct PricingRecommendationDto {
    /// `High` · `Warn` · `Medium` · `Low`.
    pub severity: String,
    pub code: String,
    pub currency: Option<String>,
    pub side: Option<String>,
}
