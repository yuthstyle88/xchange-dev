//! The per-source part of a market feed (pricing-tiers.md §4): which URLs to ask and how to read the answers.
//! Everything else (HTTP, timeout, headline, storage) is shared (`MarketRateSource`, `MarketProvider`), so a new
//! source is one `MarketFeed` impl in one file.

use crate::domain::pricing::market_quote::MarketSource;
use crate::domain::pricing::market_snapshot::MarketSnapshot;

/// What a request asks for (V.9 `market.mjs:78`: `Accept` JSON for TT, HTML for Super Rich).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketAccept {
    Json,
    Html,
}

/// One HTTP GET a feed needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketRequest {
    pub url: String,
    pub accept: MarketAccept,
    /// True when a failure of this request must not fail the source (V.9 `market.mjs:84`, TT's update time).
    pub optional: bool,
}

/// The answer to one `MarketRequest`, in the same order as `MarketFeed::requests`. `body` is `None` only for an
/// optional request that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketBody {
    pub url: String,
    pub body: Option<String>,
}

/// Why a source gave no snapshot. The text is V.9's (`TT: no rates`, `SR: exchangeList not found`, `HTTP 503`, …),
/// so the board can show it as the source's error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct MarketFeedError(pub String);

/// One market source. Pure: `parse` does no I/O and reads no clock.
pub trait MarketFeed: Send + Sync {
    fn source(&self) -> MarketSource;
    fn requests(&self) -> Vec<MarketRequest>;
    fn parse(&self, bodies: &[MarketBody]) -> Result<MarketSnapshot, MarketFeedError>;
}
