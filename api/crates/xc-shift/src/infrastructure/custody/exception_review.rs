//! In-place review of an open `cash_exceptions` row (§7 of #3413).

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;

/// Result of attempting to mark a branch's cash exception as `REVIEWED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewOutcome {
    Reviewed,
    AlreadyReviewed,
    NotFound,
}

/// Marks an `OPEN` cash exception on `branch` as `REVIEWED` in the caller's transaction.
pub async fn review_in(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
    exception_id: &str,
    reviewer: &str,
    review_note: &str,
    now: DateTime<Utc>,
) -> AppResult<ReviewOutcome> {
    let updated = sqlx::query(
        "UPDATE cash_exceptions \
            SET data = data || jsonb_build_object( \
                'status',     'REVIEWED', \
                'reviewedBy', $3::text, \
                'reviewedAt', $4::bigint, \
                'reviewNote', $5::text \
            ) \
          WHERE branch_id = $1 \
            AND id        = $2 \
            AND data->>'status' = 'OPEN'",
    )
    .bind(branch.as_str())
    .bind(exception_id)
    .bind(reviewer)
    .bind(now.timestamp_millis())
    .bind(review_note)
    .execute(&mut **tx)
    .await
    .map_err(AppError::Database)?;

    if updated.rows_affected() == 1 {
        return Ok(ReviewOutcome::Reviewed);
    }

    let exists: Option<i32> =
        sqlx::query_scalar("SELECT 1 FROM cash_exceptions WHERE branch_id = $1 AND id = $2")
            .bind(branch.as_str())
            .bind(exception_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;

    match exists {
        Some(_) => Ok(ReviewOutcome::AlreadyReviewed),
        None => Ok(ReviewOutcome::NotFound),
    }
}
