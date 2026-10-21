//! `MarketRefresh`: every 5 minutes, fetch each market source and store its headline lines (pricing-tiers.md §4;
//! V.9 `refreshMarket`, `market.mjs:101-121`). It never refreshes a branch or writes a draft.
//!
//! SKELETON (XC-PRICING PX-0): `tick` and `run` are PX-5's. `run` returns at once until then, so the spawn in
//! `bootstrap/mod.rs` is inert. `MarketRefreshConfig` is final.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tokio::sync::watch;
use uuid::Uuid;

use crate::application::market::market_feed_health::MarketFeedHealth;
use crate::domain::pricing::market_quote::MarketSource;
use crate::domain::rate::entities::rate_source::RateSource;
use crate::infrastructure::reference::pg_reference_batch_store::PgReferenceBatchStore;

/// `MARKET_FEED_ENABLED=true` turns the loop on (anything else: off); `MARKET_FEED_INTERVAL_SECS`, default 300.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketRefreshConfig {
    pub enabled: bool,
    pub interval: Duration,
}

impl MarketRefreshConfig {
    pub const DEFAULT_INTERVAL_SECS: u64 = 300;

    pub fn from_env() -> Self {
        let enabled = std::env::var("MARKET_FEED_ENABLED").is_ok_and(|v| v.trim() == "true");
        let interval = std::env::var("MARKET_FEED_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|s| *s > 0)
            .unwrap_or(Self::DEFAULT_INTERVAL_SECS);
        Self {
            enabled,
            interval: Duration::from_secs(interval),
        }
    }
}

/// One tick's outcome per source, in the order of `MarketRefresh::new`'s `sources`: the batch id written (or the
/// existing one when the answer did not change), or the error text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketTickReport {
    pub results: Vec<(MarketSource, Result<Uuid, String>)>,
}

#[allow(dead_code)] // the fields are read by PX-5; remove this line with the stub bodies
pub struct MarketRefresh {
    pub(crate) pool: PgPool,
    pub(crate) sources: Vec<(MarketSource, Arc<dyn RateSource>)>,
    pub(crate) store: Arc<PgReferenceBatchStore>,
    pub(crate) health: Arc<MarketFeedHealth>,
}

impl MarketRefresh {
    /// `sources` are `MarketRateSource`s in production and fakes in tests.
    pub fn new(
        pool: PgPool,
        sources: Vec<(MarketSource, Arc<dyn RateSource>)>,
        health: Arc<MarketFeedHealth>,
    ) -> Self {
        Self {
            pool,
            sources,
            store: Arc::new(PgReferenceBatchStore::new()),
            health,
        }
    }

    /// For each source: load the `currencies` codes, run `ReferenceBatchWriter::new(source, store)
    /// .with_provider(MarketProvider::new(source, known))` in its own transaction, commit, and record the attempt in
    /// `health` (`now`, the error if any). One source failing never stops the next.
    pub async fn tick(&self, now: DateTime<Utc>) -> MarketTickReport {
        let _ = now;
        todo!("PX-5: MarketRefresh::tick")
    }

    /// `tick(Utc::now())` every `interval` until `stop` turns true.
    pub async fn run(self: Arc<Self>, interval: Duration, stop: watch::Receiver<bool>) {
        let _ = (interval, stop);
        // PX-5: the loop. Until then the spawn is inert.
    }
}
