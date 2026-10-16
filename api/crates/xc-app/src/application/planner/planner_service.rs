//! `PlannerService` (design §4.3, §5): save, delete, read and the sign-in claim of planner drafts.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::app_error::CustomerAppError;
use crate::domain::app_session::SessionHash;
use crate::domain::owner::DraftOwner;
use crate::domain::planner::planner_draft::{DraftSave, PlannerDraft};
use crate::domain::planner::repository::PlannerRepository;
use crate::domain::settings::AppSettingsRepository;
use crate::infrastructure::app_settings_repository::PgAppSettingsRepository;
use crate::infrastructure::planner_repository::PgPlannerRepository;

pub struct PlannerService {
    pool: PgPool,
    drafts: PgPlannerRepository,
    settings: PgAppSettingsRepository,
}

impl PlannerService {
    /// Wires `PgPlannerRepository` and `PgAppSettingsRepository` itself.
    pub fn new(pool: PgPool) -> Self {
        Self {
            drafts: PgPlannerRepository::new(pool.clone()),
            settings: PgAppSettingsRepository::new(pool.clone()),
            pool,
        }
    }

    /// The draft `id` if `owner` owns it: another owner's → `outOfScope`, none → `notFound`.
    fn owned(
        draft: Option<PlannerDraft>,
        owner: &DraftOwner,
    ) -> Result<PlannerDraft, CustomerAppError> {
        match draft {
            None => Err(CustomerAppError::NotFound),
            Some(d) if &d.owner != owner => Err(CustomerAppError::OutOfScope),
            Some(d) => Ok(d),
        }
    }

    /// Creates (`save.id` is `None`) or updates a draft of `owner` in `organization_id`.
    ///
    /// - A create reads the organisation's settings: `expires_at` = `now` + the lifetime; no row or
    ///   a NULL lifetime → `appSettingsMissing`. `created_at` = `updated_at` = `now`, version 1.
    /// - An update needs the draft's owner (another owner → `outOfScope`, no such draft →
    ///   `notFound`) and its current version (`expected_version` is not the stored version →
    ///   `staleVersion`). It writes the plan, version + 1 and `updated_at` = `now`; `expires_at`
    ///   is kept.
    /// - A refusal writes nothing.
    pub async fn save_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        owner: &DraftOwner,
        save: DraftSave,
        now: DateTime<Utc>,
    ) -> Result<PlannerDraft, CustomerAppError> {
        match save.id {
            None => {
                let settings = self
                    .settings
                    .get_in(tx, organization_id)
                    .await?
                    .ok_or(CustomerAppError::AppSettingsMissing)?;
                let draft = PlannerDraft {
                    id: Uuid::new_v4(),
                    organization_id,
                    owner: owner.clone(),
                    plan: save.plan,
                    version: 1,
                    expires_at: settings.draft_expires_at(now)?,
                    created_at: now,
                    updated_at: now,
                };
                self.drafts.insert_in(tx, &draft).await?;
                Ok(draft)
            }
            Some(id) => {
                let stored = Self::owned(self.drafts.get_in(tx, id).await?, owner)?;
                if stored.organization_id != organization_id {
                    return Err(CustomerAppError::OutOfScope);
                }
                let expected = save
                    .expected_version
                    .ok_or(CustomerAppError::ValidationFailed {
                        field: "expectedVersion",
                    })?;
                if expected != stored.version {
                    return Err(CustomerAppError::StaleVersion);
                }
                let draft = PlannerDraft {
                    plan: save.plan,
                    version: stored.version + 1,
                    updated_at: now,
                    ..stored
                };
                if !self.drafts.update_in(tx, &draft, expected).await? {
                    return Err(CustomerAppError::StaleVersion);
                }
                Ok(draft)
            }
        }
    }

    /// Deletes a draft of `owner`; another owner → `outOfScope`, no such draft → `notFound`.
    pub async fn delete_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &DraftOwner,
        id: Uuid,
    ) -> Result<(), CustomerAppError> {
        Self::owned(self.drafts.get_in(tx, id).await?, owner)?;
        self.drafts.delete_in(tx, id).await
    }

    /// One draft of `owner`; another owner's → `outOfScope`, none → `notFound`.
    pub async fn get(
        &self,
        owner: &DraftOwner,
        id: Uuid,
    ) -> Result<PlannerDraft, CustomerAppError> {
        let mut tx = self.pool.begin().await?;
        let found = self.drafts.get_in(&mut tx, id).await?;
        tx.rollback().await?;
        Self::owned(found, owner)
    }

    /// Every draft of `owner` (the view `my_planner_drafts`), newest first.
    pub async fn list(&self, owner: &DraftOwner) -> Result<Vec<PlannerDraft>, CustomerAppError> {
        self.drafts.list(owner).await
    }

    /// Sign-in (P6): every draft of `session` becomes `customer_id`'s in one statement. Answers how
    /// many moved; a second call answers 0.
    pub async fn claim_session_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        session: &SessionHash,
        customer_id: &str,
        now: DateTime<Utc>,
    ) -> Result<u64, CustomerAppError> {
        self.drafts.claim_in(tx, session, customer_id, now).await
    }
}
