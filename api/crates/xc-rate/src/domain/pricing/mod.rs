//! The pricing board's pure domain (pricing-tiers.md §5): market quotes, flags, statuses, suggestions.
//! No I/O, no clock (the caller passes `now`), decimals only (never `f64`, `rate-display.md` B7).

pub mod branch_policy;
pub mod display_settings;
pub mod follow_plan;
pub mod follow_visibility;
pub mod market_feed;
pub mod market_quote;
pub mod market_snapshot;
pub mod pricing_flag;
pub mod pricing_mode;
pub mod pricing_suggestion;
pub mod recommendation;
pub mod row_status;
pub mod suggestion_settings;
pub mod tier_check;
pub mod tier_set;
