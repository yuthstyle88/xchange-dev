//! The `admin_shift_state` view (§7, §12.2).

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
use xc_shift::application::shift::service::ShiftService;

use crate::application::compat::snapshot::uri_component;
use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_shift_state::AdminShiftStateData;
use crate::presentation::dto::v3::common::{NoFilter, OpenShiftDto};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct AdminShiftStateBranchView {
    shifts: ShiftService,
}

impl AdminShiftStateBranchView {
    fn new(shifts: ShiftService) -> Self {
        Self { shifts }
    }
}

impl BranchView for AdminShiftStateBranchView {
    const NAME: &'static str = "admin_shift_state";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Data = AdminShiftStateData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let shifts = self.shifts.clone();
        let branch = branch.clone();
        async move {
            let open_shifts: Vec<OpenShiftDto> = shifts
                .open_of_branch(&branch)
                .await?
                .into_iter()
                .map(OpenShiftDto::from)
                .collect();
            let shift_open = !open_shifts.is_empty();
            let desk_url = format!("/desk?branch={}", uri_component(branch.as_str()));

            Ok(AdminShiftStateData {
                shift_open,
                desk_url,
                open_shifts,
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));
    let shifts = ShiftService::new(
        Arc::new(
            xc_cash::infrastructure::ledger::ledger_shift_repository::ledger_shift_repository(
                state.db.clone(),
            ),
        ),
        branch_tx,
    );
    ViewEndpoint::register(cfg, Arc::new(AdminShiftStateBranchView::new(shifts)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminShiftStateData::export_all(&cfg)?;
    Ok(())
}
