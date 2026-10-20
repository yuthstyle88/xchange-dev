//! OR-5a (#945): the `admin_branches_profile` view, A04's profile of the named branch.
//!
//! The same object `branchSave` answers.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use pos_infra::transactions::Transactor;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_branches_profile::{
    AdminBranchContactChannel, AdminBranchProfileData,
};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct AdminBranchesProfileBranchView {
    pool: PgPool,
}

impl AdminBranchesProfileBranchView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for AdminBranchesProfileBranchView {
    const NAME: &'static str = "admin_branches_profile";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminBranchesProfile");
    const VERSIONED: bool = true;
    type Data = AdminBranchProfileData;
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
            let saved = BranchService::new(BranchTx::new(Transactor::new(pool)))
                .require(&branch)
                .await?;
            Ok(AdminBranchProfileData {
                branch: saved.id.as_str().to_owned(),
                name: saved.name,
                code: saved.code,
                address: saved.address,
                timezone: saved.timezone,
                opening_hours: saved.opening_hours,
                booking_enabled: saved.booking_enabled,
                contact_channels: saved
                    .contact_channels
                    .into_iter()
                    .map(|ch| AdminBranchContactChannel {
                        kind: ch.kind.as_str().to_owned(),
                        value: ch.value,
                    })
                    .collect(),
                status: saved.status.as_str().to_owned(),
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(AdminBranchesProfileBranchView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminBranchProfileData::export_all(&cfg)?;
    Ok(())
}
