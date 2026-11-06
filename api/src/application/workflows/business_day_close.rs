//! Business day close workflow (ticket CS-22b #6214, design `configurable-shifts.md` §2.12).
//! Runs every minute to close waiting work across all branches at the business-day cutover.

use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use sqlx::{PgPool, Postgres, Transaction};
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

            // The branch lock comes before the ticket and kiosk rows, in `BranchTx::run`'s order, so a sweep and a
            // queue or kiosk command never wait on each other in a cycle (review of PR 6297).
            if let Err(e) = lock_branch_version(&mut tx, &branch).await {
                tracing::error!("business-day-close: branch lock failed for {branch_str}: {e}");
                continue;
            }

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

            // Views read by `branch_versions` (`kiosk_queue`, `my_queue_ticket`) must see what the sweep ended, as the
            // booking sweeper's do (#1937). Only when something ended: the sweep runs every minute, and raising the
            // version on an idle round would tell every client "changed" once a minute.
            if ended > 0
                && let Err(e) = raise_branch_version(&mut tx, &branch).await
            {
                tracing::error!("business-day-close: version raise failed for {branch_str}: {e}");
                continue;
            }

            if let Err(e) = tx.commit().await {
                tracing::error!("business-day-close: commit failed for {branch_str}: {e}");
                continue;
            }

            total_ended += ended;
        }

        Ok(total_ended)
    }
}

/// Locks the branch's `branch_versions` row `FOR UPDATE` until `tx` ends, creating it if absent, without raising it:
/// the first half of `BranchTx::run`, for a sweep that does not yet know whether it will write.
async fn lock_branch_version(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO branch_versions (branch_id, version) VALUES ($1, 0) ON CONFLICT (branch_id) DO NOTHING",
    )
    .bind(branch.as_str())
    .execute(&mut **tx)
    .await?;
    sqlx::query("SELECT version FROM branch_versions WHERE branch_id = $1 FOR UPDATE")
        .bind(branch.as_str())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Raises the branch's version by 1 inside `tx`, which already holds its row lock (`lock_branch_version`).
async fn raise_branch_version(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE branch_versions SET version = version + 1, changed_at = now() WHERE branch_id = $1",
    )
    .bind(branch.as_str())
    .execute(&mut **tx)
    .await?;
    Ok(())
}
