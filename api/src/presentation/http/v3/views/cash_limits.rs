//! LG-3b (#926): the `cash_limits` view, a branch's count-difference limit (ledger design §3.3).
//! A branch with no row answers `null` in every field but `branch`.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_cash::infrastructure::drawer_low_levels::DrawerLowLevels;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::cash_limits::{CashLimitsData, CashLimitsRow};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub async fn load_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
) -> AppResult<CashLimitsData> {
    let row: Option<CashLimitsRow> = sqlx::query_as(
        "SELECT l.count_difference_limit_thb::text, l.updated_by, NULLIF(btrim(u.name), ''), l.updated_at \
           FROM cash_limits l \
           JOIN branches b ON b.organization_id = l.organization_id AND b.id = l.branch_id \
           LEFT JOIN users u ON u.id::text = l.updated_by \
          WHERE b.id = $1",
    )
    .bind(branch.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(pos_error::AppError::Database)?;
    let levels = DrawerLowLevels::for_branch_in(tx, branch).await?;
    Ok(CashLimitsData::of(branch.as_str(), row, levels))
}

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
        let branch = branch.clone();
        async move {
            let mut tx = pool.begin().await.map_err(pos_error::AppError::Database)?;
            let data = load_in(&mut tx, &branch).await?;
            tx.commit().await.map_err(pos_error::AppError::Database)?;
            Ok(data)
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
