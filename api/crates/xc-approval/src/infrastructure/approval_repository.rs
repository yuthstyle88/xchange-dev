//! PostgreSQL implementation of `ApprovalRepository`.

use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::domain::approval_repository::ApprovalRepository;
use crate::domain::request::{ApprovalRequest, Outcome, RequestId};

pub struct PgApprovalRepository {
    #[allow(dead_code)]
    pool: PgPool,
}

impl PgApprovalRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn map_request_row(row: &sqlx::postgres::PgRow) -> ApprovalRequest {
    ApprovalRequest {
        id: RequestId::new(row.get::<String, _>("id")),
        organization_id: row.get("organization_id"),
        type_name: row.get("type"),
        branch_id: row.get("branch_id"),
        object_id: row.get("object_id"),
        object_version_hash: row.get("object_version_hash"),
        policy_version: row.get("policy_version"),
        proposed_payload: row.get("proposed_payload"),
        requested_by: row.get("requested_by"),
        requested_at: row.get("requested_at"),
        status: row.get("status"),
        idempotency_key: row.get("idempotency_key"),
    }
}

impl ApprovalRepository for PgApprovalRepository {
    fn find_by_type_and_key<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        type_name: &'a str,
        idempotency_key: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<ApprovalRequest>>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id::text AS id, organization_id::text AS organization_id, type, branch_id, object_id, \
                        object_version_hash, policy_version, proposed_payload, \
                        requested_by, requested_at, status, idempotency_key \
                 FROM approval_requests \
                 WHERE type = $1 AND idempotency_key = $2",
            )
            .bind(type_name)
            .bind(idempotency_key)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| AppError::Internal(format!("find_by_type_and_key failed: {e}")))?;

            Ok(row.as_ref().map(map_request_row))
        })
    }

    fn has_pending_for_object<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        type_name: &'a str,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS( \
                     SELECT 1 FROM approval_requests \
                     WHERE type = $1 AND object_id = $2 AND status = 'PENDING' \
                 )",
            )
            .bind(type_name)
            .bind(object_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| AppError::Internal(format!("has_pending_for_object failed: {e}")))?;

            Ok(exists)
        })
    }

    fn insert_request<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: &'a str,
        type_name: &'a str,
        branch_id: Option<String>,
        object_id: &'a str,
        object_version_hash: &'a str,
        policy_version: &'a str,
        proposed_payload: Value,
        requested_by: &'a str,
        idempotency_key: &'a str,
    ) -> BoxFuture<'a, AppResult<ApprovalRequest>> {
        Box::pin(async move {
            let row = sqlx::query(
                "INSERT INTO approval_requests ( \
                     id, organization_id, type, branch_id, object_id, object_version_hash, \
                     policy_version, proposed_payload, requested_by, \
                     requested_at, status, idempotency_key \
                 ) \
                 VALUES ( \
                     gen_random_uuid(), $1::uuid, $2, $3, $4, $5, $6, $7, $8, now(), 'PENDING', $9 \
                 ) \
                 RETURNING id::text AS id, organization_id::text AS organization_id, type, branch_id, object_id, \
                           object_version_hash, policy_version, proposed_payload, \
                           requested_by, requested_at, status, idempotency_key",
            )
            .bind(organization_id)
            .bind(type_name)
            .bind(branch_id)
            .bind(object_id)
            .bind(object_version_hash)
            .bind(policy_version)
            .bind(proposed_payload)
            .bind(requested_by)
            .bind(idempotency_key)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| {
                if let sqlx::Error::Database(ref db_err) = e {
                    if db_err.code().as_deref() == Some("23505") {
                        return AppError::Conflict(format!("unique constraint violated: {e}"));
                    }
                }
                AppError::Internal(format!("insert_request failed: {e}"))
            })?;

            Ok(map_request_row(&row))
        })
    }

    fn find_by_id_for_update<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a RequestId,
    ) -> BoxFuture<'a, AppResult<Option<ApprovalRequest>>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id::text AS id, organization_id::text AS organization_id, type, branch_id, object_id, \
                        object_version_hash, policy_version, proposed_payload, \
                        requested_by, requested_at, status, idempotency_key \
                 FROM approval_requests \
                 WHERE id = $1::uuid \
                 FOR UPDATE",
            )
            .bind(id.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| AppError::Internal(format!("find_by_id_for_update failed: {e}")))?;

            Ok(row.as_ref().map(map_request_row))
        })
    }

    fn update_status<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a RequestId,
        status: &'static str,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE approval_requests \
                 SET status = $2 \
                 WHERE id = $1::uuid",
            )
            .bind(id.as_str())
            .bind(status)
            .execute(&mut **tx)
            .await
            .map_err(|e| AppError::Internal(format!("update_status failed: {e}")))?;

            Ok(())
        })
    }

    fn insert_decision<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        request_id: &'a RequestId,
        approver_id: &'a str,
        approver_role: &'a str,
        outcome: Outcome,
        reason: &'a str,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO approval_decisions ( \
                     id, request_id, approver_id, approver_role, outcome, reason, at \
                 ) \
                 VALUES ( \
                     gen_random_uuid(), $1::uuid, $2, $3, $4, $5, now() \
                 )",
            )
            .bind(request_id.as_str())
            .bind(approver_id)
            .bind(approver_role)
            .bind(outcome.as_str())
            .bind(reason)
            .execute(&mut **tx)
            .await
            .map_err(|e| AppError::Internal(format!("insert_decision failed: {e}")))?;

            Ok(())
        })
    }
}
