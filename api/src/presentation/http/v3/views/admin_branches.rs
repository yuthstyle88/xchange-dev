//! The `admin_branches` view (§7, §12.4).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::Utc;
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::access::branch_scope::BranchScope;
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_branches::{AdminBranchItem, AdminBranchesData};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct AdminBranchesBranchView {
    pool: PgPool,
    branches: BranchService,
}

impl AdminBranchesBranchView {
    fn new(pool: PgPool, branches: BranchService) -> Self {
        Self { pool, branches }
    }
}

impl BranchView for AdminBranchesBranchView {
    const NAME: &'static str = "admin_branches";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "admin_branches");
    const VERSIONED: bool = false;
    type Data = AdminBranchesData;
    type Filter = NoFilter;

    fn load(
        &self,
        _branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branches = self.branches.clone();
        let who = who.clone();
        async move {
            let org_branch_ids: Vec<String> =
                sqlx::query_scalar("SELECT id FROM branches WHERE organization_id::text = $1")
                    .bind(&who.organization_id)
                    .fetch_all(&pool)
                    .await
                    .map_err(AppError::Database)?;
            let all_branches: Vec<_> = branches
                .list()
                .await?
                .into_iter()
                .filter(|b| org_branch_ids.iter().any(|id| id == b.id.as_str()))
                .collect();
            let permission = match Policy::need(Self::INHERITS.0, &Action::new(Self::INHERITS.1)) {
                Some(Need::Permission(p)) => p,
                _ => return Err(AppError::Internal("admin_branches need".into())),
            };
            let scope = BranchScope::of(&who, permission, Utc::now());
            let items = all_branches
                .into_iter()
                .filter(|b| scope.contains(&b.id))
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
    ViewEndpoint::register(
        cfg,
        Arc::new(AdminBranchesBranchView::new(state.db.clone(), branches)),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminBranchItem::export_all(&cfg)?;
    AdminBranchesData::export_all(&cfg)?;
    Ok(())
}
