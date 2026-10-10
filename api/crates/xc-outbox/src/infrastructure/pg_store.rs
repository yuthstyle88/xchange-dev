//! `PgOutboxStore`: `OutboxStore` on PostgreSQL (migration `20261110110000_outbox.sql`).

use std::time::Duration;

use futures::future::BoxFuture;
use pos_error::AppResult;
use serde_json::Value;
use sqlx::PgPool;

use crate::domain::event::{OutboxEvent, OutboxStatus};
use crate::domain::handler::{FinalOutcome, OutboxHandler};
use crate::domain::store::{OutboxStore, RecordStep};

#[derive(Clone)]
pub struct PgOutboxStore {
    pool: PgPool,
}

#[derive(sqlx::FromRow)]
struct ClaimedRow {
    id: String,
    seq: i64,
    organization_id: String,
    event_type: String,
    destination: String,
    aggregate_type: String,
    aggregate_id: String,
    idempotency_key: String,
    payload_redacted: Value,
    status: String,
    attempts: i32,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl PgOutboxStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl OutboxStore for PgOutboxStore {
    fn claim_due(&self, limit: i64, lease: Duration) -> BoxFuture<'_, AppResult<Vec<OutboxEvent>>> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await?;
            let rows: Vec<ClaimedRow> = sqlx::query_as(
                "WITH due_candidates AS (
                    SELECT o.id
                    FROM outbox_events o
                    WHERE o.status = 'PENDING'
                      AND o.next_attempt_at <= now()
                      AND NOT EXISTS (
                          SELECT 1 FROM outbox_events earlier
                          WHERE earlier.aggregate_type = o.aggregate_type
                            AND earlier.aggregate_id = o.aggregate_id
                            AND earlier.destination = o.destination
                            AND earlier.status = 'PENDING'
                            AND earlier.seq < o.seq
                      )
                    ORDER BY o.seq ASC
                    LIMIT $1
                    FOR UPDATE OF o SKIP LOCKED
                )
                UPDATE outbox_events o
                SET next_attempt_at = now() + ($2 * interval '1 second'),
                    attempts = attempts + 1
                FROM due_candidates c
                WHERE o.id = c.id
                RETURNING
                    o.id::text AS id,
                    o.seq,
                    o.organization_id::text AS organization_id,
                    o.event_type,
                    o.destination,
                    o.aggregate_type,
                    o.aggregate_id,
                    o.idempotency_key,
                    o.payload_redacted,
                    o.status,
                    o.attempts,
                    o.created_at",
            )
            .bind(limit)
            .bind(lease.as_secs_f64())
            .fetch_all(&mut *tx)
            .await?;

            tx.commit().await?;

            let mut events: Vec<OutboxEvent> = rows
                .into_iter()
                .map(|row| OutboxEvent {
                    id: row.id,
                    seq: row.seq,
                    organization_id: row.organization_id,
                    event_type: row.event_type,
                    destination: row.destination,
                    aggregate_type: row.aggregate_type,
                    aggregate_id: row.aggregate_id,
                    idempotency_key: row.idempotency_key,
                    payload_redacted: row.payload_redacted,
                    status: OutboxStatus::parse(&row.status).unwrap_or(OutboxStatus::Pending),
                    attempts: row.attempts,
                    created_at: row.created_at,
                })
                .collect();
            events.sort_by_key(|e| e.seq);
            Ok(events)
        })
    }

    fn record<'a>(
        &'a self,
        event: &'a OutboxEvent,
        step: RecordStep,
        handler: &'a dyn OutboxHandler,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            match step {
                RecordStep::Final(outcome) => {
                    let mut tx = self.pool.begin().await?;
                    let rows_affected = match &outcome {
                        FinalOutcome::Delivered { http_status } => sqlx::query(
                            "UPDATE outbox_events \
                                 SET status = 'DELIVERED', delivered_at = now(), \
                                     last_http_status = $2, last_error = NULL \
                                 WHERE id = $1::uuid AND status = 'PENDING' AND attempts = $3",
                        )
                        .bind(&event.id)
                        .bind(*http_status as i32)
                        .bind(event.attempts)
                        .execute(&mut *tx)
                        .await?
                        .rows_affected(),
                        FinalOutcome::Refused { http_status, error } => {
                            let truncated: String = error.chars().take(500).collect();
                            sqlx::query(
                                "UPDATE outbox_events \
                                 SET status = 'FAILED', last_http_status = $2, \
                                     last_error = $3, delivered_at = NULL \
                                 WHERE id = $1::uuid AND status = 'PENDING' AND attempts = $4",
                            )
                            .bind(&event.id)
                            .bind(*http_status as i32)
                            .bind(&truncated)
                            .bind(event.attempts)
                            .execute(&mut *tx)
                            .await?
                            .rows_affected()
                        }
                        FinalOutcome::Expired => sqlx::query(
                            "UPDATE outbox_events \
                                 SET status = 'EXPIRED', delivered_at = NULL \
                                 WHERE id = $1::uuid AND status = 'PENDING' AND attempts = $2",
                        )
                        .bind(&event.id)
                        .bind(event.attempts)
                        .execute(&mut *tx)
                        .await?
                        .rows_affected(),
                    };
                    if rows_affected == 0 {
                        tx.rollback().await?;
                        return Ok(());
                    }
                    handler.on_final(&mut tx, event, &outcome).await?;
                    tx.commit().await?;
                }
                RecordStep::Retry {
                    after,
                    http_status,
                    error,
                } => {
                    let truncated: String = error.chars().take(500).collect();
                    let status_i32 = http_status.map(|s| s as i32);
                    sqlx::query(
                        "UPDATE outbox_events \
                         SET status = 'PENDING', \
                             next_attempt_at = now() + ($2 * interval '1 second'), \
                             last_http_status = $3, \
                             last_error = $4, \
                             delivered_at = NULL \
                         WHERE id = $1::uuid AND status = 'PENDING' AND attempts = $5",
                    )
                    .bind(&event.id)
                    .bind(after.as_secs_f64())
                    .bind(status_i32)
                    .bind(&truncated)
                    .bind(event.attempts)
                    .execute(&self.pool)
                    .await?;
                }
            }
            Ok(())
        })
    }
}
