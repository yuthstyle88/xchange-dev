//! RBAC F5 (design `docs/design/blueprint/user-management-rbac.md` §11.6): the `admin_permission_matrix` view.
//! `GET /api/v3/views/admin_permission_matrix`
//!
//! Organisation-wide; versioned by `role_grant_version`. Reads `RoleGrants::current()` after making sure it is at the
//! stored version (a stale process snapshot is reloaded through `PgRoleGrantStore::load` first).
//!
//! Skeleton (claude/65357afb, `feat/rbac-groups`): `own_version` and `load` are F5's.

use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::{Actor, RoleGrantStore};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::infrastructure::access::PgRoleGrantStore;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_permission_matrix::{
    AdminPermissionMatrixData, MatrixGroup, MatrixPermission,
};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct AdminPermissionMatrixView {
    pool: PgPool,
}

impl AdminPermissionMatrixView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for AdminPermissionMatrixView {
    const NAME: &'static str = "admin_permission_matrix";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminPermissionMatrix");
    const VERSIONED: bool = true;
    const ORG_WIDE: bool = true;
    type Filter = NoFilter;
    type Data = AdminPermissionMatrixData;

    async fn own_version(&self, _branch: &BranchId, _who: &Actor) -> AppResult<Option<i64>> {
        let mut conn = self.pool.acquire().await.map_err(AppError::Database)?;
        let version = PgRoleGrantStore::new()
            .version(&mut conn)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(Some(version))
    }

    async fn load(
        &self,
        _branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> AppResult<Self::Data> {
        todo!("F5")
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(AdminPermissionMatrixView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminPermissionMatrixData::export_all(&cfg)?;
    MatrixPermission::export_all(&cfg)?;
    MatrixGroup::export_all(&cfg)?;
    Ok(())
}
