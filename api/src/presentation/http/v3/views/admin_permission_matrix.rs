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
use chrono::Utc;
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::permission_module::{Module, PermissionMatrix, PermissionModule};
use xc_auth::domain::access::{Actor, Group, Permission, Role, RoleGrantStore, RoleGrants};
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

fn permission_name(p: Permission) -> String {
    match serde_json::to_value(p) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

fn build_group(group: &Group, caller_is_executive: bool) -> MatrixGroup {
    let editable =
        caller_is_executive && !(group.built_in && group.code == Role::Executive.as_str());
    let permissions = RoleGrants::of(&group.code)
        .into_iter()
        .map(permission_name)
        .collect();
    let levels = PermissionMatrix::of_group(&group.code)
        .into_iter()
        .map(|(m, l)| (m.as_str().to_owned(), l.as_str().to_owned()))
        .collect();
    MatrixGroup {
        code: group.code.clone(),
        name: group.name.clone(),
        base: group.base.as_str().to_owned(),
        built_in: group.built_in,
        editable,
        permissions,
        levels,
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
        who: &Actor,
        _filter: &Self::Filter,
    ) -> AppResult<Self::Data> {
        let mut conn = self.pool.acquire().await.map_err(AppError::Database)?;
        let store = PgRoleGrantStore::new();
        let stored_version = store
            .version(&mut conn)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if stored_version != RoleGrants::current().version() {
            let snapshot = store
                .load(&mut conn)
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?;
            RoleGrants::install(snapshot);
        }
        drop(conn);

        let snapshot = RoleGrants::current();
        let now = Utc::now();
        let caller_is_executive = who.roles.iter().any(|r| {
            r.role == Role::Executive
                && r.branch.is_none()
                && r.valid_from <= now
                && r.valid_to.is_none_or(|to| now < to)
        });

        let modules = Module::ALL
            .into_iter()
            .map(|m| m.as_str().to_owned())
            .collect();

        let permissions = Permission::ALL
            .into_iter()
            .map(|p| {
                let at = PermissionModule::module(p);
                MatrixPermission {
                    name: permission_name(p),
                    module: at.map(|(m, _)| m.as_str().to_owned()),
                    level: at.map(|(_, l)| l.as_str().to_owned()),
                    editable: at.is_some(),
                }
            })
            .collect();

        let mut groups = Vec::new();
        for role in RoleGrants::BUILT_IN {
            if role == Role::Customer {
                continue;
            }
            if let Some(group) = snapshot.group(role.as_str()) {
                groups.push(build_group(group, caller_is_executive));
            }
        }
        for (group, _) in snapshot.groups() {
            if !group.built_in && group.code != Role::Customer.as_str() {
                groups.push(build_group(group, caller_is_executive));
            }
        }

        Ok(AdminPermissionMatrixData {
            version: snapshot.version(),
            modules,
            permissions,
            groups,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(AdminPermissionMatrixView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminPermissionMatrixData::export_all(&cfg)?;
    MatrixPermission::export_all(&cfg)?;
    MatrixGroup::export_all(&cfg)?;
    Ok(())
}
