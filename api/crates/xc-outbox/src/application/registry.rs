//! `OutboxRegistry` (design §5.2): `event_type` → handler. PL-0 builds it from the generated
//! list of `bootstrap/outbox_handlers/*.rs`, so adding a destination is one new file.

use std::collections::HashMap;
use std::sync::Arc;

use pos_error::AppResult;
use sqlx::PgPool;

use crate::domain::handler::OutboxHandler;

pub struct OutboxRegistry {
    handlers: HashMap<String, Arc<dyn OutboxHandler>>,
}

impl OutboxRegistry {
    /// Two handlers with the same `kind().event_type` fail the boot:
    /// `Err(AppError::Internal(..))` naming the event type.
    pub fn new(handlers: Vec<Arc<dyn OutboxHandler>>) -> AppResult<Self> {
        let mut map = HashMap::new();
        for handler in handlers {
            let event_type = handler.kind().event_type;
            if map.contains_key(event_type) {
                return Err(pos_error::AppError::Internal(format!(
                    "duplicate event_type in outbox registry: {event_type}"
                )));
            }
            map.insert(event_type.to_string(), handler);
        }
        Ok(Self { handlers: map })
    }

    /// The handler of `event_type`, if one is registered.
    pub fn handler(&self, event_type: &str) -> Option<Arc<dyn OutboxHandler>> {
        self.handlers.get(event_type).cloned()
    }

    /// Boot check: a `PENDING` row whose `event_type` has a handler but whose `destination` or
    /// `aggregate_type` differs from that handler's `EventKind` fails the boot
    /// (`Err(AppError::Internal(..))`). Rows of an event type with no handler pass.
    pub async fn verify_against(&self, pool: &PgPool) -> AppResult<()> {
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT DISTINCT event_type, destination, aggregate_type \
             FROM outbox_events WHERE status = 'PENDING'",
        )
        .fetch_all(pool)
        .await?;

        for (event_type, destination, aggregate_type) in rows {
            if let Some(handler) = self.handler(&event_type) {
                let kind = handler.kind();
                if kind.destination != destination || kind.aggregate_type != aggregate_type {
                    return Err(pos_error::AppError::Internal(format!(
                        "outbox event_type '{event_type}' has kind mismatch with pending rows: handler ({}, {}) vs row ({destination}, {aggregate_type})",
                        kind.destination, kind.aggregate_type
                    )));
                }
            }
        }
        Ok(())
    }
}
