//! The `reversal` view (ticket #685, RV-4).

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use sqlx::PgPool;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::reversal::{
    ReversalBy, ReversalFilter, ReversalViewData, load_reversal_summary,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct ReversalView {
    pool: PgPool,
}

impl BranchView for ReversalView {
    const NAME: &'static str = "reversal";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "transactionView");
    const VERSIONED: bool = false;
    type Data = ReversalViewData;
    type Filter = ReversalFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.as_str().to_string();
        let id = filter.id.clone();
        async move {
            let none = Ok(ReversalViewData { reversal: None });
            let Some(id) = id.and_then(|s| Uuid::from_str(&s).ok()) else {
                return none;
            };
            let reversal = load_reversal_summary(&pool, ReversalBy::Id(id), &branch).await?;
            Ok(ReversalViewData { reversal })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(ReversalView { pool: state.db.clone() }));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::reversal::export_types(dir)
}
