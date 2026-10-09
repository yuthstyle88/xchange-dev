//! PostgreSQL implementation of `AuditLog`.

use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::domain::audit_log::{AuditEntry, AuditEvent, AuditLog};
use crate::domain::redactor::Redactor;

pub struct PgAuditLog {
    pool: PgPool,
}

impl PgAuditLog {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl AuditLog for PgAuditLog {
    fn append<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        event: &'a AuditEvent,
    ) -> BoxFuture<'a, AppResult<i64>> {
        Box::pin(async move {
            let before_redacted = event.before.as_ref().map(Redactor::redact);
            let after_redacted = event.after.as_ref().map(Redactor::redact);

            let id: i64 = sqlx::query_scalar(
                "INSERT INTO audit_events ( \
                     actor_id, actor_role, scope, branch_id, action, \
                     object_ref, before_redacted, after_redacted, at \
                 ) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now()) \
                 RETURNING id",
            )
            .bind(&event.actor_id)
            .bind(&event.actor_role)
            .bind(&event.scope)
            .bind(&event.branch_id)
            .bind(&event.action)
            .bind(&event.object_ref)
            .bind(before_redacted)
            .bind(after_redacted)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| AppError::Internal(format!("append audit event failed: {e}")))?;

            Ok(id)
        })
    }

    fn page<'a>(
        &'a self,
        after_id: Option<i64>,
        limit: i64,
    ) -> BoxFuture<'a, AppResult<Vec<AuditEntry>>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, actor_id, actor_role, scope, branch_id, action, \
                        object_ref, before_redacted, after_redacted, at \
                 FROM audit_events \
                 WHERE ($1::bigint IS NULL OR id > $1) \
                 ORDER BY id ASC \
                 LIMIT $2",
            )
            .bind(after_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::Internal(format!("page audit events failed: {e}")))?;

            let entries = rows
                .into_iter()
                .map(|r| AuditEntry {
                    id: r.get("id"),
                    at: r.get("at"),
                    event: AuditEvent {
                        actor_id: r.get("actor_id"),
                        actor_role: r.get("actor_role"),
                        scope: r.get("scope"),
                        action: r.get("action"),
                        object_ref: r.get("object_ref"),
                        branch_id: r.get("branch_id"),
                        before: r.get("before_redacted"),
                        after: r.get("after_redacted"),
                    },
                })
                .collect();

            Ok(entries)
        })
    }
}
