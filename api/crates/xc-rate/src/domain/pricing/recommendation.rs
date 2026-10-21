//! V.9's owner recommendations `recommend` (`engine.mjs:154-198`; pricing-tiers.md §5.2).
//!
//! SKELETON (XC-PRICING PX-0): `Recommendation::list` is PX-8's. Types and signature stay.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::domain::pricing::market_quote::MarketQuote;

/// Sorted in this order (`engine.mjs:196`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    High,
    Warn,
    Medium,
    Low,
}

/// The fixed set the FE labels (pricing-tiers.md §5.2 table). `Debug` is the wire text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecommendationCode {
    TtStale,
    SrStale,
    SellAboveMarket,
    SellBelowMarket,
    BuyBelowMarket,
    BuyAboveMarket,
    SpreadWide,
    BelowPartnerCost,
    MarketMovedSinceUpdate,
    NotOffered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecommendationSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recommendation {
    pub severity: Severity,
    pub code: RecommendationCode,
    pub currency: Option<String>,
    pub side: Option<RecommendationSide>,
    /// The gap in % against the anchor, rounded to 2 places, sign kept (market-gap codes only).
    pub gap_pct: Option<Decimal>,
    /// The anchor price V.9 offers as the fix (market-gap codes only), per one unit.
    pub value: Option<Decimal>,
    /// The currency's THB volume over the last 7 days (0 when none), used for the order.
    pub volume_thb: Decimal,
}

/// One enabled currency's counter rate, per one unit.
#[derive(Debug, Clone, PartialEq)]
pub struct CounterRate {
    pub currency: String,
    pub buy: Decimal,
    pub sell: Decimal,
    pub partner_cost: Option<Decimal>,
    /// When the counter rate was published (V.9's `updated`).
    pub published_at: Option<DateTime<Utc>>,
    /// The currency's anchor (`MarketQuote::anchor_for`), per one unit; `None` ⇒ no market rows for it.
    pub anchor: Option<MarketQuote>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RecommendationInput {
    pub tt_stale: bool,
    pub sr_stale: bool,
    /// TT's `source_updated_at` (V.9 `market.TT.sourceUpdatedAt`).
    pub tt_source_updated_at: Option<DateTime<Utc>>,
    pub counters: Vec<CounterRate>,
    /// Currencies TT and Super Rich both list.
    pub listed_by_both: Vec<String>,
    /// Currencies the branch sells.
    pub enabled: HashSet<String>,
    /// THB volume per currency over the last 7 days (non-cancelled, LIVE).
    pub volume_thb: HashMap<String, Decimal>,
}

impl Recommendation {
    /// Every recommendation, highest severity first, then the largest 7-day volume (`engine.mjs:196-197`).
    pub fn list(input: &RecommendationInput) -> Vec<Recommendation> {
        let _ = input;
        todo!("PX-8: Recommendation::list")
    }
}
