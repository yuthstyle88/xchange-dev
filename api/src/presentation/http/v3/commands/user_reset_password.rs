//! F2-BE (#5835): `POST /api/users/reset-password` (`userResetPassword`, design
//! `docs/design/blueprint/user-management-rbac.md` §13). An admin resets a staff member's
//! password through Identity and sees the temporary password once.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;
use xc_auth::application::user::user_service::ResetPasswordError;
use xc_auth::domain::access::Actor;
use xc_auth::domain::identity::identity_client::{IdentityClient, IdentityError};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::infrastructure::identity::http_identity_client::HttpIdentityClient;
use xc_kernel::ids::BranchId;

use super::user_grant::{from_change_error, user_in_organisation, users};
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::client_address::CLIENT_ADDRESS_HEADER;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, request_ip};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "user_reset_password.ts")]
pub struct UserResetPasswordInput {
    pub user_id: String,
}

#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "user_reset_password.ts")]
pub struct UserResetPasswordOutput {
    pub user_id: String,
    pub temporary_password: String,
    pub must_change_password: bool,
}

impl std::fmt::Debug for UserResetPasswordOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserResetPasswordOutput")
            .field("user_id", &self.user_id)
            .field("must_change_password", &self.must_change_password)
            .finish_non_exhaustive()
    }
}

pub struct UserResetPasswordCommand {
    identity: Option<Arc<dyn IdentityClient>>,
}

impl CommandAction for UserResetPasswordCommand {
    const NAME: &'static str = "userResetPassword";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "userResetPassword");
    type Input = UserResetPasswordInput;
    type Output = UserResetPasswordOutput;
    const ROUTE: &'static str = "/api/users/reset-password";
    const METHOD: Method = Method::POST;

    fn stored_output(&self, output: &Self::Output) -> serde_json::Value {
        json!({
            "userId": output.user_id,
            "mustChangePassword": output.must_change_password,
        })
    }

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let user_id = user_in_organisation(tx, branch, &input.user_id).await?;
        let Some(ref identity) = self.identity else {
            let (status, code, message) = IdentityError::unavailable(
                "/admin/identities/{id}/password",
                503,
                "not_configured",
            )
            .refusal();
            return Err(V3Error::new(
                code,
                None,
                message,
                StatusCode::from_u16(status).unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
            ));
        };
        let temp = users()
            .reset_password(
                tx,
                who,
                request_ip().as_deref(),
                &user_id,
                identity.as_ref(),
            )
            .await
            .map_err(|e| match e {
                ResetPasswordError::Change(c) => from_change_error(c),
                ResetPasswordError::Identity(ie) => {
                    let (status, code, message) = ie.refusal();
                    V3Error::new(
                        code,
                        None,
                        message,
                        StatusCode::from_u16(status).unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
                    )
                }
            })?;
        Ok(UserResetPasswordOutput {
            user_id,
            temporary_password: temp.expose().to_owned(),
            must_change_password: true,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let identity = state.settings.identity.clone().map(|settings| {
        Arc::new(HttpIdentityClient::new(settings, CLIENT_ADDRESS_HEADER))
            as Arc<dyn IdentityClient>
    });
    CommandEndpoint::register(cfg, Arc::new(UserResetPasswordCommand { identity }));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    UserResetPasswordInput::export_all(&cfg)?;
    UserResetPasswordOutput::export_all(&cfg)?;
    Ok(())
}
