//! The `admin_branches` view (§7, §12.4).

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
use xc_rate::application::branch::service::BranchService;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_branches::{AdminBranchItem, AdminBranchesData};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct AdminBranchesBranchView {
    branches: BranchService,
}

impl AdminBranchesBranchView {
    fn new(branches: BranchService) -> Self {
        Self { branches }
    }
}

impl BranchView for AdminBranchesBranchView {
    const NAME: &'static str = "admin_branches";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    const VERSIONED: bool = false;
    type Data = AdminBranchesData;
    type Filter = NoFilter;

    fn load(
        &self,
        _branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let branches = self.branches.clone();
        let who = who.clone();
        async move {
            let all_branches = branches.list().await?;
            let filtered = match who.branch_id() {
                Some(own) => all_branches.into_iter().filter(|b| b.id == *own).collect(),
                None => all_branches,
            };
            let items = filtered
                .into_iter()
                .map(|b| AdminBranchItem {
                    id: b.id.to_string(),
                    name: b.name,
                })
                .collect();
            Ok(AdminBranchesData { branches: items })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));
    let branches = BranchService::new(branch_tx);
    ViewEndpoint::register(cfg, Arc::new(AdminBranchesBranchView::new(branches)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminBranchItem::export_all(&cfg)?;
    AdminBranchesData::export_all(&cfg)?;
    Ok(())
}
