//! `MarketFeedStatusStore`: each market source's last fetch attempt and error, in `market_feed_status`
//! (pricing-tiers.md §4, §12.1; V.9 keeps failed fetches in `market_snapshots`, `src/pricing/market.mjs:99-113`).
//!
//! SKELETON (XC-PRICING PX-0): `record` and `all` are PX-5's. Types and signatures stay.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::pricing::market_quote::MarketSource;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketAttempt {
    pub at: DateTime<Utc>,
    /// `None` after a success; the error text cut to 200 characters (`market.mjs:110`) after a failure.
    pub error: Option<String>,
}

pub struct MarketFeedStatusStore;

impl MarketFeedStatusStore {
    /// Upserts the source's row (`'TT'` / `'SR'`): `last_attempt_at = at`, `last_error` = the error cut to its first
    /// 200 characters, or NULL.
    pub async fn record(
        pool: &PgPool,
        source: MarketSource,
        at: DateTime<Utc>,
        error: Option<&str>,
    ) -> Result<(), RateEngineError> {
        let _ = (pool, source, at, error);
        todo!("PX-5: MarketFeedStatusStore::record")
    }

    /// Every stored row, keyed by source.
    pub async fn all(
        conn: &mut PgConnection,
    ) -> Result<HashMap<MarketSource, MarketAttempt>, RateEngineError> {
        let _ = conn;
        todo!("PX-5: MarketFeedStatusStore::all")
    }
}
