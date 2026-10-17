//! `TransferPolicy`: per-currency transfer limits set by EXECUTIVE (D:§4.1, A5).

use std::collections::HashMap;
use std::fmt;

use serde_json::Value;
use xc_kernel::money::{CurrencyCode, Money};

use crate::domain::transfer::cash_transfer::{TransferError, TransferLine, add_line_minor};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidLimit {
    pub currency: String,
    pub raw: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidLimits {
    pub entries: Vec<InvalidLimit>,
    pub parsed: HashMap<CurrencyCode, Money>,
}

impl fmt::Display for InvalidLimits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid transfer limits: ")?;
        for (i, entry) in self.entries.iter().enumerate() {
            if i > 0 {
                write!(f, "; ")?;
            }
            write!(f, "{:?}={} ({})", entry.currency, entry.raw, entry.reason)?;
        }
        Ok(())
    }
}

impl std::error::Error for InvalidLimits {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferPolicy {
    pub version: String,
    pub limits: HashMap<CurrencyCode, Money>,
}

impl TransferPolicy {
    pub fn new(version: String, limits: HashMap<CurrencyCode, Money>) -> Self {
        Self { version, limits }
    }

    /// Parses a stored `cash_transfer_policy_versions.limits` JSON value (D:§4.7).
    ///
    /// Accepts only a JSON object whose every key is an exact `CurrencyCode::parse` code (no
    /// trimming) and whose every value is a string that `Money::parse` reads. Collects every
    /// offending entry rather than stopping at the first.
    pub fn parse_limits(value: &Value) -> Result<HashMap<CurrencyCode, Money>, InvalidLimits> {
        let Some(map) = value.as_object() else {
            return Err(InvalidLimits {
                entries: vec![InvalidLimit {
                    currency: String::new(),
                    raw: value.to_string(),
                    reason: "stored limits is not a JSON object".into(),
                }],
                parsed: HashMap::new(),
            });
        };

        let mut parsed = HashMap::new();
        let mut entries = Vec::new();
        for (cur, val) in map {
            let Ok(code) = CurrencyCode::parse(cur) else {
                entries.push(InvalidLimit {
                    currency: cur.clone(),
                    raw: val.to_string(),
                    reason: "unknown currency code".into(),
                });
                continue;
            };
            let Some(amount_str) = val.as_str() else {
                entries.push(InvalidLimit {
                    currency: cur.clone(),
                    raw: val.to_string(),
                    reason: "limit must be a string".into(),
                });
                continue;
            };
            match Money::parse(amount_str) {
                Ok(m) => {
                    parsed.insert(code, m);
                }
                Err(e) => {
                    entries.push(InvalidLimit {
                        currency: cur.clone(),
                        raw: val.to_string(),
                        reason: e.to_string(),
                    });
                }
            }
        }

        if entries.is_empty() {
            Ok(parsed)
        } else {
            Err(InvalidLimits { entries, parsed })
        }
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
