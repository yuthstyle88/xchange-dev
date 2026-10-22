//! A parsed market answer, its headline quote per currency, and staleness (pricing-tiers.md §4).
//!
//! SKELETON (XC-PRICING PX-0): `headline` and `is_stale` are PX-1's. Types, constants and signatures stay.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;

use crate::domain::pricing::market_quote::MarketSource;

/// One row of a source's answer, per one unit of the currency (V.9 quotes are per unit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketRow {
    pub code: String,
    /// TT: `L` · `M` · `S` · `""` (no band). Super Rich: the row's index in its list as text (`"0"`, `"1"`, …).
    pub band: String,
    /// The denominations the quote is for (TT `description`, Super Rich `denomRem`).
    pub denoms: String,
    pub buy: Option<Decimal>,
    pub sell: Option<Decimal>,
    /// TT's `decimals`; `None` for Super Rich.
    pub decimals: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketSnapshot {
    pub source: MarketSource,
    /// TT's `updated_at`, Super Rich's page `date`; text as the source gave it.
    pub source_updated_at: Option<String>,
    pub rows: Vec<MarketRow>,
}

impl MarketSnapshot {
    /// V.9's `headline` (`market.mjs:65-75`): one row per currency, the large-note quote. TT ranks bands
    /// `L`, `""`, `M`, `S` (anything else last); Super Rich ranks by its list index (lowest first). Sorted by code.
    pub fn headline(&self) -> Vec<MarketRow> {
        let mut best: BTreeMap<&str, (u32, &MarketRow)> = BTreeMap::new();
        for row in &self.rows {
            let rank = match self.source {
                MarketSource::Tt => match row.band.as_str() {
                    "L" => 0,
                    "" => 1,
                    "M" => 2,
                    "S" => 3,
                    _ => 9,
                },
                MarketSource::SuperRich | MarketSource::Bot => {
                    row.band.parse::<u32>().unwrap_or(u32::MAX)
                }
            };
            if best
                .get(row.code.as_str())
                .is_none_or(|(cur_rank, _)| rank < *cur_rank)
            {
                best.insert(row.code.as_str(), (rank, row));
            }
        }
        best.into_values().map(|(_, row)| row.clone()).collect()
    }
}

pub struct MarketStaleness;

impl MarketStaleness {
    /// V.9 `STALE_AFTER_MS` (`market.mjs:10`).
    pub const STALE_AFTER_MINUTES: i64 = 30;
    /// V.9 `REFRESH_AFTER_MS` (`market.mjs:10`).
    pub const REFRESH_AFTER_MINUTES: i64 = 5;

    pub fn stale_after() -> Duration {
        Duration::minutes(Self::STALE_AFTER_MINUTES)
    }

    /// True when there is no good snapshot, or the newest one is older than 30 minutes (`market.mjs:133`).
    pub fn is_stale(fetched_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
        fetched_at.is_none_or(|t| now - t > Self::stale_after())
    }
}
