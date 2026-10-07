//! Per-branch payment rules (PM-R).

use crate::money::Side;
use crate::payment::channel_rule::{ChannelRule, METHODS};

/// The payment rules of a branch as one pure value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentRules {
    rows: [ChannelRule; 5],
}

impl Default for PaymentRules {
    fn default() -> Self {
        Self::defaults()
    }
}

impl PaymentRules {
    /// Returns the default payment rules (all channels on, buy on Cash/Bank Transfer, sell on all).
    pub fn defaults() -> Self {
        Self {
            rows: [
                ChannelRule {
                    id: "Cash".to_string(),
                    enabled: true,
                    buy: true,
                    sell: true,
                    account: String::new(),
                    instruction: String::new(),
                },
                ChannelRule {
                    id: "Credit Card".to_string(),
                    enabled: true,
                    buy: false,
                    sell: true,
                    account: String::new(),
                    instruction: String::new(),
                },
                ChannelRule {
                    id: "Alipay".to_string(),
                    enabled: true,
                    buy: false,
                    sell: true,
                    account: String::new(),
                    instruction: String::new(),
                },
                ChannelRule {
                    id: "PromptPay".to_string(),
                    enabled: true,
                    buy: false,
                    sell: true,
                    account: String::new(),
                    instruction: String::new(),
                },
                ChannelRule {
                    id: "Bank Transfer".to_string(),
                    enabled: true,
                    buy: true,
                    sell: true,
                    account: String::new(),
                    instruction: "If transfer, call agent · หากโอนเงิน กรุณาติดต่อพนักงาน".to_string(),
                },
            ],
        }
    }

    /// Constructs payment rules from 5 rows, validating all business rules.
    pub fn from_rows(rows: Vec<ChannelRule>) -> Option<Self> {
        if rows.len() != 5 {
            return None;
        }

        // Each of the five channels must be present exactly once.
        let mut ordered_rows: [Option<ChannelRule>; 5] = [None, None, None, None, None];
        for row in rows {
            let idx = METHODS.iter().position(|&m| m == row.id)?;
            if ordered_rows[idx].is_some() {
                return None;
            }
            // BUY can only be set on Cash or Bank Transfer (whether channel is on or off).
            if row.buy && row.id != "Cash" && row.id != "Bank Transfer" {
                return None;
            }
            // A channel that is enabled must have at least one direction (buy or sell).
            if row.enabled && !row.buy && !row.sell {
                return None;
            }
            ordered_rows[idx] = Some(row);
        }

        let [Some(r0), Some(r1), Some(r2), Some(r3), Some(r4)] = ordered_rows else {
            return None;
        };
        let rows = [r0, r1, r2, r3, r4];

        // At least one channel must stay enabled for BUY.
        if !rows.iter().any(|r| r.enabled && r.buy) {
            return None;
        }
        // At least one channel must stay enabled for SELL.
        if !rows.iter().any(|r| r.enabled && r.sell) {
            return None;
        }

        Some(Self { rows })
    }

    /// Returns the 5 channel rules in standard `METHODS` order.
    pub fn rows(&self) -> &[ChannelRule; 5] {
        &self.rows
    }

    /// Looks up a channel rule by exact id.
    pub fn rule(&self, method: &str) -> Option<&ChannelRule> {
        self.rows.iter().find(|r| r.id == method)
    }

    /// Checks whether `method` is allowed for the given `side`.
    ///
    /// "SCB Transfer" is canonicalized to "Bank Transfer"; all other names are matched exactly.
    pub fn allows(&self, method: &str, side: Side) -> bool {
        let canonical = if method == "SCB Transfer" {
            "Bank Transfer"
        } else {
            method
        };
        self.rule(canonical).is_some_and(|r| {
            r.enabled
                && match side {
                    Side::Buy => r.buy,
                    Side::Sell => r.sell,
                }
        })
    }
}
