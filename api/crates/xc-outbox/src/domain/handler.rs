//! `OutboxHandler` (design §5.2): one implementation per destination and event type, in the
//! area that owns the event. The core never matches on `event_type` itself.

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};

use crate::domain::event::OutboxEvent;
use crate::domain::event_kind::EventKind;
use crate::domain::retry_policy::RetryPolicy;

/// The final answer for one event. `on_final` receives it once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalOutcome {
    /// The destination accepted it (2xx). The row turns `DELIVERED`.
    Delivered { http_status: u16 },
    /// The destination refused it for good (a final 4xx). The row turns `FAILED`.
    Refused { http_status: u16, error: String },
    /// It was not delivered within `RetryPolicy::expire_after`. The row turns `EXPIRED`.
    Expired,
}

/// What one delivery attempt produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryResult {
    Final(FinalOutcome),
    /// Try again after `RetryPolicy::backoff(attempts)`. `error` is stored redacted, at most
    /// 500 characters.
    Retry {
        http_status: Option<u16>,
        error: String,
    },
}

pub trait OutboxHandler: Send + Sync {
    /// The kind this handler delivers; the registry keys it by `kind().event_type`.
    fn kind(&self) -> &'static EventKind;

    /// The destination's own retry timing.
    fn retry_policy(&self) -> RetryPolicy {
        RetryPolicy::default()
    }

    /// Build the request from the source rows and send it **once**, carrying
    /// `event.idempotency_key`. No retry inside; the worker owns retries. Runs outside any
    /// database transaction.
    fn deliver<'a>(&'a self, event: &'a OutboxEvent) -> BoxFuture<'a, DeliveryResult>;

    /// After a final answer, record it on the source (for example
    /// `referral_attributions.broker_outcome`). Runs in the same transaction that sets the
    /// row's final status; an error rolls both back and the row stays `PENDING`.
    fn on_final<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        event: &'a OutboxEvent,
        outcome: &'a FinalOutcome,
    ) -> BoxFuture<'a, AppResult<()>>;
}
