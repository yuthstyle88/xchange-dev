//! `MarketProvider`: the one `ReferenceProvider` for every market source (pricing-tiers.md §4).
//!
//! SKELETON (XC-PRICING PX-0): `row_key` and `parse` are PX-4's. Constructor and the two names stay.

use std::collections::HashSet;
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::pricing::market_quote::MarketSource;
use crate::domain::rate::entities::rate_source::ReferenceRates;
use crate::domain::reference::reference_provider::{ParsedBatch, ParsedLine, ReferenceProvider};

pub struct MarketProvider {
    pub(crate) source: MarketSource,
    /// The `currencies` table's codes: a line for any other code would break `rate_reference_lines`' foreign key.
    pub(crate) known: HashSet<String>,
}

impl MarketProvider {
    pub fn new(source: MarketSource, known: HashSet<String>) -> Self {
        Self { source, known }
    }
}

fn parse_decimal(val: &Value) -> Option<Decimal> {
    match val {
        Value::String(s) => Decimal::from_str(s).ok(),
        Value::Number(n) => Decimal::from_str(&n.to_string()).ok(),
        _ => None,
    }
}

impl ReferenceProvider for MarketProvider {
    fn provider(&self) -> &'static str {
        match self.source {
            MarketSource::Tt => "TT",
            MarketSource::SuperRich => "SR",
            MarketSource::Bot => "BOT",
        }
    }

    fn reference_type(&self) -> &'static str {
        match self.source {
            MarketSource::Tt => "MARKET_TT",
            MarketSource::SuperRich => "MARKET_SR",
            MarketSource::Bot => "BOT",
        }
    }

    /// A repeated TT / Super Rich answer confirms its batch (pricing-tiers.md §4).
    fn confirms_on_repeat(&self) -> bool {
        true
    }

    /// The row's `code`.
    fn row_key(&self, row: &Value) -> String {
        row.get("code")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    }

    /// One line per `MarketRateSource` row whose code is in `known` (others dropped, not an error): `unit_size` 1,
    /// `buy`/`sell` from the row, `raw` the row. `source_date` = the date part of `source_updated_at` if it parses,
    /// else `fetched_at`'s date; `source_published_at` = `source_updated_at` if it is RFC 3339; `complete` true.
    /// No line left ⇒ `RateEngineError::RateUnavailable("<TT|SR>: no known currency")`.
    fn parse(&self, rates: &ReferenceRates) -> Result<ParsedBatch, RateEngineError> {
        let updated_str = rates
            .rows
            .iter()
            .find_map(|r| r.get("source_updated_at").and_then(|v| v.as_str()));

        let source_published_at = updated_str
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let source_date = updated_str
            .and_then(|s| s.get(..10))
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .or_else(|| {
                DateTime::parse_from_rfc3339(&rates.fetched_at)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc).date_naive())
            })
            .unwrap_or_else(|| Utc::now().date_naive());

        let mut lines = Vec::new();
        for r in &rates.rows {
            let code = self.row_key(r);
            if !self.known.contains(&code) {
                continue;
            }
            lines.push(ParsedLine {
                currency: code,
                unit_size: Decimal::ONE,
                buy: r.get("buy").and_then(parse_decimal),
                sell: r.get("sell").and_then(parse_decimal),
                raw: r.clone(),
                ..Default::default()
            });
        }

        if lines.is_empty() {
            return Err(RateEngineError::RateUnavailable(format!(
                "{}: no known currency",
                self.provider()
            )));
        }

        Ok(ParsedBatch {
            source_date,
            source_published_at,
            complete: true,
            lines,
        })
    }
}
