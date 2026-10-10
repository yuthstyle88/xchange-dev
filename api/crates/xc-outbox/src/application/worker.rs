//! `OutboxWorker` (design §5.2): claims due rows, delivers each through its handler outside any
//! transaction, and records the answer. Delivery is at least once.

use std::sync::Arc;
use std::time::Duration;

use pos_error::AppResult;

use crate::application::registry::OutboxRegistry;
use crate::domain::handler::{DeliveryResult, FinalOutcome};
use crate::domain::store::{OutboxStore, RecordStep};

/// The lease a claim puts on a row: a worker that dies holding it gives it back after this.
pub const LEASE: Duration = Duration::from_secs(60);
/// Rows claimed per tick.
pub const BATCH: i64 = 20;
/// The pause between ticks of `run_forever`.
pub const TICK: Duration = Duration::from_secs(5);

/// What one tick did, for logs and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickReport {
    pub delivered: usize,
    pub failed: usize,
    pub retried: usize,
    pub expired: usize,
    /// Rows claimed whose event type has no handler. They stay `PENDING`.
    pub unhandled: usize,
}

pub struct OutboxWorker {
    store: Arc<dyn OutboxStore>,
    registry: Arc<OutboxRegistry>,
}

impl OutboxWorker {
    pub fn new(store: Arc<dyn OutboxStore>, registry: Arc<OutboxRegistry>) -> Self {
        Self { store, registry }
    }

    /// One pass: claim (`BATCH`, `LEASE`), then for each row:
    /// - older than its handler's `expire_after` ⇒ not delivered; `EXPIRED` + `on_final(Expired)`;
    /// - no handler ⇒ stays `PENDING`, one warning, no panic;
    /// - otherwise `deliver`, then `Final` ⇒ `DELIVERED`/`FAILED` + `on_final`, or
    ///   `Retry` ⇒ `next_attempt_at = now() + retry_policy().backoff(attempts)`.
    pub async fn tick(&self) -> AppResult<TickReport> {
        let events = self.store.claim_due(BATCH, LEASE).await?;
        let mut report = TickReport::default();

        for event in events {
            let handler = match self.registry.handler(&event.event_type) {
                Some(h) => h,
                None => {
                    tracing::warn!(
                        event_type = %event.event_type,
                        id = %event.id,
                        "unhandled outbox event type"
                    );
                    report.unhandled += 1;
                    continue;
                }
            };

            let policy = handler.retry_policy();
            let now = chrono::Utc::now();
            let is_expired = match chrono::Duration::from_std(policy.expire_after) {
                Ok(dur) => (now - event.created_at) > dur,
                Err(_) => false,
            };

            if is_expired {
                self.store
                    .record(
                        &event,
                        RecordStep::Final(FinalOutcome::Expired),
                        handler.as_ref(),
                    )
                    .await?;
                report.expired += 1;
                continue;
            }

            match handler.deliver(&event).await {
                DeliveryResult::Final(outcome) => {
                    let delivered = matches!(outcome, FinalOutcome::Delivered { .. });
                    let failed = matches!(outcome, FinalOutcome::Refused { .. });
                    let expired = matches!(outcome, FinalOutcome::Expired);

                    self.store
                        .record(&event, RecordStep::Final(outcome), handler.as_ref())
                        .await?;

                    if delivered {
                        report.delivered += 1;
                    } else if failed {
                        report.failed += 1;
                    } else if expired {
                        report.expired += 1;
                    }
                }
                DeliveryResult::Retry { http_status, error } => {
                    let wait = policy.backoff(event.attempts);
                    self.store
                        .record(
                            &event,
                            RecordStep::Retry {
                                after: wait,
                                http_status,
                                error,
                            },
                            handler.as_ref(),
                        )
                        .await?;
                    report.retried += 1;
                }
            }
        }

        Ok(report)
    }

    /// `tick` every `TICK`, forever; an error is logged and the loop goes on. PL-0 spawns it.
    pub async fn run_forever(self) {
        loop {
            if let Err(e) = self.tick().await {
                tracing::error!("outbox worker error in tick: {e}");
            }
            tokio::time::sleep(TICK).await;
        }
    }
}
