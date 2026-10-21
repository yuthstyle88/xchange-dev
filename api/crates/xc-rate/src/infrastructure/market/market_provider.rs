//! `MarketProvider`: the one `ReferenceProvider` for every market source (pricing-tiers.md §4).
//!
//! SKELETON (XC-PRICING PX-0): `row_key` and `parse` are PX-4's. Constructor and the two names stay.

use std::collections::HashSet;

use serde_json::Value;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::pricing::market_quote::MarketSource;
use crate::domain::rate::entities::rate_source::ReferenceRates;
use crate::domain::reference::reference_provider::{ParsedBatch, ReferenceProvider};

#[allow(dead_code)] // the fields are read by PX-4; remove this line with the stub bodies
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
        let _ = row;
        todo!("PX-4: MarketProvider::row_key")
    }

    /// One line per `MarketRateSource` row whose code is in `known` (others dropped, not an error): `unit_size` 1,
    /// `buy`/`sell` from the row, `raw` the row. `source_date` = the date part of `source_updated_at` if it parses,
    /// else `fetched_at`'s date; `source_published_at` = `source_updated_at` if it is RFC 3339; `complete` true.
    /// No line left ⇒ `RateEngineError::RateUnavailable("<TT|SR>: no known currency")`.
    fn parse(&self, rates: &ReferenceRates) -> Result<ParsedBatch, RateEngineError> {
        let _ = rates;
        todo!("PX-4: MarketProvider::parse")
    }
}
