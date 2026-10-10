//! LG-3b (#926): the `cash_limits` view, a branch's count-difference limit (ledger design §3.3).
//! A branch with no row answers `null` in every field but `branch`.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::cash_limits::CashLimitsData;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct CashLimitsBranchView {
    pool: PgPool,
}

impl CashLimitsBranchView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for CashLimitsBranchView {
    const NAME: &'static str = "cash_limits";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "cashLimitsRead");
    const VERSIONED: bool = true;
    type Data = CashLimitsData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.as_str().to_owned();
        async move {
            let row: Option<(Option<String>, String, DateTime<Utc>)> = sqlx::query_as(
                "SELECT l.count_difference_limit_thb::text, l.updated_by, l.updated_at \
                   FROM cash_limits l \
                   JOIN branches b ON b.organization_id = l.organization_id AND b.id = l.branch_id \
                  WHERE b.id = $1",
            )
            .bind(&branch)
            .fetch_optional(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;
            Ok(CashLimitsData::of(&branch, row))
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(CashLimitsBranchView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CashLimitsData::export_all(&cfg)?;
    Ok(())
}
