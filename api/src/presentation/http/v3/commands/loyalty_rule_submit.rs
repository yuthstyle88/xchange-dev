//! LY-6 (#1411, design `membership-points.md` §3, §4.6, §6.4): `loyaltyRuleSubmit` command.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;
use uuid::Uuid;
use xc_approval::application::approval_service::{ApprovalError, ApprovalService};
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_loyalty::application::rule_publisher::{LoyaltyRuleError, LoyaltyRuleWriter, RuleKind};

use super::commands::make_approval_service_with;
use crate::application::workflows::loyalty::rule_approval;
use crate::bootstrap::AppState;
use crate::bootstrap::approval_subjects::ApprovalSubjectDeps;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "loyalty_rule_submit.ts")]
pub struct LoyaltyRuleSubmitInput {
    pub version_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "loyalty_rule_submit.ts")]
pub struct LoyaltyRuleSubmitOutput {
    pub version_id: String,
    pub status: String,
    pub request_id: String,
}

pub struct LoyaltyRuleSubmitCommand {
    writer: Option<Arc<dyn LoyaltyRuleWriter>>,
    approval_service: Arc<ApprovalService>,
}

impl LoyaltyRuleSubmitCommand {
    pub fn new(
        writer: Option<Arc<dyn LoyaltyRuleWriter>>,
        approval_service: Arc<ApprovalService>,
    ) -> Self {
        Self {
            writer,
            approval_service,
        }
    }
}

fn loyalty_unavailable(message: impl Into<String>) -> V3Error {
    V3Error::new(
        "loyaltyUnavailable",
        None,
        message,
        StatusCode::SERVICE_UNAVAILABLE,
    )
}

fn map_loyalty_error(err: LoyaltyRuleError) -> V3Error {
    match err {
        LoyaltyRuleError::Unavailable(m) => loyalty_unavailable(m),
        LoyaltyRuleError::Invalid(m) => {
            V3Error::new("validationFailed", None, m, StatusCode::BAD_REQUEST)
        }
        LoyaltyRuleError::NotFound => V3Error::new(
            "notFound",
            Some("versionId"),
            "loyalty rule version not found",
            StatusCode::NOT_FOUND,
        ),
        LoyaltyRuleError::InvalidTransition(m) => {
            V3Error::new("staleVersion", None, m, StatusCode::CONFLICT)
        }
    }
}

impl CommandAction for LoyaltyRuleSubmitCommand {
    const NAME: &'static str = "loyaltyRuleSubmit";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::StaffRpc, "loyaltyRuleSubmit");
    type Input = LoyaltyRuleSubmitInput;
    type Output = LoyaltyRuleSubmitOutput;
    const ROUTE: &'static str = "/api/v3/commands/loyalty_rule_submit";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let Some(writer) = &self.writer else {
            return Err(loyalty_unavailable(
                "loyalty service token is not configured",
            ));
        };

        let version_id = Uuid::parse_str(&input.version_id).map_err(|_| {
            V3Error::new(
                "validationFailed",
                Some("versionId"),
                "invalid versionId",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let mut found_kind = None;
        for candidate in [RuleKind::Point, RuleKind::Tier] {
            if writer
                .get(candidate, version_id)
                .await
                .map_err(map_loyalty_error)?
                .is_some()
            {
                found_kind = Some(candidate);
                break;
            }
        }

        let Some(kind) = found_kind else {
            return Err(V3Error::new(
                "notFound",
                Some("versionId"),
                "loyalty rule version not found",
                StatusCode::NOT_FOUND,
            ));
        };

        let submitted = writer
            .submit(kind, version_id)
            .await
            .map_err(map_loyalty_error)?;

        let object_id = format!("{}/{version_id}", kind.as_str());
        let payload = json!({
            "kind": kind.as_str(),
            "versionId": version_id.to_string(),
        });

        let req = self
            .approval_service
            .request(
                tx,
                rule_approval::TYPE,
                &object_id,
                &payload,
                who,
                key.as_str(),
            )
            .await
            .map_err(|e| match e {
                ApprovalError::DependencyUnavailable { code, message } => {
                    V3Error::new(code, None, message, StatusCode::SERVICE_UNAVAILABLE)
                }
                ApprovalError::AlreadyPending(m) | ApprovalError::StaleVersion(m) => {
                    V3Error::new("staleVersion", None, m, StatusCode::CONFLICT)
                }
                ApprovalError::IdempotencyConflict(m) => {
                    V3Error::new("idempotencyConflict", None, m, StatusCode::CONFLICT)
                }
                ApprovalError::OutOfScope(m) => {
                    V3Error::new("outOfScope", None, m, StatusCode::FORBIDDEN)
                }
                ApprovalError::PolicyRefused { code, message } => {
                    V3Error::new(code, None, message, StatusCode::FORBIDDEN)
                }
                ApprovalError::NotFound(m) => {
                    V3Error::new("notFound", None, m, StatusCode::NOT_FOUND)
                }
                _ => server_error(),
            })?;

        Ok(LoyaltyRuleSubmitOutput {
            version_id: submitted.id.to_string(),
            status: submitted.status,
            request_id: req.id.to_string(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let deps = ApprovalSubjectDeps::from_settings(state.db.clone(), &state.settings);
    let approval_service = make_approval_service_with(&deps);
    CommandEndpoint::register(
        cfg,
        Arc::new(LoyaltyRuleSubmitCommand::new(
            deps.loyalty,
            approval_service,
        )),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    LoyaltyRuleSubmitInput::export_all(&cfg)?;
    LoyaltyRuleSubmitOutput::export_all(&cfg)?;
    Ok(())
}
