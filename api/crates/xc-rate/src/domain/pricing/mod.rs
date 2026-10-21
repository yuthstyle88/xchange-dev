//! The pricing board's pure domain (pricing-tiers.md §5): market quotes, flags, statuses, suggestions.
//! No I/O, no clock (the caller passes `now`), decimals only (never `f64`, `rate-display.md` B7).

pub mod market_feed;
pub mod market_quote;
pub mod market_snapshot;
pub mod pricing_flag;
pub mod pricing_suggestion;
pub mod recommendation;
pub mod row_status;
pub mod tier_check;
