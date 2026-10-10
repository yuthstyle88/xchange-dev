//! `OutboxWriter` (design §5.2): the only way an event enters `outbox_events`. It runs in the
//! caller's transaction, so a rollback of the business work writes no event.

use std::collections::BTreeMap;

use pos_error::AppResult;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use xc_approval::domain::redactor::Redactor;

use crate::domain::event_kind::EventKind;

pub struct OutboxWriter;

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted: BTreeMap<String, Value> = BTreeMap::new();
            for (k, v) in map {
                sorted.insert(k.clone(), canonical_json(v));
            }
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonical_json).collect()),
        other => other.clone(),
    }
}

fn hash_payload(payload: &Value) -> String {
    let canonical = canonical_json(payload);
    let s = serde_json::to_string(&canonical).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    format!("{:x}", hasher.finalize())
}

impl OutboxWriter {
    /// Enqueue one event in the caller's transaction and answer its `outbox_events.id`.
    ///
    /// - `organization_id` is the organisation's uuid as text.
    /// - `payload` holds references only. It is stored through `xc_approval`'s `Redactor` as
    ///   `payload_redacted`; `payload_hash` is the sha256 of the **unredacted** payload in
    ///   canonical form (object keys sorted), so the same content in another key order is the
    ///   same hash.
    /// - The same `(organization_id, kind.event_type, key)` with the same hash is a no-op that
    ///   answers the existing id.
    /// - The same key with another hash is `Err(AppError::Conflict("idempotencyConflict"))`,
    ///   and nothing is written.
    pub async fn enqueue_in(
        tx: &mut Transaction<'_, Postgres>,
        organization_id: &str,
        kind: &'static EventKind,
        aggregate_id: &str,
        key: &str,
        payload: &Value,
    ) -> AppResult<String> {
        let payload_hash = hash_payload(payload);
        let payload_redacted = Redactor::redact(payload);

        let existing: Option<(String, String)> = sqlx::query_as(
            "SELECT id::text, payload_hash FROM outbox_events \
             WHERE organization_id = $1::uuid AND event_type = $2 AND idempotency_key = $3",
        )
        .bind(organization_id)
        .bind(kind.event_type)
        .bind(key)
        .fetch_optional(&mut **tx)
        .await?;

        if let Some((existing_id, existing_hash)) = existing {
            if existing_hash == payload_hash {
                return Ok(existing_id);
            } else {
                return Err(pos_error::AppError::Conflict(
                    "idempotencyConflict".to_owned(),
                ));
            }
        }

        let new_id = uuid::Uuid::new_v4().to_string();
        let inserted: Option<String> = sqlx::query_scalar(
            "INSERT INTO outbox_events (
                id, organization_id, event_type, destination,
                aggregate_type, aggregate_id, idempotency_key,
                payload_hash, payload_redacted, status,
                attempts, next_attempt_at, created_at
            ) VALUES (
                $1::uuid, $2::uuid, $3, $4,
                $5, $6, $7,
                $8, $9, 'PENDING',
                0, now(), now()
            )
            ON CONFLICT (organization_id, event_type, idempotency_key) DO NOTHING
            RETURNING id::text",
        )
        .bind(&new_id)
        .bind(organization_id)
        .bind(kind.event_type)
        .bind(kind.destination)
        .bind(kind.aggregate_type)
        .bind(aggregate_id)
        .bind(key)
        .bind(&payload_hash)
        .bind(&payload_redacted)
        .fetch_optional(&mut **tx)
        .await?;

        if let Some(id) = inserted {
            return Ok(id);
        }

        let existing: Option<(String, String)> = sqlx::query_as(
            "SELECT id::text, payload_hash FROM outbox_events \
             WHERE organization_id = $1::uuid AND event_type = $2 AND idempotency_key = $3",
        )
        .bind(organization_id)
        .bind(kind.event_type)
        .bind(key)
        .fetch_optional(&mut **tx)
        .await?;

        if let Some((existing_id, existing_hash)) = existing {
            if existing_hash == payload_hash {
                Ok(existing_id)
            } else {
                Err(pos_error::AppError::Conflict(
                    "idempotencyConflict".to_owned(),
                ))
            }
        } else {
            Err(pos_error::AppError::Internal(
                "failed to enqueue outbox event".to_owned(),
            ))
        }
    }
}
