//! `TransferPolicy`: per-currency transfer limits set by EXECUTIVE (D:§4.1, A5).

use std::collections::HashMap;

use xc_kernel::money::{CurrencyCode, Money};

use crate::domain::transfer::cash_transfer::TransferLine;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferPolicy {
    pub version: String,
    pub limits: HashMap<CurrencyCode, Money>,
}

impl TransferPolicy {
    pub fn new(version: String, limits: HashMap<CurrencyCode, Money>) -> Self {
        Self { version, limits }
    }

    /// Checks whether the transfer is within limits for all currencies it moves.
    ///
    /// A transfer is within the limit when, for every currency it moves,
    /// Σ `qty_sent` × `face_value` ≤ that currency's limit.
    /// A currency with no limit, or an empty policy, is considered above the limit.
    pub fn is_within_limit(&self, lines: &[TransferLine]) -> bool {
        let mut currency_totals: HashMap<CurrencyCode, i64> = HashMap::new();
        for line in lines {
            if line.qty_sent > 0 {
                let line_minor = line.face_value.minor() * (line.qty_sent as i64);
                *currency_totals.entry(line.currency).or_insert(0) += line_minor;
            }
        }

        if currency_totals.is_empty() {
            return false;
        }

        for (cur, total_minor) in currency_totals {
            match self.limits.get(&cur) {
                Some(limit) => {
                    if total_minor > limit.minor() {
                        return false;
                    }
                }
                None => return false,
            }
        }

        true
    }
}
