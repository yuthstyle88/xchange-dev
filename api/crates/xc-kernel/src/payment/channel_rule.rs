//! Per-branch payment channel rule and methods list (PM-R).

/// The five payment channels in v2's order.
pub const METHODS: [&str; 5] = [
    "Cash",
    "Credit Card",
    "Alipay",
    "PromptPay",
    "Bank Transfer",
];

/// A single payment channel's rule for a branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRule {
    pub id: String,
    pub enabled: bool,
    pub buy: bool,
    pub sell: bool,
    pub account: String,
    pub instruction: String,
}
