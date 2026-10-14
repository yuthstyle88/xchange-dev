//! Verification case reject command (ticket #1396, design §4.5).
//! `POST /api/operations/verification/cases/{id}/reject`

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::Actor;
use xc_auth::domain::access::branch_scope::BranchScope;
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_verification::application::review_service::{ReviewError, ReviewService};
use xc_verification::domain::case::decision_reason::DecisionReason;
use xc_verification::domain::case::reviewer_actor::{ReviewerActor, ReviewerScope};
use xc_verification::domain::case::verification_case::VerificationCase;
use xc_verification::infrastructure::pg_case_repository::PgCaseRepository;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, Locks, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_case_reject.ts")]
pub struct VerificationCaseRejectInput {
    pub id: String,
    pub expected_version: i32,
    pub reason_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_case_reject.ts")]
pub struct VerificationCaseRejectOutput {
    pub id: String,
    pub organization_id: String,
    pub customer_id: String,
    pub provider: String,
    pub status: String,
    pub policy_version: i32,
    pub branch_id: Option<String>,
    pub submitted_at: Option<String>,
    pub decided_at: Option<String>,
    pub expires_at: Option<String>,
    pub reviewed_by: Option<String>,
    pub decision_reason_code: Option<String>,
    pub version: i32,
}

impl From<VerificationCase> for VerificationCaseRejectOutput {
    fn from(c: VerificationCase) -> Self {
        Self {
            id: c.id.to_string(),
            organization_id: c.organization_id.to_string(),
            customer_id: c.customer_id,
            provider: c.provider.as_str().to_string(),
            status: c.status.as_str().to_string(),
            policy_version: c.policy_version,
            branch_id: c.branch_id.map(|b| b.to_string()),
            submitted_at: c.submitted_at.map(|t| t.to_rfc3339()),
            decided_at: c.decided_at.map(|t| t.to_rfc3339()),
            expires_at: c.expires_at.map(|t| t.to_rfc3339()),
            reviewed_by: c.reviewed_by.map(|u| u.to_string()),
            decision_reason_code: c.decision_reason.map(|r| r.as_str().to_string()),
            version: c.version,
        }
    }
}

pub struct VerificationCaseRejectCommand {
    pool: PgPool,
}

impl VerificationCaseRejectCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn reviewer_actor(who: &Actor) -> Result<ReviewerActor, V3Error> {
    let user_id = Uuid::parse_str(&who.user_id).map_err(|_| out_of_scope())?;
    let organization_id = Uuid::parse_str(&who.organization_id).map_err(|_| out_of_scope())?;
    let now = Utc::now();
    let scope = match BranchScope::of(who, Permission::VerificationReview, now) {
        BranchScope::All => ReviewerScope::Organization { organization_id },
        BranchScope::Branches(branches) => {
            let branch = who
                .branch
                .clone()
                .or_else(|| {
                    if branches.len() == 1 {
                        branches.first().cloned()
                    } else {
                        None
                    }
                })
                .ok_or_else(out_of_scope)?;
            ReviewerScope::Branch {
                organization_id,
                branch_id: branch,
            }
        }
    };
    Ok(ReviewerActor { user_id, scope })
}

fn map_review_error(err: ReviewError) -> V3Error {
    match err {
        ReviewError::StaleVersion => V3Error::new(
            "staleVersion",
            Some("expectedVersion"),
            "stale version",
            StatusCode::CONFLICT,
        ),
        ReviewError::OutOfScope => V3Error::new(
            "outOfScope",
            None,
            "out of scope",
            StatusCode::FORBIDDEN,
        ),
        ReviewError::ValidationFailed => V3Error::new(
            "validationFailed",
            None,
            "validation failed",
            StatusCode::BAD_REQUEST,
        ),
        ReviewError::NotFound => V3Error::new(
            "notFound",
            None,
            "not found",
            StatusCode::NOT_FOUND,
        ),
        ReviewError::Store(_) => server_error(),
    }
}

impl CommandAction for VerificationCaseRejectCommand {
    const NAME: &'static str = "verificationCaseReject";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "verificationCaseReject");
    type Input = VerificationCaseRejectInput;
    type Output = VerificationCaseRejectOutput;
    const ROUTE: &'static str = "/api/operations/verification/cases/{id}/reject";
    const METHOD: Method = Method::POST;

    fn locks(&self, _branch: &BranchId, _input: &Self::Input) -> Locks {
        Locks::Org
    }

    async fn run<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        who: &'a Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let reviewer = reviewer_actor(who)?;
        let reason = DecisionReason::parse(&input.reason_code).ok_or_else(|| {
            V3Error::new(
                "validationFailed",
                Some("reasonCode"),
                "unlisted reason code",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let case_id = Uuid::parse_str(&input.id).map_err(|_| {
            V3Error::new(
                "invalidRequest",
                Some("id"),
                "invalid case id",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let cases = Arc::new(PgCaseRepository::new(self.pool.clone()));
        let audit = Arc::new(PgAuditLog::new(self.pool.clone()));
        let service = ReviewService::new(self.pool.clone(), cases, audit);

        let updated = service
            .reject(&reviewer, case_id, input.expected_version, reason)
            .await
            .map_err(map_review_error)?;

        Ok(VerificationCaseRejectOutput::from(updated))
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(VerificationCaseRejectCommand::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    VerificationCaseRejectInput::export_all(&cfg)?;
    VerificationCaseRejectOutput::export_all(&cfg)?;
    Ok(())
}
