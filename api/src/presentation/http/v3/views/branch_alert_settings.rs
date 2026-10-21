//! EX-5 (#1075): the `branch_alert_settings` view.
//!
//! Answers a branch's alert settings (stale-data minutes and stock floors).
//! A branch with no row answers null for staleDataMinutes and version, and an empty floor list.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::executive::alert_settings_repository::PgAlertSettingsRepository;
use crate::presentation::dto::v3::branch_alert_settings::{BranchAlertSettingsData, StockFloorDto};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct BranchAlertSettingsView {
    pool: PgPool,
    repo: PgAlertSettingsRepository,
}

impl BranchAlertSettingsView {
    pub fn new(pool: PgPool) -> Self {
        let repo = PgAlertSettingsRepository::new(pool.clone());
        Self { pool, repo }
    }
}

impl BranchView for BranchAlertSettingsView {
    const NAME: &'static str = "branch_alert_settings";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "branchAlertSettings");
    const VERSIONED: bool = true;
    type Data = BranchAlertSettingsData;
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
            let org_id: Uuid =
                sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_one(&pool)
                    .await
                    .map_err(AppError::Database)?;

            self.repo.read(org_id, &branch).await
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(BranchAlertSettingsView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    BranchAlertSettingsData::export_all(&cfg)?;
    StockFloorDto::export_all(&cfg)?;
    Ok(())
}
