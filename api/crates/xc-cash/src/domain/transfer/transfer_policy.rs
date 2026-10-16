//! `TransferPolicy`: per-currency transfer limits set by EXECUTIVE (D:§4.1, A5).

use std::collections::HashMap;

use xc_kernel::money::{CurrencyCode, Money};

use crate::domain::transfer::cash_transfer::{TransferError, TransferLine, add_line_minor};

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
    pub fn is_within_limit(&self, lines: &[TransferLine]) -> Result<bool, TransferError> {
        let mut currency_totals: HashMap<CurrencyCode, i64> = HashMap::new();
        for line in lines {
            if line.qty_sent > 0 {
                add_line_minor(
                    &mut currency_totals,
                    line.currency,
                    line.face_value,
                    line.qty_sent,
                )?;
            }
        }

        if currency_totals.is_empty() {
            return Ok(false);
        }

        for (cur, total_minor) in currency_totals {
            match self.limits.get(&cur) {
                Some(limit) => {
                    let limit_minor = limit.try_minor().map_err(|e| {
                        TransferError::Internal(format!(
                            "stored policy limit for {cur} overflows i64: {e}"
                        ))
                    })?;
                    if total_minor > limit_minor {
                        return Ok(false);
                    }
                }
                None => return Ok(false),
            }
        }

        Ok(true)
    }
}
