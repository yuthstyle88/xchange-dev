//! Business day close workflow (ticket CS-22b #6214, design `configurable-shifts.md` §2.12).
//! Runs every minute to close waiting work across all branches at the business-day cutover.

use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use sqlx::PgPool;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_shift::application::shift_plan::service::ShiftPlanService;
use xc_shift::infrastructure::shift_plan::pg_store::PgShiftTemplateStore;

use crate::presentation::http::v3::cash::queue_day_hook;

pub struct BusinessDayCloser {
    pool: PgPool,
}

impl BusinessDayCloser {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn sweep_at(&self, now: DateTime<Utc>) -> AppResult<usize> {
        let branch_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM branches ORDER BY id")
            .fetch_all(&self.pool)
            .await?;

        let mut total_ended = 0;
        let hook = queue_day_hook(&self.pool);

        for branch_str in branch_ids {
            let branch = BranchId::new(&branch_str);
            let mut tx = match self.pool.begin().await {
                Ok(tx) => tx,
                Err(e) => {
                    tracing::error!("business-day-close: begin failed for {branch_str}: {e}");
                    continue;
                }
            };

            let locked: bool = match sqlx::query_scalar(
                "SELECT pg_try_advisory_xact_lock(hashtext('business-day-close:' || $1))",
            )
            .bind(branch.as_str())
            .fetch_one(&mut *tx)
            .await
            {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("business-day-close: lock query failed for {branch_str}: {e}");
                    continue;
                }
            };

            if !locked {
                continue;
            }

            let branch_tx = BranchTx::new(Transactor::new(self.pool.clone()));
            let plan_svc = ShiftPlanService::new(Arc::new(PgShiftTemplateStore), branch_tx);
            let plan = match plan_svc.plan_in(&mut tx, &branch).await {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("business-day-close: plan load failed for {branch_str}: {e}");
                    continue;
                }
            };

            let Some((cutover, _)) = plan.span_for(now) else {
                continue;
            };

            let held_before = cutover - TimeDelta::hours(24);

            let ended = match hook
                .on_business_day_end(&mut tx, &branch, cutover, held_before, now)
                .await
            {
                Ok(n) => n,
                Err(e) => {
                    tracing::error!("business-day-close: sweep failed for {branch_str}: {e}");
                    continue;
                }
            };

            if let Err(e) = tx.commit().await {
                tracing::error!("business-day-close: commit failed for {branch_str}: {e}");
                continue;
            }

            total_ended += ended;
        }

        Ok(total_ended)
    }
}
