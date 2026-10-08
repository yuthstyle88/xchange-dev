//! The `me` view (§7.6).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::actor_kind::ActorKind;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use sqlx::PgPool;
use xc_auth::domain::account::repositories::account_repository::AccountRepository;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::{Me, NoFilter};
use crate::presentation::dto::v3::me::MeView;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct MeBranchView {
    accounts: PgAccountRepository,
    db: PgPool,
}

impl BranchView for MeBranchView {
    const NAME: &'static str = "me";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Data = MeView;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let branch = branch.clone();
        let who = who.clone();
        let accounts = self.accounts.clone();
        let db = self.db.clone();
        async move {
            let (name, username) = match who.kind() {
                ActorKind::Owner => {
                    let rows: Vec<(String, String)> =
                        sqlx::query_as("SELECT name, username FROM owners LIMIT 2")
                            .fetch_all(&db)
                            .await
                            .map_err(pos_error::AppError::Database)?;
                    if rows.len() == 1 {
                        let (n, u) = rows.into_iter().next().unwrap();
                        (Some(n), Some(u))
                    } else {
                        (None, None)
                    }
                }
                ActorKind::Staff => {
                    if let Some(emp) = accounts.find_employee_by_id(who.id()).await? {
                        (Some(emp.name), Some(emp.username))
                    } else {
                        (None, None)
                    }
                }
            };
            let me = Me {
                kind: match who.kind() {
                    ActorKind::Owner => "owner",
                    ActorKind::Staff => "staff",
                }
                .to_owned(),
                id: who.id().to_owned(),
                role: who.role().as_str().to_owned(),
                branch: branch.as_str().to_owned(),
                name,
                username,
            };
            Ok(MeView(me))
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let view = MeBranchView {
        accounts: PgAccountRepository::new(state.db.clone()),
        db: state.db.clone(),
    };
    ViewEndpoint::register(cfg, Arc::new(view));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MeView::export_all(&cfg)?;
    Ok(())
}
