//! CU-P1 (#1378): `POST /api/customers/{id}/phone-verifications/{challengeId}/confirm` (`customerPhoneConfirm`).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use pos_infra::transactions::Transactor;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_customer::application::customer::phone_verification_service::{
    PhoneVerificationError, PhoneVerificationService,
};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::infrastructure::notify::notify_sms_sender::NotifySmsSender;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomerPhoneConfirmInput {
    pub id: String,
    #[serde(alias = "challenge_id")]
    pub challenge_id: String,
    pub branch: String,
    pub code: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomerPhoneConfirmOutput {
    pub verified: bool,
    pub identity_id: Uuid,
}

pub struct CustomerPhoneConfirmCommand {
    pool: PgPool,
}

impl CustomerPhoneConfirmCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CommandAction for CustomerPhoneConfirmCommand {
    const NAME: &'static str = "customerPhoneConfirm";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "saveTransaction");
    type Input = CustomerPhoneConfirmInput;
    type Output = CustomerPhoneConfirmOutput;
    const ROUTE: &'static str = "/api/customers/{id}/phone-verifications/{challengeId}/confirm";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let challenge_id = Uuid::parse_str(&input.challenge_id).map_err(|_| {
            V3Error::new(
                "notFound",
                None,
                "invalid challenge id",
                StatusCode::NOT_FOUND,
            )
        })?;

        let sms_sender = Arc::new(NotifySmsSender::from_env());
        let branch_tx = BranchTx::new(Transactor::new(self.pool.clone()));
        let service = PhoneVerificationService::new(branch_tx, sms_sender);
        let now = Utc::now();

        let res = service
            .confirm_in_tx(tx, who, branch, &input.id, challenge_id, &input.code, now)
            .await
            .map_err(map_phone_error)?;

        Ok(CustomerPhoneConfirmOutput {
            verified: res.verified,
            identity_id: res.identity_id,
        })
    }
}

fn map_phone_error(err: PhoneVerificationError) -> V3Error {
    let status =
        StatusCode::from_u16(err.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let field = match &err {
        PhoneVerificationError::ValidationFailed(_) => Some("phone"),
        _ => None,
    };
    V3Error::new(err.code(), field, err.to_string(), status)
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(CustomerPhoneConfirmCommand::new(state.db.clone())),
    );
}

pub fn export_types(_dir: &Path) -> Result<(), ts_rs::ExportError> {
    Ok(())
}
