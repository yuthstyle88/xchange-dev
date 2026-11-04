//! Backdated keying (`docs/design/backdated-keying.md` §5; guard G): a business date is closed once FINANCE has
//! certified that branch-day (`reconciliation_certifications`, append-only), the only per-date close record.

use chrono::NaiveDate;
use pos_error::{AppError, AppResult};
use xc_kernel::ids::BranchId;

pub struct ClosedDays;

impl ClosedDays {
    /// `[required]` `true` when `reconciliation_certifications` holds a row for (`branch`, `date`), read in the
    /// caller's transaction.
    pub async fn is_closed_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        date: NaiveDate,
    ) -> AppResult<bool> {
        let is_closed: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM reconciliation_certifications WHERE branch_id = $1 AND business_date = $2)",
        )
        .bind(branch.as_str())
        .bind(date)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        Ok(is_closed)
    }
}
