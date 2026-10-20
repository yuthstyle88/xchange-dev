//! The `admin_operations` view (§7, §12.7).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::application::shift::service::ShiftService;

use crate::bootstrap::AppState;
use crate::infrastructure::custody::custody_service::pg_custody_service;
use crate::presentation::dto::v3::admin_operations::{
    AdminOperationsData, AdminOperationsEntity,
};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct AdminOperationsBranchView {
    custody: CustodyService,
}

impl AdminOperationsBranchView {
    fn new(custody: CustodyService) -> Self {
        Self { custody }
    }
}

impl BranchView for AdminOperationsBranchView {
    const NAME: &'static str = "admin_operations";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    const VERSIONED: bool = true;
    type Data = AdminOperationsData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let custody = self.custody.clone();
        let branch = branch.clone();
        async move {
            let state = custody.state(&branch).await?;
            let entity = state.entity.map(|e| AdminOperationsEntity {
                company: e.company,
                site: e.site,
            });
            Ok(AdminOperationsData { entity })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));
    let shifts = ShiftService::new(
        Arc::new(xc_cash::infrastructure::ledger::ledger_shift_repository::ledger_shift_repository(state.db.clone())),
        branch_tx.clone(),
    );
    let custody = pg_custody_service(&state.db, &shifts, &branch_tx);
    ViewEndpoint::register(cfg, Arc::new(AdminOperationsBranchView::new(custody)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminOperationsEntity::export_all(&cfg)?;
    AdminOperationsData::export_all(&cfg)?;
    Ok(())
}

