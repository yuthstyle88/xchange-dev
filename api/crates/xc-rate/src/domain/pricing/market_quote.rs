//! Market quotes for the pricing board (pricing-tiers.md §4, §5.1, §5.2).
//!
//! SKELETON (PT-0): `MarketQuote::anchor_for` is PT-8's. The types and signature stay.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use xc_kernel::money::Rate;

use crate::domain::pricing::pricing_flag::PricingFlag;

/// Where a quote comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarketSource {
    Tt,
    SuperRich,
    Bot,
}

/// One source's quote for one currency, per its unit size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketQuote {
    pub source: MarketSource,
    pub buy: Option<Rate>,
    pub sell: Option<Rate>,
    pub unit_size: Decimal,
    pub fetched_at: DateTime<Utc>,
    pub stale: bool,
}

impl MarketQuote {
    /// V.9's `anchorFor` (§5.2 `[required]`): the TT quote if there is one; else the Super Rich quote, with
    /// `NoTtQuote`; else no anchor, with `NoAnchor`. A stale anchor also adds `StaleSource`. BoT quotes are never an
    /// anchor here (they are the draft path's base, not the market).
    pub fn anchor_for(quotes: &[MarketQuote]) -> (Option<&MarketQuote>, Vec<PricingFlag>) {
        let (anchor, mut flags) =
            if let Some(tt) = quotes.iter().find(|q| q.source == MarketSource::Tt) {
                (Some(tt), Vec::new())
            } else if let Some(sr) = quotes.iter().find(|q| q.source == MarketSource::SuperRich) {
                (Some(sr), vec![PricingFlag::NoTtQuote])
            } else {
                return (None, vec![PricingFlag::NoAnchor]);
            };
        if let Some(q) = anchor
            && q.stale
        {
            flags.push(PricingFlag::StaleSource);
        }
        (anchor, flags)
    }
}
