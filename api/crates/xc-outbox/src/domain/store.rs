//! `OutboxStore` (design §5.2, §8): the worker's port onto `outbox_events`. [proposed] The
//! method set is internal to the core; the tests drive the worker, not this trait, except
//! `claim_due`, which the lease test calls directly to stand for a crashed worker.

use std::time::Duration;

use futures::future::BoxFuture;
use pos_error::AppResult;

use crate::domain::event::OutboxEvent;
use crate::domain::handler::{FinalOutcome, OutboxHandler};

/// How one claimed event ends this attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordStep {
    /// Set the final status and call `handler.on_final`, in one transaction.
    Final(FinalOutcome),
    /// Stay `PENDING`; next attempt after `after`.
    Retry {
        after: Duration,
        http_status: Option<u16>,
        error: String,
    },
}

pub trait OutboxStore: Send + Sync {
    /// In one short transaction: take up to `limit` `PENDING` rows whose `next_attempt_at` has
    /// passed and that have no earlier `PENDING` row of the same
    /// `(aggregate_type, aggregate_id, destination)`, in `seq` order, `FOR UPDATE SKIP LOCKED`;
    /// set `next_attempt_at = now() + lease` and `attempts = attempts + 1`; commit.
    fn claim_due(&self, limit: i64, lease: Duration) -> BoxFuture<'_, AppResult<Vec<OutboxEvent>>>;

    /// Record one attempt's end. For `Final`, the status change and `handler.on_final` share
    /// one transaction.
    fn record<'a>(
        &'a self,
        event: &'a OutboxEvent,
        step: RecordStep,
        handler: &'a dyn OutboxHandler,
    ) -> BoxFuture<'a, AppResult<()>>;
}
