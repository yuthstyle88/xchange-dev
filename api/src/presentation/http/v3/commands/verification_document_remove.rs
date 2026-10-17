//! Verification document remove command (ticket #1399, design §4.4, §9.1 VF-8).
//! `POST /api/app/verification/documents/{id}/remove`

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::document_store::DocumentStore;
use xc_kernel::ids::BranchId;
use xc_verification::application::case_service::CaseService;
use xc_verification::domain::case::case_actor::CaseActor;
use xc_verification::domain::case::case_error::CaseError;
use xc_verification::domain::case::customer_actor::CustomerActor;
use xc_verification::infrastructure::pg_case_repository::PgCaseRepository;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, Locks, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;

pub const COMMAND_NAME: &str = "verificationDocumentRemove";

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_document_remove.ts")]
pub struct VerificationDocumentRemoveInput {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_document_remove.ts")]
pub struct VerificationDocumentRemoveOutput {
    pub ok: bool,
}

pub struct VerificationDocumentRemoveCommand {
    pool: PgPool,
    doc_store: Arc<dyn DocumentStore>,
}

impl VerificationDocumentRemoveCommand {
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

impl CommandAction for VerificationDocumentRemoveCommand {
    const NAME: &'static str = COMMAND_NAME;
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, COMMAND_NAME);
    type Input = VerificationDocumentRemoveInput;
    type Output = VerificationDocumentRemoveOutput;
    const ROUTE: &'static str = "/api/app/verification/documents/{id}/remove";
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
        let Some(customer_id) = who.customer_id.as_deref() else {
            return Err(out_of_scope());
        };

        let doc_id = Uuid::parse_str(&input.id).map_err(|_| {
            V3Error::new(
                "invalidRequest",
                Some("id"),
                "invalid document id",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let actor = CaseActor::Customer(CustomerActor {
            customer_id: customer_id.to_string(),
        });
        let cases = Arc::new(PgCaseRepository::new(self.pool.clone()));
        let audit = Arc::new(PgAuditLog::new(self.pool.clone()));
        let service = CaseService::new(self.pool.clone(), cases, self.doc_store.clone(), audit);

        service
            .remove_document(&actor, doc_id)
            .await
            .map_err(map_case_error)?;
        Ok(VerificationDocumentRemoveOutput { ok: true })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(VerificationDocumentRemoveCommand::new(
            state.db.clone(),
            state.document_store.clone(),
        )),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    VerificationDocumentRemoveInput::export_all(&cfg)?;
    VerificationDocumentRemoveOutput::export_all(&cfg)?;
    Ok(())
}
