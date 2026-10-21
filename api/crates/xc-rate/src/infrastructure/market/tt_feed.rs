//! `TtFeed`: TT Currency Exchange, Pattaya head office (V.9 `src/pricing/market.mjs:7,15-28`).
//!
//! SKELETON (XC-PRICING PX-0): `parse` is PX-2's. URLs, `requests` and signatures stay.

use crate::domain::pricing::market_feed::{
    MarketAccept, MarketBody, MarketFeed, MarketFeedError, MarketRequest,
};
use crate::domain::pricing::market_quote::MarketSource;
use crate::domain::pricing::market_snapshot::MarketSnapshot;

pub const TT_RATES_URL: &str =
    "https://api.software.ttexchange.com/currencies?is_main=false&branch_id=1";
pub const TT_UPDATED_URL: &str =
    "https://api.software.ttexchange.com/currencies/updated_at?branch_id=1";

/// Two requests: the rates (required, JSON array) and the update time (optional, `{"updated_at": "…"}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtFeed {
    pub rates_url: String,
    pub updated_url: String,
}

impl TtFeed {
    pub fn live() -> Self {
        Self::at(TT_RATES_URL, TT_UPDATED_URL)
    }

    pub fn at(rates_url: &str, updated_url: &str) -> Self {
        Self {
            rates_url: rates_url.to_string(),
            updated_url: updated_url.to_string(),
        }
    }
}

impl MarketFeed for TtFeed {
    fn source(&self) -> MarketSource {
        MarketSource::Tt
    }

    fn requests(&self) -> Vec<MarketRequest> {
        vec![
            MarketRequest {
                url: self.rates_url.clone(),
                accept: MarketAccept::Json,
                optional: false,
            },
            MarketRequest {
                url: self.updated_url.clone(),
                accept: MarketAccept::Json,
                optional: true,
            },
        ]
    }

    /// V.9 `parseTT`: `bodies[0]` the rates array, `bodies[1]` the update time (its body may be `None`).
    /// A row is `{name: "USD(L)", description, current_buy_rate, current_sell_rate, decimals}`; a name that is not
    /// `^[A-Z]{3}(\([LMS]\))?$` or a row with neither price > 0 is skipped. Errors: `TT: unexpected response`
    /// (not an array), `TT: no rates` (nothing left).
    fn parse(&self, bodies: &[MarketBody]) -> Result<MarketSnapshot, MarketFeedError> {
        let _ = bodies;
        todo!("PX-2: TtFeed::parse")
    }
}
