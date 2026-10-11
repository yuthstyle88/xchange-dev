//! `PgReversalRepository`: `reversals` on PostgreSQL (design §3.1, §6).

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Row};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Money;

use crate::domain::reversal::repository::ReversalRepository;
use crate::domain::reversal::reversal::{
    NewReversal, PostedReversalRow, Reversal, ReversalId, ReversalStatus,
};
use crate::domain::transaction::transaction::TransactionId;

/// The partial unique index of V-5 (migration `20261112110000_reversals.sql`).
const ONE_LIVE: &str = "one_live_reversal_per_transaction";

#[derive(Clone)]
pub struct PgReversalRepository {
    #[allow(dead_code)]
    pool: PgPool,
}

impl PgReversalRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn stale(m: impl std::fmt::Display) -> AppError {
    AppError::Domain(format!("staleVersion: {m}"))
}

impl ReversalRepository for PgReversalRepository {
    async fn insert_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        r: &'a NewReversal,
    ) -> AppResult<()> {
        if let Some(live) = self.live_of_in(tx, r.original_transaction_id).await? {
            return Err(stale(format!(
                "exchange {} already has the live reversal {}",
                r.original_transaction_id.0, live.0
            )));
        }
        let inserted = sqlx::query(
            "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, \
                reason, amount_thb, business_date, requested_by, requested_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, 'REQUESTED', $5, $6::numeric, $7, $8, $9, $9, $9)",
        )
        .bind(r.id.0)
        .bind(r.organization_id)
        .bind(r.branch_id.as_str())
        .bind(r.original_transaction_id.0)
        .bind(&r.reason)
        .bind(r.amount_thb.to_string())
        .bind(r.business_date)
        .bind(&r.requested_by)
        .bind(r.requested_at)
        .execute(&mut **tx)
        .await;
        match inserted {
            Ok(_) => Ok(()),
            // a concurrent request won the race after the check above; the transaction is aborted
            Err(sqlx::Error::Database(e)) if e.constraint() == Some(ONE_LIVE) => {
                Err(stale(format!(
                    "exchange {} already has a live reversal",
                    r.original_transaction_id.0
                )))
            }
            Err(e) => Err(AppError::Database(e)),
        }
    }

    async fn set_approval_request_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: ReversalId,
        request: Uuid,
    ) -> AppResult<()> {
        let done = sqlx::query(
            "UPDATE reversals SET approval_request_id = $2, updated_at = now() \
             WHERE id = $1 AND status = 'REQUESTED' AND approval_request_id IS NULL",
        )
        .bind(id.0)
        .bind(request)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        if done.rows_affected() != 1 {
            return Err(stale(format!(
                "reversal {} is not awaiting its request",
                id.0
            )));
        }
        Ok(())
    }

    async fn find_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: ReversalId,
    ) -> AppResult<Option<Reversal>> {
        let Some(row) = sqlx::query(
            "SELECT id, organization_id, branch_id, original_transaction_id, status, reason, \
                amount_thb::text AS amount_thb, business_date, approval_request_id, requested_by, \
                requested_at \
             FROM reversals WHERE id = $1",
        )
        .bind(id.0)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?
        else {
            return Ok(None);
        };
        let get = |e: sqlx::Error| AppError::Database(e);
        let status: String = row.try_get("status").map_err(get)?;
        let amount: String = row.try_get("amount_thb").map_err(get)?;
        let request: Option<Uuid> = row.try_get("approval_request_id").map_err(get)?;
        Ok(Some(Reversal {
            id: ReversalId(row.try_get("id").map_err(get)?),
            organization_id: row.try_get("organization_id").map_err(get)?,
            branch_id: BranchId::new(row.try_get::<String, _>("branch_id").map_err(get)?),
            original_transaction_id: TransactionId(
                row.try_get("original_transaction_id").map_err(get)?,
            ),
            status: ReversalStatus::parse(&status)
                .ok_or_else(|| AppError::Internal(format!("reversal status {status}")))?,
            reason: row.try_get("reason").map_err(get)?,
            amount_thb: Money::parse(&amount)
                .map_err(|e| AppError::Internal(format!("reversal amount_thb: {e}")))?,
            business_date: row.try_get::<NaiveDate, _>("business_date").map_err(get)?,
            approval_request_id: request.ok_or_else(|| {
                AppError::Internal(format!("reversal {} has no approval request yet", id.0))
            })?,
            requested_by: row.try_get("requested_by").map_err(get)?,
            requested_at: row
                .try_get::<DateTime<Utc>, _>("requested_at")
                .map_err(get)?,
        }))
    }

    async fn live_of_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        original: TransactionId,
    ) -> AppResult<Option<ReversalId>> {
        let id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM reversals WHERE original_transaction_id = $1 AND status <> 'REJECTED'",
        )
        .bind(original.0)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        Ok(id.map(ReversalId))
    }

    async fn set_status_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: ReversalId,
        from: ReversalStatus,
        to: ReversalStatus,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let done = sqlx::query(
            "UPDATE reversals SET status = $3, updated_at = $4 WHERE id = $1 AND status = $2",
        )
        .bind(id.0)
        .bind(from.as_str())
        .bind(to.as_str())
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        if done.rows_affected() != 1 {
            return Err(stale(format!("reversal {} is not {}", id.0, from.as_str())));
        }
        Ok(())
    }

    async fn post_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: ReversalId,
        p: &'a PostedReversalRow,
    ) -> AppResult<()> {
        let done = sqlx::query(
            "UPDATE reversals SET status = 'POSTED', posting_id = $2, receipt_id = $3, shift_id = $4, \
                posted_by = $5, posted_at = $6, realized_margin_reversed_thb = $7::numeric, \
                cost_difference_thb = $8::numeric, updated_at = $6 \
             WHERE id = $1 AND status = 'APPROVED'",
        )
        .bind(id.0)
        .bind(p.posting_id)
        .bind(p.receipt_id)
        .bind(&p.shift_id)
        .bind(&p.posted_by)
        .bind(p.posted_at)
        .bind(p.realized_margin_reversed_thb.map(|d| d.to_string()))
        .bind(p.cost_difference_thb.map(|d| d.to_string()))
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        if done.rows_affected() != 1 {
            return Err(stale(format!("reversal {} is not APPROVED", id.0)));
        }
        Ok(())
    }
}
