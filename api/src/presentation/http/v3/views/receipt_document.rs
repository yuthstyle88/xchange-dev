//! The B04 receipt document reprint view (ticket #1294, design `desk-transaction-detail.md` §4.2).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Row};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::files::file_service::FileService;
use crate::presentation::dto::v3::receipt_document::{
    ReceiptDocumentDto, ReceiptDocumentFilter, ReceiptDocumentViewData,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct ReceiptDocumentView {
    pool: PgPool,
    files: FileService,
}

impl BranchView for ReceiptDocumentView {
    const NAME: &'static str = "receipt_document";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "receiptReprint");
    const VERSIONED: bool = false;
    type Data = ReceiptDocumentViewData;
    type Filter = ReceiptDocumentFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let files = self.files.clone();
        let branch = branch.clone();
        let number = filter.number.clone();

        async move {
            let none = Ok(ReceiptDocumentViewData { receipt: None });
            let Some(number) = number.filter(|s| !s.is_empty()) else {
                return none;
            };

            let row = sqlx::query(
                "SELECT r.number, r.revision, r.issued_at, r.document_ref \
                 FROM receipts r \
                 JOIN transactions t ON t.id = r.transaction_id \
                 WHERE r.number = $1 AND t.branch_id = $2",
            )
            .bind(&number)
            .bind(branch.as_str())
            .fetch_optional(&pool)
            .await
            .map_err(AppError::Database)?;

            let Some(row) = row else {
                return none;
            };

            let doc_ref: String = row.get("document_ref");
            let stored = files.open_in_branch(&doc_ref, &branch).await?;
            let Some(file) = stored else {
                return none;
            };

            let html = String::from_utf8(file.bytes)
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let issued_at: DateTime<Utc> = row.get("issued_at");

            Ok(ReceiptDocumentViewData {
                receipt: Some(ReceiptDocumentDto {
                    number: row.get("number"),
                    revision: row.get("revision"),
                    issued_at: issued_at.to_rfc3339(),
                    html,
                }),
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let files = FileService::new(state.db.clone(), state.settings.files.dir.clone());
    ViewEndpoint::register(
        cfg,
        Arc::new(ReceiptDocumentView {
            pool: state.db.clone(),
            files,
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::receipt_document::export_types(dir)
}
