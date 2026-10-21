//! OR-5a (#945): the `admin_users` view, A05's users of the named branch's organisation, by name.
//!
//! Ordered by name, then id; `before` is the last row's `userId` of the previous page. Roles come
//! from `user_roles` (ended ones included, oldest first); no invite code and no `auth_subject`.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_users::{AdminUserRole, AdminUserRow, AdminUsersFilter};
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint, push_literal_contains};

/// `(user_id, role_code, branch_id, valid_from, valid_to)` of one `user_roles` row.
type RoleRow = (
    String,
    String,
    Option<String>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

pub struct AdminUsersPagedView {
    pool: PgPool,
}

impl AdminUsersPagedView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl PagedView for AdminUsersPagedView {
    const NAME: &'static str = "admin_users";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminUsers");
    const CURSOR_FIELD: &'static str = "userId";
    type Row = AdminUserRow;
    type Filter = AdminUsersFilter;

    fn page(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let pool = self.pool.clone();
        let branch = branch.as_str().to_owned();
        let filter = filter.clone();
        async move {
            type UserSqlRow = (
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                String,
                i32,
            );
            let mut builder = sqlx::QueryBuilder::<sqlx::Postgres>::new(
                "SELECT u.id::text, u.name, u.employee_code, u.department, u.position, u.tell_code, u.status, u.version \
                 FROM users u JOIN branches b ON b.id = ",
            );
            builder.push_bind(&branch);
            builder.push(" AND u.organization_id = b.organization_id WHERE 1=1");

            if let Some(ref before_id) = before {
                builder.push(
                    " AND (u.name, u.id::text) > (SELECT p.name, p.id::text FROM users p JOIN branches pb ON pb.id = ",
                );
                builder.push_bind(&branch);
                builder.push(" AND p.organization_id = pb.organization_id WHERE p.id::text = ");
                builder.push_bind(before_id);
                builder.push(")");
            }

            if let Some(ref q) = filter.q
                && !q.is_empty()
            {
                push_literal_contains(
                    &mut builder,
                    &["u.name", "coalesce(u.employee_code, '')"],
                    q,
                );
            }

            builder.push(" ORDER BY u.name, u.id LIMIT ");
            builder.push_bind(i64::from(limit));

            let users: Vec<UserSqlRow> = builder
                .build_query_as()
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?;

            let ids: Vec<String> = users.iter().map(|u| u.0.clone()).collect();
            let roles: Vec<RoleRow> = sqlx::query_as(
                "SELECT user_id::text, role_code, branch_id, valid_from, valid_to \
                     FROM user_roles WHERE user_id::text = ANY($1) \
                     ORDER BY valid_from, id",
            )
            .bind(&ids)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let at = |t: DateTime<Utc>| t.to_rfc3339_opts(SecondsFormat::Millis, true);
            Ok(users
                .into_iter()
                .map(
                    |(
                        user_id,
                        name,
                        employee_code,
                        department,
                        position,
                        tell_code,
                        status,
                        version,
                    )| {
                        AdminUserRow {
                            roles: roles
                                .iter()
                                .filter(|r| r.0 == user_id)
                                .map(|(_, role, branch, from, to)| AdminUserRole {
                                    role: role.clone(),
                                    branch: branch.clone(),
                                    valid_from: at(*from),
                                    valid_to: to.map(at),
                                })
                                .collect(),
                            user_id,
                            name,
                            employee_code,
                            department,
                            position,
                            tell_code,
                            status,
                            version,
                        }
                    },
                )
                .collect())
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(cfg, Arc::new(AdminUsersPagedView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::admin_users::export_types(dir)
}
