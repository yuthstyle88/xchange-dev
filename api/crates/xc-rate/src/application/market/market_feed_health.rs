//! `MarketFeedHealth`: each source's last attempt and error, kept in memory (pricing-tiers.md §4; V.9 keeps it in
//! `market_snapshots`, `market.mjs:99-113`, which v3 has no column for).
//!
//! SKELETON (XC-PRICING PX-0): `record` and `get` are PX-5's. Types and signatures stay.

use std::collections::HashMap;
use std::sync::RwLock;

use chrono::{DateTime, Utc};

use crate::domain::pricing::market_quote::MarketSource;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketAttempt {
    pub at: DateTime<Utc>,
    /// `None` after a success; the error text cut to 200 characters (`market.mjs:110`) after a failure.
    pub error: Option<String>,
}

#[allow(dead_code)] // the fields are read by PX-5; remove this line with the stub bodies
#[derive(Debug, Default)]
pub struct MarketFeedHealth {
    pub(crate) attempts: RwLock<HashMap<MarketSource, MarketAttempt>>,
}

impl MarketFeedHealth {
    /// Replaces the source's last attempt.
    pub fn record(&self, source: MarketSource, at: DateTime<Utc>, error: Option<String>) {
        let _ = (source, at, error);
        todo!("PX-5: MarketFeedHealth::record")
    }

    pub fn get(&self, source: MarketSource) -> Option<MarketAttempt> {
        let _ = source;
        todo!("PX-5: MarketFeedHealth::get")
    }
}
