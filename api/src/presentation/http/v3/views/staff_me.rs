//! The `staff_me` view (§3.1).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use xc_auth::domain::access::RolePermissions;
use xc_auth::domain::access::v3_roles::actor_from_v3;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::staff_me::{StaffMeView, StaffRoleDto};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct StaffMeBranchView {
    pool: PgPool,
}

impl StaffMeBranchView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for StaffMeBranchView {
    const NAME: &'static str = "staff_me";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::StaffRpc, "staffMe");
    const VERSIONED: bool = true;
    type Data = StaffMeView;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        let who = who.clone();

        async move {
            let user_id = who.id();
            let name: String = sqlx::query_scalar("SELECT name FROM users WHERE id::text = $1")
                .bind(user_id)
                .fetch_optional(&pool)
                .await
                .map_err(AppError::Database)?
                .unwrap_or_default();

            let branch_name: String = sqlx::query_scalar("SELECT name FROM branches WHERE id = $1")
                .bind(branch.as_str())
                .fetch_optional(&pool)
                .await
                .map_err(AppError::Database)?
                .unwrap_or_default();

            let user = actor_from_v3(
                &who,
                "00000000-0000-0000-0000-000000000001",
                DateTime::<Utc>::UNIX_EPOCH,
            );

            let roles: Vec<StaffRoleDto> = user
                .roles
                .iter()
                .map(|r| StaffRoleDto {
                    role: r.role.as_str().to_string(),
                    branch_id: r.branch.as_ref().map(|b| b.to_string()),
                })
                .collect();

            let mut permissions: Vec<String> = user
                .roles
                .iter()
                .flat_map(|r| RolePermissions::of(r.role).iter().copied())
                .map(|p| serde_json::to_string(&p).unwrap().trim_matches('"').to_string())
                .collect();
            permissions.sort();
            permissions.dedup();

            Ok(StaffMeView {
                user_id: user_id.to_string(),
                name,
                roles,
                branch_id: branch.to_string(),
                branch_name,
                permissions,
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(StaffMeBranchView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::staff_me::export_types(dir)
}
