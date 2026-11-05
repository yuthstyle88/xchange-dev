//! CS-20 (design `docs/design/configurable-shifts.md` §2.10, T9): the instant a business day ends, which every
//! "until the end of the sale's day" deadline means: a reversal's window, `reversible_until` on the completion and the
//! loyalty facts.

use std::sync::Arc;

use chrono::{DateTime, Days, NaiveDate, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use sqlx::{PgPool, Postgres, Transaction};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_shift::application::shift_plan::service::ShiftPlanService;
use xc_shift::infrastructure::shift_plan::pg_store::PgShiftTemplateStore;

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
        let plan = ShiftPlanService::new(
            Arc::new(PgShiftTemplateStore),
            BranchTx::new(Transactor::new(self.pool.clone())),
        )
        .plan_in(tx, branch)
        .await
        .map_err(|e| match e {
            AppError::Domain(_) => {
                AppError::Internal(format!("branch {} does not exist", branch.as_str()))
            }
            other => other,
        })?;
        if let Some(end) = plan.business_day_end(business_date) {
            return Ok(end);
        }
        let tz = plan.timezone();
        business_date
            .checked_add_days(Days::new(1))
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .and_then(|d| tz.from_local_datetime(&d).earliest())
            .map(|d| d.with_timezone(&Utc))
            .ok_or_else(|| AppError::Internal(format!("the end of {business_date}")))
    }
}
