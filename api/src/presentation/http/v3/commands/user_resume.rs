//! RBAC f13 (F1b, #3149): `POST /api/users/resume` (`userResume`). Resume a user (`UserService::resume`); `users.version` moves.
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

use super::user_disable::UserChangeOutput;
use super::user_grant::{changed, from_change_error, user_in_organisation, users};
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "user_resume.ts")]
pub struct UserResumeInput {
    pub user_id: String,
}

pub struct UserResumeCommand(#[allow(dead_code)] PgPool);

impl CommandAction for UserResumeCommand {
    const NAME: &'static str = "userResume";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "userResume");
    type Input = UserResumeInput;
    type Output = UserChangeOutput;
    const ROUTE: &'static str = "/api/users/resume";
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
            .resume(tx, who, None, &user_id)
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
    CommandEndpoint::register(cfg, Arc::new(UserResumeCommand(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    UserResumeInput::export_all(&cfg)?;
    Ok(())
}
