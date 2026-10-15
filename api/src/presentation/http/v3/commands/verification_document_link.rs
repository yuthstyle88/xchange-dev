//! Verification document link command (ticket #1396, design §4.5).
//! `POST /api/operations/verification/documents/{id}/link`

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
use xc_auth::application::session::sha256_hex;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::document_store::DocumentStore;
use xc_kernel::ids::BranchId;
use xc_verification::application::access::document_access::{AccessError, DocumentAccess};
use xc_verification::application::access::download_link::DownloadLink;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, Locks, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;

const COMMAND_NAME: &str = "verificationDocumentLink";

pub struct VerificationDocumentLinkCommand {
    pool: PgPool,
    doc_store: Arc<dyn DocumentStore>,
}

impl VerificationDocumentLinkCommand {
    pub fn new(pool: PgPool, doc_store: Arc<dyn DocumentStore>) -> Self {
        Self { pool, doc_store }
    }
}

impl CommandAction for VerificationDocumentLinkCommand {
    const NAME: &'static str = COMMAND_NAME;
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, COMMAND_NAME);
    type Input = VerificationDocumentLinkInput;
    type Output = VerificationDocumentLinkOutput;
    const ROUTE: &'static str = "/api/operations/verification/documents/{id}/link";
    const METHOD: Method = Method::POST;

    fn locks(&self, _branch: &BranchId, _input: &Self::Input) -> Locks {
        Locks::Org
    }

    async fn run<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        who: &'a Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let doc_id = Uuid::parse_str(&input.id).map_err(|_| {
            V3Error::new(
                "invalidRequest",
                Some("id"),
                "invalid document id",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let keys = xc_customer::get_pii_keys().map_err(|_| server_error())?;
        let audit = Arc::new(PgAuditLog::new(self.pool.clone()));
        let download_link = DownloadLink::new(keys);
        let access = DocumentAccess::new(
            self.pool.clone(),
            self.doc_store.clone(),
            audit,
            download_link,
        );

        let session: String = sqlx::query_scalar(
            "SELECT hash FROM user_sessions WHERE user_id::text = $1 AND expires > now() ORDER BY expires DESC LIMIT 1",
        )
        .bind(&who.user_id)
        .fetch_optional(&mut **tx)
        .await
        .ok()
        .flatten()
        // stopgap: see #1943
        .unwrap_or_else(|| sha256_hex(&format!("session-{}", who.user_id)));

        let now = Utc::now();
        match access.link_for_reviewer(who, &session, doc_id, now).await {
            Ok(issued) => Ok(VerificationDocumentLinkOutput {
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

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_document_link.ts")]
pub struct VerificationDocumentLinkInput {
    pub id: String,
    pub expected_version: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_document_link.ts")]
pub struct VerificationDocumentLinkOutput {
    pub url: String,
    pub expires_at: String,
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(VerificationDocumentLinkCommand::new(
            state.db.clone(),
            state.document_store.clone(),
        )),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    VerificationDocumentLinkInput::export_all(&cfg)?;
    VerificationDocumentLinkOutput::export_all(&cfg)?;
    Ok(())
}
