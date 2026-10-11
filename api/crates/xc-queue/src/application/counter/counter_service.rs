//! `CounterService` (design §5, Q7, owner Q-A4): a teller's counter follows their OPEN shift
//! (CA-3, one OPEN shift per teller), and pausing it stops that counter calling.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};
use xc_auth::domain::access::{Actor, Authorizer, Permission, Target};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::application::status::status_service::StatusService;
use crate::domain::counter::counter::{Counter, CounterStatus};
use crate::domain::repository::QueueRepository;

#[derive(Clone)]
pub struct CounterService {
    branch_tx: BranchTx,
    repo: Arc<dyn QueueRepository>,
    status: StatusService,
}

impl CounterService {
    pub fn new(branch_tx: BranchTx, repo: Arc<dyn QueueRepository>, status: StatusService) -> Self {
        Self {
            branch_tx,
            repo,
            status,
        }
    }

    /// The caller's own counter becomes PAUSED (`QueueOperate`); `active_counters` drops by one.
    /// Waiting tickets are untouched. No counter (no OPEN shift) → `QueueError::NoOpenShift`.
    pub async fn pause(
        &self,
        actor: &Actor,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<Counter> {
        let service = self.clone();
        let b = branch.clone();
        let act = actor.clone();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { service.pause_in(tx, &act, &b, now).await })
            })
            .await
    }

    /// `pause` inside the caller's transaction, which must already hold the branch's lock (the v3
    /// command seam's `BranchTx`, QU-7 #1266).
    pub async fn pause_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<Counter> {
        Authorizer::check(
            Some(actor),
            Permission::QueueOperate,
            &Target::new(&actor.organization_id, Some(branch.clone()), None),
            now,
        )
        .map_err(|_| AppError::Forbidden)?;

        let updated: Option<(String,)> = sqlx::query_as(
            "UPDATE queue_counters \
             SET status = 'PAUSED' \
             WHERE branch_id = $1 AND teller_id = $2::uuid \
             RETURNING organization_id::text",
        )
        .bind(branch.as_str())
        .bind(&actor.user_id)
        .fetch_optional(&mut **tx)
        .await?;

        let organization_id = match updated {
            Some((org_id,)) => org_id,
            None => return Err(AppError::Validation("noOpenShift".to_owned())),
        };

        self.status.refresh_in(tx, branch, now).await?;

        Ok(Counter {
            organization_id,
            branch_id: branch.clone(),
            teller_id: actor.user_id.clone(),
            status: CounterStatus::Paused,
        })
    }

    /// The caller's own counter becomes ACTIVE again; `active_counters` rises by one.
    pub async fn resume(
        &self,
        actor: &Actor,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<Counter> {
        let service = self.clone();
        let b = branch.clone();
        let act = actor.clone();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { service.resume_in(tx, &act, &b, now).await })
            })
            .await
    }

    /// `resume` inside the caller's transaction, which must already hold the branch's lock (the v3
    /// command seam's `BranchTx`, QU-7 #1266).
    pub async fn resume_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<Counter> {
        Authorizer::check(
            Some(actor),
            Permission::QueueOperate,
            &Target::new(&actor.organization_id, Some(branch.clone()), None),
            now,
        )
        .map_err(|_| AppError::Forbidden)?;

        let updated: Option<(String,)> = sqlx::query_as(
            "UPDATE queue_counters \
             SET status = 'ACTIVE' \
             WHERE branch_id = $1 AND teller_id = $2::uuid \
             RETURNING organization_id::text",
        )
        .bind(branch.as_str())
        .bind(&actor.user_id)
        .fetch_optional(&mut **tx)
        .await?;

        let organization_id = match updated {
            Some((org_id,)) => org_id,
            None => return Err(AppError::Validation("noOpenShift".to_owned())),
        };

        self.status.refresh_in(tx, branch, now).await?;

        Ok(Counter {
            organization_id,
            branch_id: branch.clone(),
            teller_id: actor.user_id.clone(),
            status: CounterStatus::Active,
        })
    }

    /// Called by the shift's open, in its transaction: inserts the teller's ACTIVE counter and
    /// refreshes the status.
    pub async fn on_shift_opened(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        teller_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Counter> {
        let facts = self
            .repo
            .branch_in(tx, branch)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "branch",
                id: branch.as_str().to_owned(),
            })?;

        sqlx::query(
            "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
             VALUES ($1::uuid, $2, $3::uuid, 'ACTIVE') \
             ON CONFLICT (branch_id, teller_id) DO UPDATE SET status = 'ACTIVE'",
        )
        .bind(&facts.organization_id)
        .bind(branch.as_str())
        .bind(teller_id)
        .execute(&mut **tx)
        .await?;

        self.status.refresh_in(tx, branch, now).await?;

        Ok(Counter {
            organization_id: facts.organization_id,
            branch_id: branch.clone(),
            teller_id: teller_id.to_owned(),
            status: CounterStatus::Active,
        })
    }

    /// Called by the shift's close, in its transaction: deletes the teller's counter and
    /// refreshes the status.
    pub async fn on_shift_closed(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        teller_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query("DELETE FROM queue_counters WHERE branch_id = $1 AND teller_id = $2::uuid")
            .bind(branch.as_str())
            .bind(teller_id)
            .execute(&mut **tx)
            .await?;

        self.status.refresh_in(tx, branch, now).await?;

        Ok(())
    }
}
