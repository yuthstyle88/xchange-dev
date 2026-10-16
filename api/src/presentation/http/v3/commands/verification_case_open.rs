//! Verification case open command (ticket #1399, design §4.4, §9.1 VF-8).
//! `POST /api/app/verification/cases`

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::document_store::DocumentStore;
use xc_kernel::ids::BranchId;
use xc_verification::application::case_service::CaseService;
use xc_verification::domain::case::case_actor::CaseActor;
use xc_verification::domain::case::case_error::CaseError;
use xc_verification::domain::case::customer_actor::CustomerActor;
use xc_verification::domain::case::verification_case::VerificationCase;
use xc_verification::infrastructure::pg_case_repository::PgCaseRepository;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, Locks, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;

pub const COMMAND_NAME: &str = "verificationCaseOpen";

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_case_open.ts")]
pub struct VerificationCaseOpenInput {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_case_open.ts")]
pub struct VerificationCaseOpenOutput {
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

impl From<VerificationCase> for VerificationCaseOpenOutput {
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

pub struct VerificationCaseOpenCommand {
    pool: PgPool,
    doc_store: Arc<dyn DocumentStore>,
}

impl VerificationCaseOpenCommand {
    pub fn new(pool: PgPool, doc_store: Arc<dyn DocumentStore>) -> Self {
        Self { pool, doc_store }
    }
}

fn map_case_error(err: CaseError) -> V3Error {
    match err {
        CaseError::NoVerificationPolicy => V3Error::new(
            "noVerificationPolicy",
            None,
            "no verification policy",
            StatusCode::BAD_REQUEST,
        ),
        CaseError::CaseLocked => V3Error::new(
            "caseLocked",
            None,
            "case locked",
            StatusCode::BAD_REQUEST,
        ),
        CaseError::DocumentTypeNotAccepted => V3Error::new(
            "documentTypeNotAccepted",
            Some("documentType"),
            "document type not accepted",
            StatusCode::BAD_REQUEST,
        ),
        CaseError::DocumentsMissing => V3Error::new(
            "documentsMissing",
            None,
            "documents missing",
            StatusCode::BAD_REQUEST,
        ),
        CaseError::OutOfScope => out_of_scope(),
        CaseError::IdempotencyConflict => V3Error::new(
            "idempotencyConflict",
            None,
            "idempotency conflict",
            StatusCode::CONFLICT,
        ),
        CaseError::File(msg) => V3Error::new(
            "validationFailed",
            None,
            msg,
            StatusCode::BAD_REQUEST,
        ),
        CaseError::NotFound => V3Error::new(
            "notFound",
            None,
            "not found",
            StatusCode::NOT_FOUND,
        ),
        CaseError::Store(_) | CaseError::Documents(_) => server_error(),
    }
}

impl CommandAction for VerificationCaseOpenCommand {
    const NAME: &'static str = COMMAND_NAME;
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, COMMAND_NAME);
    type Input = VerificationCaseOpenInput;
    type Output = VerificationCaseOpenOutput;
    const ROUTE: &'static str = "/api/app/verification/cases";
    const METHOD: Method = Method::POST;

    fn locks(&self, _branch: &BranchId, _input: &Self::Input) -> Locks {
        Locks::Org
    }

    async fn run<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        who: &'a Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let Some(customer_id) = who.customer_id.as_deref() else {
            return Err(out_of_scope());
        };

        let actor = CaseActor::Customer(CustomerActor {
            customer_id: customer_id.to_string(),
        });
        let cases = Arc::new(PgCaseRepository::new(self.pool.clone()));
        let audit = Arc::new(PgAuditLog::new(self.pool.clone()));
        let service = CaseService::new(self.pool.clone(), cases, self.doc_store.clone(), audit);

        let case = service.open(&actor, customer_id).await.map_err(map_case_error)?;
        Ok(case.into())
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(VerificationCaseOpenCommand::new(
            state.db.clone(),
            state.document_store.clone(),
        )),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    VerificationCaseOpenInput::export_all(&cfg)?;
    VerificationCaseOpenOutput::export_all(&cfg)?;
    Ok(())
}
