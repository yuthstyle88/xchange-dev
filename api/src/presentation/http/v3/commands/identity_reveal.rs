//! K8: `POST /api/customers/{id}/identities/{identityId}/reveal` (`identityReveal`, design
//! `customer-identity-consent.md` §4.3, §4.4). Body `{"branch", "reason"}`; `reason` is required.
//!
//! The answer holds the plain value, so the idempotency record keeps only `identityId` and `type`
//! (`stored_output`): a replay never answers the value again, and no value is written to `idempotency_records`.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use pos_error::AppError;
use pos_infra::transactions::Transactor;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_customer::application::customer::service::CustomerService;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;

/// The longest `reason` accepted, in characters.
pub const MAX_REASON_CHARS: usize = 500;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityRevealInput {
    pub id: String,
    pub identity_id: Uuid,
    pub branch: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityRevealOutput {
    pub identity_id: Uuid,
    #[serde(rename = "type")]
    pub identity_type: String,
    pub value: String,
}

pub struct IdentityRevealCommand {
    pool: PgPool,
}

impl IdentityRevealCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CommandAction for IdentityRevealCommand {
    const NAME: &'static str = "identityReveal";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "identityReveal");
    type Input = IdentityRevealInput;
    type Output = IdentityRevealOutput;
    const ROUTE: &'static str = "/api/customers/{id}/identities/{identityId}/reveal";
    const METHOD: Method = Method::POST;

    fn stored_output(&self, output: &Self::Output) -> serde_json::Value {
        json!({
            "identityId": output.identity_id,
            "type": output.identity_type,
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
        let reason = input.reason.trim();
        if reason.is_empty() || reason.chars().count() > MAX_REASON_CHARS {
            return Err(V3Error::new(
                "validationFailed",
                Some("reason"),
                "กรุณาระบุเหตุผล",
                StatusCode::BAD_REQUEST,
            ));
        }

        let service = CustomerService::new(BranchTx::new(Transactor::new(self.pool.clone())));
        let revealed = service
            .reveal_in(tx, who, branch, &input.id, input.identity_id, reason, Utc::now())
            .await
            .map_err(|err| match err {
                AppError::NotFound { .. } => {
                    V3Error::new("notFound", None, "notFound", StatusCode::NOT_FOUND)
                }
                AppError::Validation(msg) => V3Error::new(
                    "validationFailed",
                    Some("reason"),
                    msg,
                    StatusCode::BAD_REQUEST,
                ),
                AppError::Forbidden => out_of_scope(),
                other => {
                    tracing::error!(error = %other, "identityReveal failed");
                    server_error()
                }
            })?;

        Ok(IdentityRevealOutput {
            identity_id: revealed.identity_id,
            identity_type: revealed.identity_type,
            value: revealed.value,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(IdentityRevealCommand::new(state.db.clone())));
}

pub fn export_types(_dir: &Path) -> Result<(), ts_rs::ExportError> {
    Ok(())
}
