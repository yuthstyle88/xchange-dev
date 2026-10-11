//! The `staff_floor` view (§3.1).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_auth::domain::access::v3_roles::actor_from_v3;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::application::staff_floor::StaffFloorReader;
use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::staff_floor::StaffFloorView;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct StaffFloorBranchView {
    reader: Arc<StaffFloorReader>,
}

impl StaffFloorBranchView {
    pub fn new(reader: Arc<StaffFloorReader>) -> Self {
        Self { reader }
    }
}

impl BranchView for StaffFloorBranchView {
    const NAME: &'static str = "staff_floor";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::StaffRpc, "staffFloor");
    const VERSIONED: bool = true;
    type Data = StaffFloorView;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let reader = self.reader.clone();
        let branch = branch.clone();
        let who = who.clone();

        async move {
            let user = actor_from_v3(
                &who,
                "00000000-0000-0000-0000-000000000001",
                DateTime::<Utc>::UNIX_EPOCH,
            );
            reader.floor(&user, &branch).await
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let reader = Arc::new(StaffFloorReader::new(state.db.clone()));
    ViewEndpoint::register(cfg, Arc::new(StaffFloorBranchView::new(reader)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::staff_floor::export_types(dir)
}
