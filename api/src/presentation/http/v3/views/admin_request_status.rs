//! The `admin_request_status` view (§7, §12.2).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::application::compat::abandon_request::AbandonRequest;
use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_request_status::{
    AdminRequestStatusData, AdminRequestStatusFilter,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct AdminRequestStatusBranchView {
    abandon: AbandonRequest,
}

impl AdminRequestStatusBranchView {
    fn new(abandon: AbandonRequest) -> Self {
        Self { abandon }
    }
}

impl BranchView for AdminRequestStatusBranchView {
    const NAME: &'static str = "admin_request_status";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    const VERSIONED: bool = false;
    type Data = AdminRequestStatusData;
    type Filter = AdminRequestStatusFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let abandon = self.abandon.clone();
        let branch = branch.clone();
        let request_id = filter.request_id.clone();
        async move {
            let applied = abandon.check_applied(&branch, &request_id).await?;
            Ok(AdminRequestStatusData { applied })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));
    let abandon = AbandonRequest::new(branch_tx);
    ViewEndpoint::register(
        cfg,
        Arc::new(AdminRequestStatusBranchView::new(abandon)),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminRequestStatusFilter::export_all(&cfg)?;
    AdminRequestStatusData::export_all(&cfg)?;
    Ok(())
}
