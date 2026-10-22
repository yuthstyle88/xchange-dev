//! `MarketRefresh`: every 5 minutes, fetch each market source and store its headline lines (pricing-tiers.md §4;
//! V.9 `refreshMarket`, `market.mjs:101-121`). It never refreshes a branch or writes a draft.
//!
//! `run` ticks at once, then every interval, until `stop` turns true; `bootstrap/mod.rs` spawns it only when
//! `MARKET_FEED_ENABLED=true` (PX-5, #4389).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;
use uuid::Uuid;

use crate::application::reference::reference_batch_writer::ReferenceBatchWriter;
use crate::domain::pricing::market_quote::MarketSource;
use crate::domain::rate::entities::rate_source::RateSource;
use crate::infrastructure::market::market_feed_status_store::MarketFeedStatusStore;
use crate::infrastructure::market::market_provider::MarketProvider;
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

pub struct MarketRefresh {
    pub(crate) pool: PgPool,
    pub(crate) sources: Vec<(MarketSource, Arc<dyn RateSource>)>,
    pub(crate) store: Arc<PgReferenceBatchStore>,
}

impl MarketRefresh {
    /// `sources` are `MarketRateSource`s in production and fakes in tests.
    pub fn new(pool: PgPool, sources: Vec<(MarketSource, Arc<dyn RateSource>)>) -> Self {
        Self {
            pool,
            sources,
            store: Arc::new(PgReferenceBatchStore::new()),
        }
    }

    /// For each source: load the `currencies` codes, run `ReferenceBatchWriter::new(source, store)
    /// .with_provider(MarketProvider::new(source, known))` in its own transaction, commit, and record the attempt with
    /// `MarketFeedStatusStore::record` (`now`, the error if any). One source failing never stops the next.
    pub async fn tick(&self, now: DateTime<Utc>) -> MarketTickReport {
        let mut results = Vec::with_capacity(self.sources.len());
        for (source, rate_source) in &self.sources {
            let source = *source;
            let outcome: Result<Uuid, String> = async {
                let codes: Vec<String> = sqlx::query_scalar("SELECT trim(code) FROM currencies")
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| e.to_string())?;
                let known: HashSet<String> = codes.into_iter().collect();

                let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
                let writer = ReferenceBatchWriter::new(rate_source.clone(), self.store.clone())
                    .with_provider(Arc::new(MarketProvider::new(source, known)));
                match writer.fetch_and_write(&mut tx).await {
                    Ok(batch_id) => {
                        tx.commit().await.map_err(|e| e.to_string())?;
                        Ok(batch_id)
                    }
                    Err(e) => {
                        let _ = tx.rollback().await;
                        Err(e.to_string())
                    }
                }
            }
            .await;

            let err_ref = outcome.as_ref().err().map(String::as_str);
            if let Err(e) = MarketFeedStatusStore::record(&self.pool, source, now, err_ref).await {
                tracing::warn!(?source, error = %e, "failed to record market feed status");
            }
            results.push((source, outcome));
        }
        MarketTickReport { results }
    }

    /// `tick(Utc::now())` every `interval` until `stop` turns true.
    pub async fn run(self: Arc<Self>, interval: Duration, mut stop: watch::Receiver<bool>) {
        if *stop.borrow() {
            return;
        }
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() {
                        break;
                    }
                }
                _ = ticker.tick() => {
                    if *stop.borrow() {
                        break;
                    }
                    let report = self.tick(Utc::now()).await;
                    for (source, result) in report.results {
                        if let Err(err) = result {
                            tracing::warn!(?source, error = %err, "market refresh source failed");
                        }
                    }
                }
            }
        }
    }
}
