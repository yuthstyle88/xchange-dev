//! CS-20 (design `docs/design/configurable-shifts.md` §2.10, T9): the instant a business day ends, which every
//! "until the end of the sale's day" deadline means: a reversal's window, `reversible_until` on the completion and the
//! loyalty facts.

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::AppResult;
use sqlx::{PgPool, Postgres, Transaction};
use xc_kernel::ids::BranchId;

pub struct BusinessDayEnd {
    pool: PgPool,
}

impl BusinessDayEnd {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// `[required]` The end of `business_date` at `branch`: `ShiftPlan::business_day_end(business_date)` under the
    /// branch's live plan (`ShiftPlanService::plan_in`, as `backdate.rs` loads it). A branch with no live template falls
    /// back to local midnight after `business_date` in the branch timezone, today's rule (the same fallback as CS-20b).
    /// `AppError::Internal` for an unknown branch or an unparsable timezone.
    pub async fn of_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        business_date: NaiveDate,
    ) -> AppResult<DateTime<Utc>> {
        let _ = (&self.pool, tx, branch, business_date);
        todo!("CS-20")
    }
}
