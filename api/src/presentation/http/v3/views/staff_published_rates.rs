//! Staff published rates view (§4.8).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use sqlx::PgPool;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::published_rates::PublishedRatesData;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct StaffPublishedRatesBranchView {
    pool: PgPool,
}

impl StaffPublishedRatesBranchView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for StaffPublishedRatesBranchView {
    const NAME: &'static str = "staff_published_rates";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::StaffRpc, "adminSnapshot");
    const VERSIONED: bool = true;
    type Data = PublishedRatesData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        async move { super::published_rates::published_rates_data(&pool, &branch).await }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(StaffPublishedRatesBranchView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    super::published_rates::export_types(dir)
}

