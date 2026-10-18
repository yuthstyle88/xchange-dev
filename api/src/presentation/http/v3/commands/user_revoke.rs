//! OR-5a (#945): `POST /api/users/revoke` (`userRevoke`). A05: revoke a scoped role (`UserService::revoke`); `users.version` moves.
//!
//! The organisation check, error mapping and answer are `user_grant.rs`'s shared helpers.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::application::user::user_service::creator_may_grant;
use xc_auth::domain::access::{Actor, ScopedRole};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, out_of_scope};
use crate::presentation::http::v3::error::V3Error;
use super::user_grant::{
    changed, from_change_error, parse_role, role_branch, user_in_organisation, users,
};

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "user_revoke.ts")]
pub struct UserRevokeInput {
    pub user_id: String,
    pub role: String,
    pub branch: Option<String>,
}

/// The user after the change: its id, status and the new `users.version`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "user_revoke.ts")]
pub struct UserChangeOutput {
    pub user_id: String,
    pub status: String,
    pub version: i32,
}

pub struct UserRevokeCommand(#[allow(dead_code)] PgPool);

impl CommandAction for UserRevokeCommand {
    const NAME: &'static str = "userRevoke";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "userRevoke");
    type Input = UserRevokeInput;
    type Output = UserChangeOutput;
    const ROUTE: &'static str = "/api/users/revoke";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let role = parse_role(&input.role)?;
        let role_branch = role_branch(input.branch.as_deref())?;
        let now = chrono::Utc::now();
        if !creator_may_grant(
            &who.roles,
            &[ScopedRole::new(role, role_branch.clone(), now, None)],
            now,
        ) {
            return Err(out_of_scope());
        }
        let user_id = user_in_organisation(tx, branch, &input.user_id).await?;
        users()
            .revoke(tx, who, None, &user_id, role, role_branch.as_ref(), now)
            .await
            .map_err(from_change_error)?;
        let out = changed(tx, &user_id).await?;
        Ok(UserChangeOutput {
            user_id: out.user_id,
            status: out.status,
            version: out.version,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(UserRevokeCommand(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    UserRevokeInput::export_all(&cfg)?;
    UserChangeOutput::export_all(&cfg)?;
    Ok(())
}
