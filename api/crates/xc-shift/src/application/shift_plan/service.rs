//! Shift plan application service (configurable-shifts.md §2.3, §2.9, ticket CS-3).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::domain::shift::value_objects::shift_error::ShiftError;
use crate::domain::shift_plan::repository::ShiftTemplateStore;
use crate::domain::shift_plan::shift_instance::ShiftInstance;
use crate::domain::shift_plan::shift_plan::ShiftPlan;
use crate::domain::shift_plan::shift_template::ShiftTemplateInput;

/// What a teller may open now: the covering instances, and the one to preselect when exactly one covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShiftChoices {
    pub choices: Vec<ShiftInstance>,
    pub preselect: Option<String>,
}

#[derive(Clone)]
pub struct ShiftPlanService {
    store: Arc<dyn ShiftTemplateStore>,
    branch_tx: BranchTx,
}

impl ShiftPlanService {
    pub fn new(store: Arc<dyn ShiftTemplateStore>, branch_tx: BranchTx) -> Self {
        Self { store, branch_tx }
    }

    pub fn store(&self) -> &Arc<dyn ShiftTemplateStore> {
        &self.store
    }

    /// The branch's live plan in `branches.timezone`.
    pub async fn plan_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
    ) -> AppResult<ShiftPlan> {
        let tz_str: Option<String> =
            sqlx::query_scalar("SELECT timezone FROM branches WHERE id = $1")
                .bind(branch.as_str())
                .fetch_optional(&mut **tx)
                .await?;
        let tz_str =
            tz_str.ok_or_else(|| AppError::Domain(ShiftError::BranchNotFound.to_string()))?;
        let tz: Tz = tz_str
            .parse()
            .map_err(|e| AppError::Internal(format!("invalid timezone {tz_str}: {e}")))?;
        let templates = self.store.live_in(tx, branch).await?;
        Ok(ShiftPlan::new(templates, tz))
    }

    /// Validates (`ShiftPlan::validate`) and replaces the list. A refusal is
    /// `AppError::Validation(format!("shiftPlanInvalid:{reason}:{detail}"))`, with `detail` empty when there is none;
    /// nothing is written.
    pub async fn save_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        org: &str,
        inputs: Vec<ShiftTemplateInput>,
        now: DateTime<Utc>,
    ) -> AppResult<ShiftPlan> {
        let validated = ShiftPlan::validate(inputs).map_err(|r| {
            AppError::Validation(format!("shiftPlanInvalid:{}:{}", r.reason(), r.detail()))
        })?;
        self.store
            .replace_in(tx, branch, org, &validated, now)
            .await?;
        self.plan_in(tx, branch).await
    }

    /// The instances covering `now`; `preselect` is the template id when exactly one covers.
    pub async fn choices_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<ShiftChoices> {
        let choices = self.plan_in(tx, branch).await?.covering(now);
        let preselect = match choices.as_slice() {
            [only] => Some(only.template_id.clone()),
            _ => None,
        };
        Ok(ShiftChoices { choices, preselect })
    }

    /// Seeds the defaults for a new branch in its own branch transaction; idempotent.
    ///
    /// `BranchTx::run` holds the branch lock, so two concurrent seeds run one after the other and the second answers
    /// `false` instead of hitting `shift_templates_live_name`; it also raises the branch version.
    pub async fn seed_defaults(&self, branch: &BranchId, org: &str) -> AppResult<bool> {
        let store = self.store.clone();
        let branch_id = branch.clone();
        let org = org.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { store.seed_defaults_in(tx, &branch_id, &org).await })
            })
            .await
    }
}
