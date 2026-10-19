//! `StatusService` (design §5, Q8, Q9, Q13, owner Q-A5/Q-A6): keeps `branch_service_status`.
//! `refresh_in` runs in every queue transaction (§4.3); `open` / `close` are BRANCH_MANAGER's
//! (`BranchDayOperate`, §3).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};
use xc_auth::domain::access::{Actor, Authorizer, Permission, Target};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::domain::repository::QueueRepository;
use crate::domain::status::branch_service_status::BranchServiceStatus;
use crate::domain::status::wait_estimator::{WaitEstimate, WaitEstimator};

/// The service times (`served_at → completed_at`, in seconds) `WaitEstimator` samples: today's at
/// `service_date`, oldest first, and the last 30 of earlier days.
async fn samples_in(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
    service_date: chrono::NaiveDate,
) -> AppResult<(Vec<i64>, Vec<i64>)> {
    let today_rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT EXTRACT(EPOCH FROM (completed_at - served_at))::bigint \
             FROM queue_tickets \
             WHERE branch_id = $1 AND status = 'COMPLETED' AND service_date = $2 \
             ORDER BY completed_at ASC",
    )
    .bind(branch.as_str())
    .bind(service_date)
    .fetch_all(&mut **tx)
    .await?;
    let today_samples: Vec<i64> = today_rows.into_iter().map(|(s,)| s).collect();

    let earlier_rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT EXTRACT(EPOCH FROM (completed_at - served_at))::bigint \
             FROM queue_tickets \
             WHERE branch_id = $1 AND status = 'COMPLETED' AND service_date < $2 \
             ORDER BY completed_at DESC \
             LIMIT 30",
    )
    .bind(branch.as_str())
    .bind(service_date)
    .fetch_all(&mut **tx)
    .await?;
    let earlier_samples: Vec<i64> = earlier_rows.into_iter().map(|(s,)| s).collect();
    Ok((today_samples, earlier_samples))
}

/// Ticket fields needed to count how many WAITING tickets are ahead of it in Q6 call order and
/// estimate its wait (QU-7 #1266, C03 #3404).
pub struct TicketRef<'a> {
    pub id: uuid::Uuid,
    pub branch: &'a BranchId,
    pub service_type: &'a str,
    pub sequence: i32,
    pub status: &'a str,
}

#[derive(Clone)]
pub struct StatusService {
    branch_tx: BranchTx,
    repo: Arc<dyn QueueRepository>,
}

impl StatusService {
    pub fn new(branch_tx: BranchTx, repo: Arc<dyn QueueRepository>) -> Self {
        Self { branch_tx, repo }
    }

    /// Counts WAITING tickets ahead of `ticket` in Q6 call order (every BOOKING before a WALK_IN,
    /// then by sequence) and estimates its wait inside `tx`. A ticket no longer WAITING has `(0, None)`.
    pub async fn ticket_wait_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        ticket: TicketRef<'_>,
        now: DateTime<Utc>,
    ) -> AppResult<(i32, Option<WaitEstimate>)> {
        if ticket.status != "WAITING" {
            return Ok((0, None));
        }
        let ahead: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM queue_tickets \
              WHERE branch_id = $1 AND status = 'WAITING' AND id <> $2 \
                AND ((service_type = $3 AND sequence < $4) \
                     OR (service_type = 'BOOKING' AND $3 = 'WALK_IN'))",
        )
        .bind(ticket.branch.as_str())
        .bind(ticket.id)
        .bind(ticket.service_type)
        .bind(ticket.sequence)
        .fetch_one(&mut **tx)
        .await?;
        let estimate = self.estimate_for_in(tx, ticket.branch, ahead, now).await?;
        Ok((ahead as i32, estimate))
    }

    /// The wait for a customer with `ahead` WAITING tickets before them, as `refresh_in` computes
    /// the branch's (`WaitEstimator`, Q-A5/Q-A6), inside the caller's transaction. `None` when there
    /// is nothing to estimate from. QU-7 #1266 (`my_queue_ticket`).
    pub async fn estimate_for_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        ahead: i64,
        now: DateTime<Utc>,
    ) -> AppResult<Option<WaitEstimate>> {
        let facts = self
            .repo
            .branch_in(tx, branch)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "branch",
                id: branch.as_str().to_owned(),
            })?;
        let tz: chrono_tz::Tz = facts
            .timezone
            .parse()
            .map_err(|e| AppError::Internal(format!("invalid timezone {}: {e}", facts.timezone)))?;
        let service_date = now.with_timezone(&tz).date_naive();
        let active_counters: (i64,) = sqlx::query_as(
            "SELECT count(*) FROM queue_counters WHERE branch_id = $1 AND status = 'ACTIVE'",
        )
        .bind(branch.as_str())
        .fetch_one(&mut **tx)
        .await?;
        let (today, earlier) = samples_in(tx, branch, service_date).await?;
        Ok(WaitEstimator::estimate(
            ahead,
            active_counters.0 as i32,
            &today,
            &earlier,
        ))
    }

    /// Recomputes the branch's row inside the caller's transaction: `active_counters`, the
    /// estimate and its range from `WaitEstimator` (the WAITING tickets are the tickets ahead),
    /// `estimate_as_of = now`, and a higher `queue_version`.
    pub async fn refresh_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<BranchServiceStatus> {
        let facts = self
            .repo
            .branch_in(tx, branch)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "branch",
                id: branch.as_str().to_owned(),
            })?;

        let tz: chrono_tz::Tz = facts
            .timezone
            .parse()
            .map_err(|e| AppError::Internal(format!("invalid timezone {}: {e}", facts.timezone)))?;
        let service_date = now.with_timezone(&tz).date_naive();

        let active_counters_row: (i64,) = sqlx::query_as(
            "SELECT count(*) FROM queue_counters WHERE branch_id = $1 AND status = 'ACTIVE'",
        )
        .bind(branch.as_str())
        .fetch_one(&mut **tx)
        .await?;
        let active_counters = active_counters_row.0 as i32;

        let ahead_row: (i64,) = sqlx::query_as(
            "SELECT count(*) FROM queue_tickets WHERE branch_id = $1 AND status = 'WAITING'",
        )
        .bind(branch.as_str())
        .fetch_one(&mut **tx)
        .await?;
        let ahead = ahead_row.0;

        let (today_samples, earlier_samples) = samples_in(tx, branch, service_date).await?;

        let estimate =
            WaitEstimator::estimate(ahead, active_counters, &today_samples, &earlier_samples);
        let (est_sec, est_low, est_high) = match estimate {
            Some(e) => (Some(e.seconds), Some(e.low_seconds), Some(e.high_seconds)),
            None => (None, None, None),
        };
        let estimate_as_of = Some(now);

        let update_res: Option<(String, bool, i64)> = sqlx::query_as(
            "UPDATE branch_service_status \
             SET active_counters = $2, \
                 estimate_seconds = $3, \
                 estimate_low_seconds = $4, \
                 estimate_high_seconds = $5, \
                 estimate_as_of = $6, \
                 queue_version = queue_version + 1 \
             WHERE branch_id = $1 \
             RETURNING organization_id::text, queue_open, queue_version",
        )
        .bind(branch.as_str())
        .bind(active_counters)
        .bind(est_sec)
        .bind(est_low)
        .bind(est_high)
        .bind(estimate_as_of)
        .fetch_optional(&mut **tx)
        .await?;

        let (organization_id, queue_open, queue_version) = match update_res {
            Some(row) => row,
            None => {
                let insert_row: (String, bool, i64) = sqlx::query_as(
                    "INSERT INTO branch_service_status ( \
                         organization_id, branch_id, queue_open, active_counters, \
                         queue_version, estimate_seconds, estimate_low_seconds, \
                         estimate_high_seconds, estimate_as_of \
                     ) VALUES ( \
                         $1::uuid, $2, false, $3, \
                         1, $4, $5, \
                         $6, $7 \
                     ) \
                     RETURNING organization_id::text, queue_open, queue_version",
                )
                .bind(&facts.organization_id)
                .bind(branch.as_str())
                .bind(active_counters)
                .bind(est_sec)
                .bind(est_low)
                .bind(est_high)
                .bind(estimate_as_of)
                .fetch_one(&mut **tx)
                .await?;
                insert_row
            }
        };

        Ok(BranchServiceStatus {
            branch_id: branch.clone(),
            organization_id,
            queue_open,
            active_counters,
            queue_version,
            estimate,
            estimate_as_of,
        })
    }

    /// Opens the branch's queue (`BranchDayOperate`); upserts the row of a branch that has none
    /// (the lane controller's ruling 5 on #879).
    pub async fn open(
        &self,
        actor: &Actor,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<BranchServiceStatus> {
        let status = self.clone();
        let b = branch.clone();
        let act = actor.clone();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { status.open_in(tx, &act, &b, now).await })
            })
            .await
    }

    /// `open` inside the caller's transaction, which must already hold the branch's lock (the v3
    /// command seam's `BranchTx`, QU-7 #1266).
    pub async fn open_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<BranchServiceStatus> {
        Authorizer::check(
            Some(actor),
            Permission::BranchDayOperate,
            &Target::new(&actor.organization_id, Some(branch.clone()), None),
            now,
        )
        .map_err(|_| AppError::Forbidden)?;

        let facts = self
            .repo
            .branch_in(tx, branch)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "branch",
                id: branch.as_str().to_owned(),
            })?;
        sqlx::query(
            "INSERT INTO branch_service_status (organization_id, branch_id, queue_open, queue_version) \
             VALUES ($1::uuid, $2, true, 0) \
             ON CONFLICT (branch_id) DO UPDATE SET queue_open = true",
        )
        .bind(&facts.organization_id)
        .bind(branch.as_str())
        .execute(&mut **tx)
        .await?;
        self.refresh_in(tx, branch, now).await
    }

    /// Closes the branch's queue (`BranchDayOperate`): no new ticket; existing ones are served.
    pub async fn close(
        &self,
        actor: &Actor,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<BranchServiceStatus> {
        let status = self.clone();
        let b = branch.clone();
        let act = actor.clone();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { status.close_in(tx, &act, &b, now).await })
            })
            .await
    }

    /// `close` inside the caller's transaction, which must already hold the branch's lock (the v3
    /// command seam's `BranchTx`, QU-7 #1266).
    pub async fn close_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<BranchServiceStatus> {
        Authorizer::check(
            Some(actor),
            Permission::BranchDayOperate,
            &Target::new(&actor.organization_id, Some(branch.clone()), None),
            now,
        )
        .map_err(|_| AppError::Forbidden)?;

        let facts = self
            .repo
            .branch_in(tx, branch)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "branch",
                id: branch.as_str().to_owned(),
            })?;
        sqlx::query(
            "INSERT INTO branch_service_status (organization_id, branch_id, queue_open, queue_version) \
             VALUES ($1::uuid, $2, false, 0) \
             ON CONFLICT (branch_id) DO UPDATE SET queue_open = false",
        )
        .bind(&facts.organization_id)
        .bind(branch.as_str())
        .execute(&mut **tx)
        .await?;
        self.refresh_in(tx, branch, now).await
    }
}
