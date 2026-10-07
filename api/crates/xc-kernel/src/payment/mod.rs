//! Per-branch payment rules and channel configuration (PM-R).

pub mod channel_rule;
pub mod payment_rules;

pub use channel_rule::{ChannelRule, METHODS};
pub use payment_rules::PaymentRules;
