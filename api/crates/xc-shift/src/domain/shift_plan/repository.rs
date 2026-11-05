//! Storage port for shift templates (configurable-shifts.md §3.2).

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;

use super::shift_template::{ShiftTemplate, ShiftTemplateInput};

pub trait ShiftTemplateStore: Send + Sync + 'static {
    /// The branch's live templates, by position.
    fn live_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Vec<ShiftTemplate>>>;

    /// Update the rows with an id, insert the rows without, retire live rows not listed (`retired_at = now`).
    /// Positions follow list order. Returns the live templates after the change.
    fn replace_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        org: &'a str,
        inputs: &'a [ShiftTemplateInput],
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<Vec<ShiftTemplate>>>;

    /// Inserts `ShiftPlan::defaults()` when the branch has no live template; returns whether it inserted.
    fn seed_defaults_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        org: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>>;
}
