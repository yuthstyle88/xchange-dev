//! OR-5a (#945): `POST /api/users/disable` (`userDisable`). A05: disable a user (`UserService::disable`); `users.version` moves, so sessions end.
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
use xc_auth::domain::user::UserRepository;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, request_ip,
};
use crate::presentation::http::v3::error::V3Error;
use super::user_grant::{
    changed, custody_holds, from_app_error, from_change_error, user_in_organisation, users,
};

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "user_disable.ts")]
pub struct UserDisableInput {
    pub user_id: String,
}

/// The user after the change: its id, status and the new `users.version`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "user_disable.ts")]
pub struct UserChangeOutput {
    pub user_id: String,
    pub status: String,
    pub version: i32,
}

pub struct UserDisableCommand(#[allow(dead_code)] PgPool);

impl CommandAction for UserDisableCommand {
    const NAME: &'static str = "userDisable";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "userDisable");
    type Input = UserDisableInput;
    type Output = UserChangeOutput;
    const ROUTE: &'static str = "/api/users/disable";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let user_id = user_in_organisation(tx, branch, &input.user_id).await?;
        let now = chrono::Utc::now();
        let live_roles: Vec<ScopedRole> = users()
            .repo()
            .find_roles_by_user_id(tx, &user_id)
            .await
            .map_err(from_app_error)?
            .into_iter()
            .filter(|r| r.valid_from <= now && r.valid_to.is_none_or(|to| to > now))
            .collect();
        if !creator_may_grant(&who.roles, &live_roles, now) {
            return Err(out_of_scope());
        }
        users()
            .disable(tx, who, request_ip().as_deref(), &user_id, custody_holds().as_ref())
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
    CommandEndpoint::register(cfg, Arc::new(UserDisableCommand(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    UserDisableInput::export_all(&cfg)?;
    UserChangeOutput::export_all(&cfg)?;
    Ok(())
}
