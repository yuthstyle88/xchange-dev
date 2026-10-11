use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_rate::application::reference::reference_batch_writer::ReferenceBatchWriter;
use xc_rate::domain::draft::rate_draft::DraftId;
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::publication::published_rates::PublicationRef;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait ReferenceFetch: Send + Sync {
    fn fetch_and_write<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
    ) -> BoxFuture<'a, Result<Uuid, RateEngineError>>;
}

impl ReferenceFetch for ReferenceBatchWriter {
    fn fetch_and_write<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
    ) -> BoxFuture<'a, Result<Uuid, RateEngineError>> {
        Box::pin(self.fetch_and_write(tx))
    }
}

pub trait BranchRefresh: Send + Sync {
    fn refresh<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        batch: Uuid,
    ) -> BoxFuture<'a, Result<RefreshOutcome, RateEngineError>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    Published(PublicationRef),
    AwaitingOwner(DraftId),
    UpToDate,
    NothingToCopy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateFeedConfig {
    pub interval: Duration,
}

impl RateFeedConfig {
    pub fn from_env() -> Self {
        const DEFAULT_SECS: u64 = 10_800;
        let interval = match std::env::var("RATE_FEED_INTERVAL_SECS") {
            Ok(val) => match val.parse::<u64>() {
                Ok(secs) if secs > 0 => Duration::from_secs(secs),
                _ => {
                    tracing::warn!(
                        "RATE_FEED_INTERVAL_SECS is invalid ({val:?}), using default {DEFAULT_SECS}s"
                    );
                    Duration::from_secs(DEFAULT_SECS)
                }
            },
            Err(_) => Duration::from_secs(DEFAULT_SECS),
        };
        Self { interval }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickReport {
    pub skipped: bool,
    pub batch: Option<Uuid>,
    pub fetch_error: Option<String>,
    pub refreshed: Vec<(String, RefreshOutcome)>,
    pub failed: Vec<(String, String)>,
}

pub const RATE_FEED_LOCK_KEY: i64 = 0x5241_5445_4645_4544; // b"RATEFEED"

pub struct RateFeedLoop {
    pub(crate) pool: PgPool,
    pub(crate) fetch: Arc<dyn ReferenceFetch>,
    pub(crate) refresh: Arc<dyn BranchRefresh>,
    pub(crate) config: RateFeedConfig,
}

impl RateFeedLoop {
    pub fn new(
        pool: PgPool,
        fetch: Arc<dyn ReferenceFetch>,
        refresh: Arc<dyn BranchRefresh>,
        config: RateFeedConfig,
    ) -> Self {
        Self {
            pool,
            fetch,
            refresh,
            config,
        }
    }

    pub async fn tick(&self) -> Result<TickReport, RateEngineError> {
        // 1. Take advisory transaction lock in its own transaction
        let mut lock_tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
            .bind(RATE_FEED_LOCK_KEY)
            .fetch_one(&mut *lock_tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

        if !locked {
            let _ = lock_tx.rollback().await;
            return Ok(TickReport {
                skipped: true,
                batch: None,
                fetch_error: None,
                refreshed: Vec::new(),
                failed: Vec::new(),
            });
        }

        // 2. Fetch once in a transaction of its own
        let mut fetch_tx = match self.pool.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                let _ = lock_tx.rollback().await;
                return Err(RateEngineError::Database(e.to_string()));
            }
        };

        let batch = match self.fetch.fetch_and_write(&mut fetch_tx).await {
            Ok(batch_id) => {
                if let Err(e) = fetch_tx.commit().await {
                    let _ = lock_tx.rollback().await;
                    return Err(RateEngineError::Database(e.to_string()));
                }
                batch_id
            }
            Err(e) => {
                tracing::error!("rate feed fetch error: {e}");
                let _ = fetch_tx.rollback().await;
                let _ = lock_tx.rollback().await;
                return Ok(TickReport {
                    skipped: false,
                    batch: None,
                    fetch_error: Some(e.to_string()),
                    refreshed: Vec::new(),
                    failed: Vec::new(),
                });
            }
        };

        // 3. For each branches.id with price_source = 'BOT_DRAFT' ordered by id
        let branch_ids: Vec<String> = match sqlx::query_scalar(
            "SELECT id FROM branches WHERE price_source = 'BOT_DRAFT' ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
        {
            Ok(ids) => ids,
            Err(e) => {
                let _ = lock_tx.rollback().await;
                return Err(RateEngineError::Database(e.to_string()));
            }
        };

        let mut refreshed = Vec::new();
        let mut failed = Vec::new();

        for branch_str in branch_ids {
            let branch = BranchId::new(&branch_str);
            let mut branch_tx = match self.pool.begin().await {
                Ok(tx) => tx,
                Err(e) => {
                    failed.push((branch_str, e.to_string()));
                    continue;
                }
            };

            match self.refresh.refresh(&mut branch_tx, &branch, batch).await {
                Ok(outcome) => {
                    if let Err(e) = branch_tx.commit().await {
                        failed.push((branch_str, e.to_string()));
                    } else {
                        refreshed.push((branch_str, outcome));
                    }
                }
                Err(e) => {
                    let _ = branch_tx.rollback().await;
                    failed.push((branch_str, e.to_string()));
                }
            }
        }

        let _ = lock_tx.rollback().await;

        Ok(TickReport {
            skipped: false,
            batch: Some(batch),
            fetch_error: None,
            refreshed,
            failed,
        })
    }

    pub async fn run(self: Arc<Self>, mut stop: tokio::sync::watch::Receiver<bool>) {
        if *stop.borrow() {
            return;
        }

        if let Err(e) = self.tick().await {
            tracing::error!("rate feed tick failed: {e}");
        }

        loop {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() {
                        break;
                    }
                }
                _ = tokio::time::sleep(self.config.interval) => {
                    if *stop.borrow() {
                        break;
                    }
                    if let Err(e) = self.tick().await {
                        tracing::error!("rate feed tick failed: {e}");
                    }
                }
            }
        }
    }
}
