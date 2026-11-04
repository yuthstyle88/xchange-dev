//! Shift plan application service (configurable-shifts.md §2.3, §2.9, ticket CS-3).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

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
        let _ = (tx, branch, &self.store);
        todo!("CS-3")
    }

    /// Validates (`ShiftPlan::validate`) and replaces the list. A refusal is
    /// `AppError::Validation(format!("shiftPlanInvalid:{reason}:{index}"))`, with `index` empty when there is none;
    /// nothing is written.
    pub async fn save_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        org: &str,
        inputs: Vec<ShiftTemplateInput>,
        now: DateTime<Utc>,
    ) -> AppResult<ShiftPlan> {
        let _ = (tx, branch, org, inputs, now);
        todo!("CS-3")
    }

    /// The instances covering `now`; `preselect` is the template id when exactly one covers.
    pub async fn choices_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<ShiftChoices> {
        let _ = (tx, branch, now);
        todo!("CS-3")
    }

    /// Seeds the defaults for a new branch in its own branch transaction; idempotent.
    pub async fn seed_defaults(&self, branch: &BranchId, org: &str) -> AppResult<bool> {
        let _ = (branch, org, &self.branch_tx);
        todo!("CS-3")
    }
}
