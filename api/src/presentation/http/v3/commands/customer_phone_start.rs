//! CU-P1 (#1378): `POST /api/customers/{id}/phone-verifications` (`customerPhoneStart`).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::{DateTime, Utc};
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
pub struct CustomerPhoneStartInput {
    pub id: String,
    pub branch: String,
    pub phone: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomerPhoneStartOutput {
    pub challenge_id: Uuid,
    pub expires_at: DateTime<Utc>,
}

pub struct CustomerPhoneStartCommand {
    pool: PgPool,
}

impl CustomerPhoneStartCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CommandAction for CustomerPhoneStartCommand {
    const NAME: &'static str = "customerPhoneStart";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "saveTransaction");
    type Input = CustomerPhoneStartInput;
    type Output = CustomerPhoneStartOutput;
    const ROUTE: &'static str = "/api/customers/{id}/phone-verifications";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let sms_sender = Arc::new(NotifySmsSender::from_env());
        let branch_tx = BranchTx::new(Transactor::new(self.pool.clone()));
        let service = PhoneVerificationService::new(branch_tx, sms_sender);
        let now = Utc::now();

        let res = service
            .start_in_tx(tx, who, branch, &input.id, &input.phone, now)
            .await
            .map_err(map_phone_error)?;

        Ok(CustomerPhoneStartOutput {
            challenge_id: res.challenge_id,
            expires_at: res.expires_at,
        })
    }
}

fn map_phone_error(err: PhoneVerificationError) -> V3Error {
    let status =
        StatusCode::from_u16(err.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let (field, message) = match &err {
        PhoneVerificationError::ValidationFailed(reason) => (Some("phone"), reason.clone()),
        _ => (None, err.to_string()),
    };
    V3Error::new(err.code(), field, message, status)
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(CustomerPhoneStartCommand::new(state.db.clone())),
    );
}

pub fn export_types(_dir: &Path) -> Result<(), ts_rs::ExportError> {
    Ok(())
}
