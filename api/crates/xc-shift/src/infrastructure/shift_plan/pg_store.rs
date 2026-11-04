//! Postgres `shift_templates` store (configurable-shifts.md §3.1, ticket CS-2).

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;

use crate::domain::shift_plan::repository::ShiftTemplateStore;
use crate::domain::shift_plan::shift_template::{ShiftTemplate, ShiftTemplateInput};

#[derive(Debug, Clone, Copy, Default)]
pub struct PgShiftTemplateStore;

impl ShiftTemplateStore for PgShiftTemplateStore {
    fn live_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Vec<ShiftTemplate>>> {
        let _ = (tx, branch);
        todo!("CS-2")
    }

    fn replace_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        org: &'a str,
        inputs: &'a [ShiftTemplateInput],
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<Vec<ShiftTemplate>>> {
        let _ = (tx, branch, org, inputs, now);
        todo!("CS-2")
    }

    fn seed_defaults_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        org: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        let _ = (tx, branch, org);
        todo!("CS-2")
    }
}
