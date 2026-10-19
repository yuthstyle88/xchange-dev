//! RBAC f15 (F3, #3151): `POST /api/users/profile` (`userProfileSave`). Save profile fields.
//!
//! The organisation check, error mapping and answer are `user_grant.rs`'s shared helpers.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use pos_error::AppError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::application::user::user_service::UserChangeError;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::user::UserProfile;
use xc_kernel::ids::BranchId;

use super::user_grant::{changed, from_change_error, user_in_organisation, users};
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, request_ip,
};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "user_profile_save.ts")]
pub struct UserProfileSaveInput {
    pub user_id: String,
    pub employee_code: Option<String>,
    pub department: Option<String>,
    pub position: Option<String>,
    pub tell_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "user_profile_save.ts")]
pub struct UserProfileSaveOutput {
    pub user_id: String,
    pub employee_code: Option<String>,
    pub department: Option<String>,
    pub position: Option<String>,
    pub tell_code: Option<String>,
    pub version: i32,
}

pub struct UserProfileSaveCommand(#[allow(dead_code)] PgPool);

impl CommandAction for UserProfileSaveCommand {
    const NAME: &'static str = "userProfileSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "userProfileSave");
    type Input = UserProfileSaveInput;
    type Output = UserProfileSaveOutput;
    const ROUTE: &'static str = "/api/users/profile";
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
        let profile = UserProfile {
            employee_code: input.employee_code,
            department: input.department,
            position: input.position,
            tell_code: input.tell_code,
        };
        let saved = users()
            .save_profile(tx, who, request_ip().as_deref(), &user_id, &profile)
            .await
            .map_err(|e| match e {
                UserChangeError::App(AppError::Conflict(ref m)) if m == "employeeCodeTaken" => {
                    V3Error::new(
                        "employeeCodeTaken",
                        Some("employeeCode"),
                        "employee code already used",
                        StatusCode::CONFLICT,
                    )
                }
                other => from_change_error(other),
            })?;
        let out = changed(tx, &user_id).await?;
        Ok(UserProfileSaveOutput {
            user_id: out.user_id,
            employee_code: saved.employee_code,
            department: saved.department,
            position: saved.position,
            tell_code: saved.tell_code,
            version: out.version,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(UserProfileSaveCommand(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    UserProfileSaveInput::export_all(&cfg)?;
    UserProfileSaveOutput::export_all(&cfg)?;
    Ok(())
}
