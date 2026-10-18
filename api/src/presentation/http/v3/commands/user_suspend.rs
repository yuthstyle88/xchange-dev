//! OR-5b (#3148): `POST /api/users/suspend` (`userSuspend`). A05: suspend a user (`UserService::suspend`); `users.version` moves, so sessions end.
//!
//! The organisation check, error mapping and answer are `user_grant.rs`'s shared helpers.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;
use super::user_disable::UserChangeOutput;
use super::user_grant::{
    changed, custody_holds, from_change_error, user_in_organisation, users,
};

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "user_suspend.ts")]
pub struct UserSuspendInput {
    pub user_id: String,
}

pub struct UserSuspendCommand(#[allow(dead_code)] PgPool);

impl CommandAction for UserSuspendCommand {
    const NAME: &'static str = "userSuspend";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "userSuspend");
    type Input = UserSuspendInput;
    type Output = UserChangeOutput;
    const ROUTE: &'static str = "/api/users/suspend";
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
        users()
            .suspend(tx, who, None, &user_id, custody_holds().as_ref())
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
    CommandEndpoint::register(cfg, Arc::new(UserSuspendCommand(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    UserSuspendInput::export_all(&cfg)?;
    Ok(())
}
