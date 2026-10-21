//! `SuperRichFeed`: Super Rich Thailand, head office H01 (V.9 `src/pricing/market.mjs:8,30-63`).
//!
//! SKELETON (XC-PRICING PX-0): `parse` is PX-3's. URL, `requests` and signatures stay.

use crate::domain::pricing::market_feed::{
    MarketAccept, MarketBody, MarketFeed, MarketFeedError, MarketRequest,
};
use crate::domain::pricing::market_quote::MarketSource;
use crate::domain::pricing::market_snapshot::MarketSnapshot;

pub const SR_URL: &str = "https://www.superrichthailand.com/exchange-rate";

/// One request: the HTML page, whose rates are server-rendered into Next.js flight chunks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuperRichFeed {
    pub url: String,
}

impl SuperRichFeed {
    pub fn live() -> Self {
        Self::at(SR_URL)
    }

    pub fn at(url: &str) -> Self {
        Self {
            url: url.to_string(),
        }
    }
}

impl MarketFeed for SuperRichFeed {
    fn source(&self) -> MarketSource {
        MarketSource::SuperRich
    }

    fn requests(&self) -> Vec<MarketRequest> {
        vec![MarketRequest {
            url: self.url.clone(),
            accept: MarketAccept::Html,
            optional: false,
        }]
    }

    /// V.9 `parseSuperRich`: concatenate every decodable `self.__next_f.push([1,"…"])` string, cut the object after
    /// `"exchangeList":` by brace depth (quotes and escapes respected), read `"date":"YYYY-MM-DD"`. One row per
    /// (code, list index), codes `^[A-Z]{3,4}$`, `denomRem`/`buyText`/`sellText`; a row with neither price > 0 is
    /// dropped. Errors: `SR: exchangeList not found`, `SR: exchangeList is cut off`, `SR: no rates`.
    fn parse(&self, bodies: &[MarketBody]) -> Result<MarketSnapshot, MarketFeedError> {
        let _ = bodies;
        todo!("PX-3: SuperRichFeed::parse")
    }
}
