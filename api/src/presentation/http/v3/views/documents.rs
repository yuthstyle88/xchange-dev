//! The `documents` and `staff_documents` views (#1349, design `xchange-v3-wave3-report-documents.md` §A D-A1):
//! one exchange's documents, for the owner's page `/documents` and the staff page `/staff/documents`. The two
//! differ only in their name, the entry path they inherit, and how a link is written, so one view serves both.

use std::future::Future;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::web;
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use serde_json::Value;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;

use crate::application::compat::staff_view::rewrite_urls;
use crate::application::workflows::documents::{DocumentKind, DocumentsLookup, NO_DOCUMENTS};
use crate::bootstrap::AppState;
use crate::infrastructure::files::file_service::FileService;
use crate::presentation::dto::v3::documents::{
    DocumentsData, DocumentsFilter, DocumentsLink, DocumentsLinkKind,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

/// What differs between the two documents views (§A.3).
pub trait DocumentsAudience: Send + Sync + 'static {
    const NAME: &'static str;
    const INHERITS: (EntryPath, &'static str);
    /// Whether a link carries v2's download name.
    const SHOW_DOWNLOAD: bool;
    /// A file's link for this audience, from the owner's `/files/<key>`.
    fn href(url: &str) -> String;
}

/// `/documents`: the owner and every signed-in employee (`DocumentsList`).
pub struct OwnerDocuments;

impl DocumentsAudience for OwnerDocuments {
    const NAME: &'static str = "documents";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::UserRoute, "documents");
    const SHOW_DOWNLOAD: bool = true;

    fn href(url: &str) -> String {
        url.to_owned()
    }
}

/// `/staff/documents`: the staff session (`IdentityDocumentRead`), through the staff file door.
pub struct StaffDocuments;

impl DocumentsAudience for StaffDocuments {
    const NAME: &'static str = "staff_documents";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::StaffRpc, "documents");
    const SHOW_DOWNLOAD: bool = false;

    fn href(url: &str) -> String {
        // the staff path's own rewriting of links (T11s)
        match rewrite_urls(Value::String(url.to_owned())) {
            Value::String(href) => href,
            _ => url.to_owned(),
        }
    }
}

pub struct DocumentsBranchView<A> {
    lookup: DocumentsLookup,
    audience: PhantomData<fn() -> A>,
}

impl<A: DocumentsAudience> DocumentsBranchView<A> {
    fn new(lookup: DocumentsLookup) -> Self {
        Self {
            lookup,
            audience: PhantomData,
        }
    }
}

impl<A: DocumentsAudience> BranchView for DocumentsBranchView<A> {
    const NAME: &'static str = A::NAME;
    const INHERITS: (EntryPath, &'static str) = A::INHERITS;
    type Data = DocumentsData;
    type Filter = DocumentsFilter;

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
            let found = lookup.find(&branch, &r#ref).await?;
            let links = found
                .links
                .iter()
                .map(|link| DocumentsLink {
                    kind: match link.kind() {
                        DocumentKind::Passport => DocumentsLinkKind::Passport,
                        DocumentKind::PaymentProof => DocumentsLinkKind::PaymentProof,
                        DocumentKind::Receipt => DocumentsLinkKind::Receipt,
                    },
                    href: A::href(&link.url),
                    download: A::SHOW_DOWNLOAD.then(|| link.file_name(&found.document_ref)),
                })
                .collect();
            Ok(DocumentsData {
                r#ref: found.document_ref,
                links,
            })
        }
    }

    /// v2's page was a 404, so not found is a refusal, never an empty list.
    fn refusal(&self, err: &AppError) -> Option<V3Error> {
        match err {
            AppError::Domain(sentence) if sentence == NO_DOCUMENTS => Some(V3Error::new(
                "documentsNotFound",
                Some("ref"),
                NO_DOCUMENTS,
                StatusCode::NOT_FOUND,
            )),
            _ => None,
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
        Arc::new(DocumentsBranchView::<OwnerDocuments>::new(lookup.clone())),
    );
    ViewEndpoint::register(
        cfg,
        Arc::new(DocumentsBranchView::<StaffDocuments>::new(lookup)),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DocumentsFilter::export_all(&cfg)?;
    DocumentsLinkKind::export_all(&cfg)?;
    DocumentsLink::export_all(&cfg)?;
    DocumentsData::export_all(&cfg)?;
    Ok(())
}
