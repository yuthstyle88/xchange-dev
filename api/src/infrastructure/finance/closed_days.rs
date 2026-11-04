//! Backdated keying (`docs/design/backdated-keying.md` §5; guard G): a business date is closed once FINANCE has
//! certified that branch-day (`reconciliation_certifications`, append-only), the only per-date close record.

use chrono::NaiveDate;
use pos_error::AppResult;
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
        let _ = (tx, branch, date);
        todo!("BK-5: backdated-keying.md §5")
    }
}
