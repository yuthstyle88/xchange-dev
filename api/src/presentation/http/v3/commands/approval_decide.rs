//! Approval decide command (§4.7, §5, §6).

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_approval::application::approval_service::{ApprovalError, ApprovalService};
use xc_approval::domain::request::{Outcome, RequestId};
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use super::commands::make_approval_service;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "approval_decide.ts")]
pub struct ApprovalDecideInput {
    pub id: String,
    pub outcome: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "approval_decide.ts")]
pub struct ApprovalDecideOutput {
    pub status: String,
}

pub struct ApprovalDecideCommand {
    service: Arc<ApprovalService>,
}

impl ApprovalDecideCommand {
    pub fn new(service: Arc<ApprovalService>) -> Self {
        Self { service }
    }
}

impl CommandAction for ApprovalDecideCommand {
    const NAME: &'static str = "approvalDecide";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "approvalDecide");
    type Input = ApprovalDecideInput;
    type Output = ApprovalDecideOutput;
    const ROUTE: &'static str = "/api/approvals/{id}/decide";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let req_uuid = Uuid::from_str(&input.id).map_err(|_| {
            V3Error::new(
                "validationFailed",
                Some("id"),
                "invalid request id",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let outcome = match input.outcome.as_str() {
            "APPROVED" => Outcome::Approved,
            "REJECTED" => Outcome::Rejected,
            _ => {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("outcome"),
                    "outcome must be APPROVED or REJECTED",
                    StatusCode::BAD_REQUEST,
                ));
            }
        };

        self.service
            .decide(
                tx,
                &RequestId::new(req_uuid),
                who,
                outcome,
                &input.reason,
                key.as_str(),
            )
            .await
            .map_err(|e| match e {
                ApprovalError::NotFound(m) => {
                    V3Error::new("notFound", None, m, StatusCode::NOT_FOUND)
                }
                ApprovalError::PolicyRefused { code, message } => {
                    V3Error::new(code, None, message, StatusCode::FORBIDDEN)
                }
                ApprovalError::OutOfScope(m) => {
                    V3Error::new("outOfScope", None, m, StatusCode::FORBIDDEN)
                }
                ApprovalError::StaleVersion(m) => {
                    V3Error::new("staleVersion", None, m, StatusCode::CONFLICT)
                }
                ApprovalError::IdempotencyConflict(m) => {
                    V3Error::new("idempotencyConflict", None, m, StatusCode::CONFLICT)
                }
                _ => server_error(),
            })?;

        Ok(ApprovalDecideOutput {
            status: input.outcome,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let approval_service = make_approval_service(state.db.clone());
    CommandEndpoint::register(cfg, Arc::new(ApprovalDecideCommand::new(approval_service)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ApprovalDecideInput::export_all(&cfg)?;
    ApprovalDecideOutput::export_all(&cfg)?;
    Ok(())
}
