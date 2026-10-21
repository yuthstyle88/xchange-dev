//! V.9's Employee Register view (`docs/blueprint/XCHANGE-RBAC-V9.md:107`, #3154).
//!
//! Keyset paging is on the user (`before` = last row's `userId`), so all of a user's rows land on one page.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::Utc;
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_employee_register::AdminEmployeeRegisterRow;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

type UserSqlRow = (
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

type RoleSqlRow = (String, String, Option<String>);

pub struct AdminEmployeeRegisterPagedView {
    pool: PgPool,
}

impl AdminEmployeeRegisterPagedView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl PagedView for AdminEmployeeRegisterPagedView {
    const NAME: &'static str = "admin_employee_register";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminEmployeeRegister");
    type Row = AdminEmployeeRegisterRow;
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
            let users: Vec<UserSqlRow> = sqlx::query_as(
                "SELECT u.id::text, u.employee_code, u.name, u.position, u.department, u.tell_code, u.status \
                 FROM users u JOIN branches b ON b.id = $1 AND u.organization_id = b.organization_id \
                 WHERE $2::text IS NULL \
                    OR ( \
                        (u.employee_code IS NULL, COALESCE(u.employee_code, ''), u.name, u.id::text) > \
                        (SELECT p.employee_code IS NULL, COALESCE(p.employee_code, ''), p.name, p.id::text \
                         FROM users p JOIN branches pb ON pb.id = $1 AND p.organization_id = pb.organization_id \
                         WHERE p.id::text = $2) \
                    ) \
                 ORDER BY u.employee_code IS NULL, u.employee_code, u.name, u.id \
                 LIMIT $3",
            )
            .bind(&branch)
            .bind(before.as_deref())
            .bind(i64::from(limit))
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let ids: Vec<String> = users.iter().map(|u| u.0.clone()).collect();
            let now = Utc::now();
            let roles: Vec<RoleSqlRow> = sqlx::query_as(
                "SELECT user_id::text, role_code, branch_id \
                 FROM user_roles \
                 WHERE user_id::text = ANY($1) \
                   AND valid_from <= $2 \
                   AND (valid_to IS NULL OR valid_to > $2) \
                 ORDER BY role_code, id",
            )
            .bind(&ids)
            .bind(now)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let mut out = Vec::new();
            for (user_id, employee_code, name, position, department, tell_code, status) in users {
                let user_roles: Vec<&RoleSqlRow> =
                    roles.iter().filter(|r| r.0 == user_id).collect();
                if user_roles.is_empty() {
                    out.push(AdminEmployeeRegisterRow {
                        user_id,
                        employee_code,
                        name,
                        position,
                        department,
                        tell_code,
                        role: None,
                        branch: None,
                        status,
                    });
                } else {
                    for r in user_roles {
                        out.push(AdminEmployeeRegisterRow {
                            user_id: user_id.clone(),
                            employee_code: employee_code.clone(),
                            name: name.clone(),
                            position: position.clone(),
                            department: department.clone(),
                            tell_code: tell_code.clone(),
                            role: Some(r.1.clone()),
                            branch: r.2.clone(),
                            status: status.clone(),
                        });
                    }
                }
            }
            Ok(out)
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(
        cfg,
        Arc::new(AdminEmployeeRegisterPagedView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminEmployeeRegisterRow::export_all(&cfg)?;
    Ok(())
}
