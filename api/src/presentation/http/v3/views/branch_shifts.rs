//! Ticket CS-9 (#6188): `branch_shifts` view (configurable-shifts.md §2.9).
//!
//! Answers `{templates, cutover, defaultShiftHours}` for the named branch.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use sqlx::PgPool;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_shift::application::shift_plan::service::ShiftPlanService;
use xc_shift::infrastructure::shift_plan::pg_store::PgShiftTemplateStore;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::branch_shifts::BranchShiftsData;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct BranchShiftsBranchView {
    pool: PgPool,
}

impl BranchShiftsBranchView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for BranchShiftsBranchView {
    const NAME: &'static str = "branch_shifts";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminBranchesProfile");
    const VERSIONED: bool = true;
    type Data = BranchShiftsData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        async move {
            let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
            let mut tx = pool.begin().await?;
            let svc = ShiftPlanService::new(Arc::new(PgShiftTemplateStore), branch_tx);
            let plan = svc.plan_in(&mut tx, &branch).await?;
            tx.commit().await?;
            Ok(BranchShiftsData::from_plan(&plan))
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(BranchShiftsBranchView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::branch_shifts::export_types(dir)
}
