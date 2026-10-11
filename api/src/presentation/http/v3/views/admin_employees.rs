//! The `admin_employees` view (§7, §12.7).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::Utc;
use pos_error::{AppError, AppResult};
use serde_json::json;
use ts_rs::TS;
use xc_auth::application::account::service::EmployeesAnswer;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_employees::{AdminEmployeeRow, AdminEmployeesData};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::extractors::actor::Accounts;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct AdminEmployeesBranchView {
    accounts: Accounts,
}

impl AdminEmployeesBranchView {
    fn new(accounts: Accounts) -> Self {
        Self { accounts }
    }
}

impl BranchView for AdminEmployeesBranchView {
    const NAME: &'static str = "admin_employees";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::OwnerRoute, "list");
    const VERSIONED: bool = false;
    type Data = AdminEmployeesData;
    type Filter = NoFilter;

    fn load(
        &self,
        _branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let accounts = self.accounts.clone();
        let who_id = who.user_id.clone();
        async move {
            match accounts
                .employees("list", &json!({}), &who_id, Utc::now())
                .await?
            {
                EmployeesAnswer::List(people) => {
                    let rows = people
                        .into_iter()
                        .map(|e| AdminEmployeeRow {
                            id: e.id,
                            username: e.username,
                            name: e.name,
                            role: e.role,
                            branch_id: e.branch_id,
                            active: e.active,
                            must_change: e.must_change,
                        })
                        .collect();
                    Ok(AdminEmployeesData { employees: rows })
                }
                EmployeesAnswer::Saved => {
                    Err(AppError::Internal("unexpected saved answer for list".to_string()))
                }
            }
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let accounts = Accounts::new(
        Arc::new(PgAccountRepository::new(state.db.clone())),
        PasswordHasher::new(),
    );
    ViewEndpoint::register(cfg, Arc::new(AdminEmployeesBranchView::new(accounts)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminEmployeeRow::export_all(&cfg)?;
    AdminEmployeesData::export_all(&cfg)?;
    Ok(())
}

