//! The pricing board's pure domain (pricing-tiers.md §5): market quotes, flags, statuses, suggestions.
//! No I/O, no clock (the caller passes `now`), decimals only (never `f64`, `rate-display.md` B7).

pub mod market_quote;
pub mod pricing_flag;
pub mod pricing_suggestion;
pub mod row_status;
