//! The `transaction_documents` view (§7, §12.2).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;

use crate::application::workflows::documents::DocumentsLookup;
use crate::bootstrap::AppState;
use crate::infrastructure::files::file_service::FileService;
use crate::presentation::dto::v3::transaction_documents::{
    TransactionDocumentFile, TransactionDocumentKind, TransactionDocumentsData,
    TransactionDocumentsFilter,
};
use crate::presentation::dto::v3::transaction_row::TransactionRow;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct TransactionDocumentsBranchView {
    lookup: DocumentsLookup,
}

impl TransactionDocumentsBranchView {
    fn new(lookup: DocumentsLookup) -> Self {
        Self { lookup }
    }
}

fn ending(mime: &str) -> &'static str {
    match mime {
        "image/svg+xml" => "svg",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "application/pdf" => "pdf",
        "text/html" => "html",
        _ => "bin",
    }
}

impl BranchView for TransactionDocumentsBranchView {
    const NAME: &'static str = "transaction_documents";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::UserRoute, "documents");
    type Data = TransactionDocumentsData;
    type Filter = TransactionDocumentsFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let lookup = self.lookup.clone();
        let branch = branch.clone();
        let r#ref = filter.r#ref.clone();
        async move {
            let found = lookup.find(&branch, &r#ref).await;
            let files = match found {
                Ok(docs) => {
                    let mut result = Vec::new();
                    for link in docs.links {
                        if link.label == crate::application::workflows::documents::RECEIPT {
                            result.push(TransactionDocumentFile {
                                kind: TransactionDocumentKind::Receipt,
                                path: link.url,
                                download: Some(format!(
                                    "{}_RECEIPT_FULL.{}",
                                    docs.document_ref,
                                    ending(&link.mime)
                                )),
                            });
                        } else if link.label == "ภาพรายละเอียดใบเสร็จ" {
                            result.push(TransactionDocumentFile {
                                kind: TransactionDocumentKind::ReceiptImage,
                                path: link.url,
                                download: Some(format!(
                                    "{}_RECEIPT.{}",
                                    docs.document_ref,
                                    ending(&link.mime)
                                )),
                            });
                        }
                    }
                    result
                }
                Err(AppError::Domain(ref msg))
                    if msg == crate::application::workflows::documents::NO_DOCUMENTS =>
                {
                    Vec::new()
                }
                Err(err) => return Err(err),
            };
            Ok(TransactionDocumentsData { r#ref, files })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branch_tx = || BranchTx::new(Transactor::new(state.db.clone()));
    let lookup = DocumentsLookup::new(
        TransactionService::new(
            Arc::new(PgTransactionRepository::new(state.db.clone())),
            branch_tx(),
        ),
        BranchService::new(branch_tx()),
        FileService::new(state.db.clone(), state.settings.files.dir.clone()),
    );
    ViewEndpoint::register(
        cfg,
        Arc::new(TransactionDocumentsBranchView::new(lookup)),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    TransactionDocumentsFilter::export_all(&cfg)?;
    TransactionDocumentFile::export_all(&cfg)?;
    TransactionDocumentsData::export_all(&cfg)?;
    TransactionRow::export_all(&cfg)?;
    Ok(())
}
