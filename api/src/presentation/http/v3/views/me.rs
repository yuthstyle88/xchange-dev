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
use chrono::Utc;
use xc_auth::application::session::SessionService;
use xc_auth::application::session::session_service::SessionError;
use xc_auth::domain::access::BranchRef as SessionBranchRef;
use xc_kernel::ids::BranchId;

use sqlx::PgPool;
use xc_auth::domain::account::repositories::account_repository::AccountRepository;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::{BranchRef, Me, NoFilter};
use crate::presentation::dto::v3::me::MeView;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct MeBranchView {
    accounts: PgAccountRepository,
    db: PgPool,
}

pub(crate) fn identity_version(who: &Actor) -> i64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in who.id().as_bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash ^= 0; // '\0'
    hash = hash.wrapping_mul(FNV_PRIME);
    for &byte in who.role().as_str().as_bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    (hash & 0x001f_ffff_ffff_ffff) as i64
}

impl BranchView for MeBranchView {
    const NAME: &'static str = "me";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Data = MeView;
    type Filter = NoFilter;

    fn own_version(
        &self,
        _branch: &BranchId,
        who: &Actor,
    ) -> impl Future<Output = AppResult<Option<i64>>> + Send {
        let v = identity_version(who);
        async move { Ok(Some(v)) }
    }

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
            // A Blueprint session's S (SB-1 §14.4); `None` for the old owner and staff cookies.
            let switchable = SessionService::new(db.clone(), None, None, None)
                .switchable_of_user(who.id(), Utc::now())
                .await
                .map_err(|e| match e {
                    SessionError::Database(e) => pos_error::AppError::Internal(e),
                    other => pos_error::AppError::Internal(other.refusal().2),
                })?;
            me_of(&db, &accounts, &who, &branch, switchable).await.map(MeView)
        }
    }
}

/// The `Me` of `who` working in `branch` (§7.6, SB-1 §14.4): the `me` view and `POST /api/auth/session/branch`
/// both answer it. `switchable` is a Blueprint session's S; without one (the old owner and staff cookies, until
/// OR-7) `branches` lists `branch` alone.
pub(crate) async fn me_of(
    db: &PgPool,
    accounts: &PgAccountRepository,
    who: &Actor,
    branch: &BranchId,
    switchable: Option<Vec<SessionBranchRef>>,
) -> AppResult<Me> {
    let (name, username) = match who.kind() {
        ActorKind::Owner => {
            let rows: Vec<(String, String)> =
                sqlx::query_as("SELECT name, username FROM owners LIMIT 2")
                    .fetch_all(db)
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
    let branches: Vec<BranchRef> = match switchable {
        Some(s) => s
            .into_iter()
            .map(|b| BranchRef {
                id: b.id.as_str().to_owned(),
                code: b.code,
                name: b.name,
            })
            .collect(),
        None => {
            let row: Option<(String, String, String)> =
                sqlx::query_as("SELECT id, COALESCE(code, id), name FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_optional(db)
                    .await
                    .map_err(pos_error::AppError::Database)?;
            row.map(|(id, code, name)| BranchRef { id, code, name })
                .into_iter()
                .collect()
        }
    };
    let branch_name = match branches.iter().find(|b| b.id == branch.as_str()) {
        Some(b) => b.name.clone(),
        None => sqlx::query_scalar::<_, String>("SELECT name FROM branches WHERE id = $1")
            .bind(branch.as_str())
            .fetch_optional(db)
            .await
            .map_err(pos_error::AppError::Database)?
            .unwrap_or_else(|| branch.as_str().to_owned()),
    };
    Ok(Me {
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
        branch_name,
        branches,
    })
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
