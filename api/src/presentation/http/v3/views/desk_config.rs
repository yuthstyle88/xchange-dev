//! The `desk_config` view (§4.1, §6).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::desk_config::DeskConfig;
use crate::presentation::http::v3::desk::config::DeskConfigCore;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct DeskConfigView {
    state: AppState,
}

impl DeskConfigView {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

impl BranchView for DeskConfigView {
    const NAME: &'static str = "desk_config";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminDeskConfig");
    const VERSIONED: bool = true;
    type Data = DeskConfig;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let state = self.state.clone();
        let branch = branch.clone();
        async move { DeskConfigCore::read(&state, &branch).await }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(DeskConfigView::new(state.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::desk_config::export_types(dir)
}
