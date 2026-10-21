//! TT Pattaya and Super Rich feeds (pricing-tiers.md §4): one HTTP core, one provider, one `MarketFeed` per source.

pub mod market_provider;
pub mod market_rate_source;
pub mod superrich_feed;
pub mod tt_feed;
