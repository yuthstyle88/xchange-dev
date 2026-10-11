//! The `me` view (§7.6).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use ts_rs::TS;
use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use xc_auth::domain::access::Actor;
use xc_auth::domain::access::authorizer::{Authorizer, Target};
use xc_auth::domain::access::role_permissions::RolePermissions;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::application::session::SessionService;
use xc_auth::application::session::session_service::SessionError;
use xc_auth::domain::access::BranchRef as SessionBranchRef;
use xc_kernel::ids::BranchId;

use sqlx::PgPool;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::{BranchRef, Me, MeRole, NoFilter};
use crate::presentation::dto::v3::me::MeView;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct MeBranchView {
    db: PgPool,
}

fn valid_at(row: &ScopedRole, now: DateTime<Utc>) -> bool {
    row.valid_from <= now && row.valid_to.is_none_or(|to| now < to)
}

/// `Me.roles` (#1030 §3.3): the actor's valid-now rows in the sign-in session route's shape, ordered by role name,
/// then branch id. `kiosk_notifications` names its caller with the same rows.
pub(crate) fn me_roles(who: &Actor, now: DateTime<Utc>) -> Vec<MeRole> {
    let mut rows: Vec<MeRole> = who
        .roles
        .iter()
        .filter(|r| valid_at(r, now))
        .map(|r| MeRole {
            role: r.role.as_str().to_owned(),
            branch_id: r.branch.as_ref().map(|b| b.as_str().to_owned()),
        })
        .collect();
    rows.sort_by(|a, b| a.role.cmp(&b.role).then_with(|| a.branch_id.cmp(&b.branch_id)));
    rows.dedup();
    rows
}

/// `Me.permissions` (#1030 §3.3): of the union of `RolePermissions::of` over the actor's roles, the ones
/// `Authorizer::check` grants at `branch`, by their serde names, sorted. `Authorizer::check` is the only judge.
pub(crate) fn me_permissions(who: &Actor, branch: &BranchId, now: DateTime<Utc>) -> Vec<String> {
    let target = Target::new(&who.organization_id, Some(branch.clone()), None);
    let mut names = BTreeSet::new();
    for row in &who.roles {
        for &p in RolePermissions::of(row.role) {
            if Authorizer::check(Some(who), p, &target, now).is_ok()
                && let Ok(serde_json::Value::String(name)) = serde_json::to_value(p)
            {
                names.insert(name);
            }
        }
    }
    names.into_iter().collect()
}

/// The `me` version (#1030 §3.3): `user_id` and the sorted `role@branch` list, so a grant or a revocation ends the
/// view's cursor the way an account switch does.
pub(crate) fn identity_version(who: &Actor) -> i64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET_BASIS;
    let mut eat = |bytes: &[u8]| {
        for &byte in bytes {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    };
    eat(who.user_id.as_bytes());
    for row in me_roles(who, Utc::now()) {
        eat(b"\0");
        eat(row.role.as_bytes());
        eat(b"@");
        eat(row.branch_id.as_deref().unwrap_or("*").as_bytes());
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
        let db = self.db.clone();
        async move {
            // A Blueprint session's S (SB-1 §14.4); `None` for the old owner and staff cookies.
            let switchable = SessionService::new(db.clone(), None, None, None)
                .switchable_of_user(&who.user_id, Utc::now())
                .await
                .map_err(|e| match e {
                    SessionError::Database(e) => pos_error::AppError::Internal(e),
                    other => pos_error::AppError::Internal(other.refusal().2),
                })?;
            me_of(&db, &who, &branch, switchable).await.map(MeView)
        }
    }
}

/// The `Me` of `who` working in `branch` (§7.6, SB-1 §14.4): the `me` view and `POST /api/auth/session/branch`
/// both answer it. `switchable` is a Blueprint session's S; without one (the old owner and staff cookies, until
/// OR-7) `branches` lists `branch` alone.
pub(crate) async fn me_of(
    db: &PgPool,
    who: &Actor,
    branch: &BranchId,
    switchable: Option<Vec<SessionBranchRef>>,
) -> AppResult<Me> {
    let now = Utc::now();
    // `users.name` by id (#1030 §3.3); an old cookie's actor (`"owner"`, a staff id) has no `users` row.
    let name: Option<String> =
        sqlx::query_scalar::<_, Option<String>>("SELECT name FROM users WHERE id::text = $1")
            .bind(&who.user_id)
            .fetch_optional(db)
            .await
            .map_err(pos_error::AppError::Database)?
            .flatten();
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
        id: who.user_id.clone(),
        name,
        roles: me_roles(who, now),
        permissions: me_permissions(who, branch, now),
        branch: branch.as_str().to_owned(),
        branch_name,
        branches,
    })
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let view = MeBranchView {
        db: state.db.clone(),
    };
    ViewEndpoint::register(cfg, Arc::new(view));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MeView::export_all(&cfg)?;
    Ok(())
}
