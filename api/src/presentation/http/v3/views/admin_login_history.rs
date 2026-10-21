//! RBAC F4b (#3153): the `admin_login_history` view, querying login history for an organisation.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::user::user_repository::UserRepository;
use xc_auth::infrastructure::user::PgUserRepository;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_login_history::AdminLoginHistoryRow;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

pub struct AdminLoginHistoryPagedView {
    pool: PgPool,
}

impl AdminLoginHistoryPagedView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl PagedView for AdminLoginHistoryPagedView {
    const NAME: &'static str = "admin_login_history";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminLoginHistory");
    type Row = AdminLoginHistoryRow;
    type Filter = NoFilter;

    fn page(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let pool = self.pool.clone();
        let branch = branch.as_str().to_owned();
        async move {
            let mut tx = pool.begin().await.map_err(AppError::Database)?;
            let session_org: Option<String> =
                sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = $1")
                    .bind(&branch)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(AppError::Database)?;

            let Some(session_org) = session_org else {
                return Ok(Vec::new());
            };

            let entries = PgUserRepository::new()
                .list_logins(&mut tx, &session_org, before.as_deref(), limit)
                .await?;

            tx.commit().await.map_err(AppError::Database)?;

            let at = |t: DateTime<Utc>| t.to_rfc3339_opts(SecondsFormat::Millis, true);
            Ok(entries
                .into_iter()
                .map(|e| AdminLoginHistoryRow {
                    id: e.id,
                    at: at(e.at),
                    user_id: e.user_id,
                    user_name: e.user_name,
                    ip: e.ip,
                    result: e.result,
                })
                .collect())
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(
        cfg,
        Arc::new(AdminLoginHistoryPagedView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminLoginHistoryRow::export_all(&cfg)?;
    Ok(())
}
