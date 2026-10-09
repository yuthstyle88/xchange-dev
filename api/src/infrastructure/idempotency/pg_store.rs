//! PostgreSQL implementation of the idempotency store (§6, §5.1, §5.6).

use pos_error::AppResult;
use sqlx::PgPool;

use super::store::{BoxFuture, Claim, ClaimToken, IdempotencyKey, IdempotencyStore};

/// PostgreSQL-backed idempotency store over `idempotency_records`.
#[derive(Clone)]
pub struct PgIdempotencyStore {
    pool: PgPool,
    organization_id: String,
}

impl PgIdempotencyStore {
    pub fn new(pool: PgPool, organization_id: impl Into<String>) -> Self {
        Self {
            pool,
            organization_id: organization_id.into(),
        }
    }

    pub fn organization_id(&self) -> &str {
        &self.organization_id
    }
}

impl IdempotencyStore for PgIdempotencyStore {
    fn claim<'a>(
        &'a self,
        op: &'a str,
        key: &'a IdempotencyKey,
        hash: &'a str,
    ) -> BoxFuture<'a, AppResult<Claim>> {
        Box::pin(async move {
            let token = ClaimToken::new(uuid::Uuid::new_v4().to_string());

            // Attempt to insert a new pending record.
            let inserted: Option<i32> = sqlx::query_scalar(
                "INSERT INTO idempotency_records (organization_id, operation, key, request_hash, status, created_at, claim_token) \
                 VALUES ($1::uuid, $2, $3, $4, 'PENDING', now(), $5) \
                 ON CONFLICT (organization_id, operation, key) DO NOTHING \
                 RETURNING 1",
            )
            .bind(&self.organization_id)
            .bind(op)
            .bind(key.as_str())
            .bind(hash)
            .bind(token.as_str())
            .fetch_optional(&self.pool)
            .await?;

            if inserted.is_some() {
                return Ok(Claim::New(token));
            }

            // Record already exists. Query existing record.
            let row: Option<(String, Option<String>, String, bool)> = sqlx::query_as(
                "SELECT request_hash, response_ref::text, status, (created_at <= now() - interval '60 seconds') \
                 FROM idempotency_records \
                 WHERE organization_id = $1::uuid AND operation = $2 AND key = $3",
            )
            .bind(&self.organization_id)
            .bind(op)
            .bind(key.as_str())
            .fetch_optional(&self.pool)
            .await?;

            let (stored_hash, response_ref, status, is_expired) = match row {
                Some(r) => r,
                None => {
                    // Record was released/deleted concurrently. Try insert again.
                    let reinserted: Option<i32> = sqlx::query_scalar(
                        "INSERT INTO idempotency_records (organization_id, operation, key, request_hash, status, created_at, claim_token) \
                         VALUES ($1::uuid, $2, $3, $4, 'PENDING', now(), $5) \
                         ON CONFLICT (organization_id, operation, key) DO NOTHING \
                         RETURNING 1",
                    )
                    .bind(&self.organization_id)
                    .bind(op)
                    .bind(key.as_str())
                    .bind(hash)
                    .bind(token.as_str())
                    .fetch_optional(&self.pool)
                    .await?;
                    if reinserted.is_some() {
                        return Ok(Claim::New(token));
                    } else {
                        return Ok(Claim::Conflict);
                    }
                }
            };

            if stored_hash != hash {
                return Ok(Claim::Conflict);
            }

            if let ("FINISHED", Some(json_text)) = (status.as_str(), response_ref) {
                let val: serde_json::Value = serde_json::from_str(&json_text)?;
                return Ok(Claim::Replay(val));
            }

            if !is_expired {
                // In-flight pending record younger than 60s
                return Ok(Claim::Conflict);
            }

            // PENDING record older than 60s with the same hash: takeover
            let taken_over = sqlx::query(
                "UPDATE idempotency_records \
                 SET created_at = now(), \
                     claim_token = $1 \
                 WHERE organization_id = $2::uuid \
                   AND operation = $3 \
                   AND key = $4 \
                   AND status = 'PENDING' \
                   AND request_hash = $5 \
                   AND created_at <= now() - interval '60 seconds'",
            )
            .bind(token.as_str())
            .bind(&self.organization_id)
            .bind(op)
            .bind(key.as_str())
            .bind(hash)
            .execute(&self.pool)
            .await?;

            if taken_over.rows_affected() == 1 {
                Ok(Claim::New(token))
            } else {
                Ok(Claim::Conflict)
            }
        })
    }

    fn finish_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        op: &'a str,
        key: &'a IdempotencyKey,
        token: &'a ClaimToken,
        response: serde_json::Value,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let json_str = response.to_string();
            let res = sqlx::query(
                "UPDATE idempotency_records \
                 SET response_ref = $1::jsonb, status = 'FINISHED' \
                 WHERE organization_id = $2::uuid AND operation = $3 AND key = $4 AND claim_token = $5 AND status = 'PENDING'",
            )
            .bind(json_str)
            .bind(&self.organization_id)
            .bind(op)
            .bind(key.as_str())
            .bind(token.as_str())
            .execute(&mut **tx)
            .await?;

            if res.rows_affected() != 1 {
                return Err(pos_error::AppError::Conflict(
                    "idempotency record could not be finished (claim token mismatch or not pending)".to_string(),
                ));
            }

            Ok(())
        })
    }

    fn release<'a>(
        &'a self,
        op: &'a str,
        key: &'a IdempotencyKey,
        token: &'a ClaimToken,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            sqlx::query(
                "DELETE FROM idempotency_records \
                 WHERE organization_id = $1::uuid AND operation = $2 AND key = $3 AND claim_token = $4 AND status = 'PENDING'",
            )
            .bind(&self.organization_id)
            .bind(op)
            .bind(key.as_str())
            .bind(token.as_str())
            .execute(&self.pool)
            .await?;

            Ok(())
        })
    }
}
