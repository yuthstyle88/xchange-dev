//! Verification my document link command (ticket #1399, design §4.4, §9.1 VF-8).
//! `POST /api/app/verification/documents/{id}/link`

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
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::document_store::DocumentStore;
use xc_kernel::ids::BranchId;
use xc_verification::application::access::document_access::{AccessError, DocumentAccess};
use xc_verification::application::access::download_link::DownloadLink;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::caller::Caller;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, Locks, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;

pub const COMMAND_NAME: &str = "verificationMyDocumentLink";

fn link_refused() -> V3Error {
    V3Error::new(
        "linkRefused",
        None,
        "the link is refused",
        StatusCode::FORBIDDEN,
    )
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_my_document_link.ts")]
pub struct VerificationMyDocumentLinkInput {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_my_document_link.ts")]
pub struct VerificationMyDocumentLinkOutput {
    pub url: String,
    pub expires_at: String,
}

pub struct VerificationMyDocumentLinkCommand {
    pool: PgPool,
    doc_store: Arc<dyn DocumentStore>,
}

impl VerificationMyDocumentLinkCommand {
    pub fn new(pool: PgPool, doc_store: Arc<dyn DocumentStore>) -> Self {
        Self { pool, doc_store }
    }
}

impl CommandAction for VerificationMyDocumentLinkCommand {
    const NAME: &'static str = COMMAND_NAME;
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, COMMAND_NAME);
    type Input = VerificationMyDocumentLinkInput;
    type Output = VerificationMyDocumentLinkOutput;
    const ROUTE: &'static str = "/api/app/verification/documents/{id}/link";
    const METHOD: Method = Method::POST;

    fn locks(&self, _branch: &BranchId, _input: &Self::Input) -> Locks {
        Locks::Org
    }

    async fn run<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _who: &'a Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        Err(link_refused())
    }

    async fn run_as<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        caller: &'a Caller,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let who = caller
            .user
            .as_ref()
            .ok_or_else(|| V3Error::from(PolicyError::SignIn))?;
        let Some(_customer_id) = who.customer_id.as_deref() else {
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
        let session = caller.session_hash.as_deref().ok_or_else(link_refused)?;

        let keys = xc_customer::get_pii_keys().map_err(|_| server_error())?;
        let audit = Arc::new(PgAuditLog::new(self.pool.clone()));
        let download_link = DownloadLink::new(keys);
        let access = DocumentAccess::new(
            self.pool.clone(),
            self.doc_store.clone(),
            audit,
            download_link,
        );

        let now = Utc::now();
        match access.link_for_owner(who, session, doc_id, now).await {
            Ok(issued) => Ok(VerificationMyDocumentLinkOutput {
                url: issued.url,
                expires_at: issued.expires_at.to_rfc3339(),
            }),
            Err(AccessError::OutOfScope) => Err(out_of_scope()),
            Err(AccessError::NotFound) => Err(V3Error::new(
                "notFound",
                None,
                "not found",
                StatusCode::NOT_FOUND,
            )),
            Err(other) => Err(V3Error::new(
                "serverError",
                None,
                other.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )),
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(VerificationMyDocumentLinkCommand::new(
            state.db.clone(),
            state.document_store.clone(),
        )),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    VerificationMyDocumentLinkInput::export_all(&cfg)?;
    VerificationMyDocumentLinkOutput::export_all(&cfg)?;
    Ok(())
}
