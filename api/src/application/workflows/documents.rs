//! The documents of one exchange: v2's action `documents`.
//!
//! Reads through `TransactionService`, `BranchService` and `FileService`; decides nothing
//! about who may ask (the routes and the RPC dispatcher do).

use pos_error::{AppError, AppResult};
use serde_json::{Value, json};
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;

use crate::infrastructure::files::file_service::FileService;

pub const NO_DOCUMENTS: &str = "ไม่พบเอกสารรายการ";
pub const PASSPORT: &str = "พาสปอร์ต";
pub const PAYMENT_PROOF: &str = "หลักฐานชำระเงิน";
pub const RECEIPT: &str = "ใบเสร็จ";

/// One document of an exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentLink {
    /// v2's label: `พาสปอร์ต`, `หลักฐานชำระเงิน` or `ใบเสร็จ`.
    pub label: String,
    /// `/files/<key>`, the owner's way in.
    pub url: String,
    /// The file's recorded type; empty when the file cannot be found in the branch.
    pub mime: String,
}

/// What a document is, whatever its label says (design §A D-A5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    Passport,
    PaymentProof,
    Receipt,
}

impl DocumentLink {
    /// The document's kind, from v2's label.
    pub fn kind(&self) -> DocumentKind {
        match self.label.as_str() {
            PASSPORT => DocumentKind::Passport,
            RECEIPT => DocumentKind::Receipt,
            _ => DocumentKind::PaymentProof,
        }
    }

    /// v2's download name: `<ref>_<PASSPORT|PAYMENT|RECEIPT_FULL>.<ext>`, the ending from the recorded type.
    pub fn file_name(&self, document_ref: &str) -> String {
        let tag = match self.kind() {
            DocumentKind::Passport => "PASSPORT",
            DocumentKind::PaymentProof => "PAYMENT",
            DocumentKind::Receipt => "RECEIPT_FULL",
        };
        format!("{document_ref}_{tag}.{}", ending(&self.mime))
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

/// The documents of one exchange, in v2's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Documents {
    pub document_ref: String,
    pub links: Vec<DocumentLink>,
}

impl Documents {
    /// What v2's action `documents` answers: `{"ref": …, "links": [[label, url], …]}`.
    pub fn to_json(&self) -> Value {
        json!({
            "ref": self.document_ref,
            "links": self.links.iter().map(|l| json!([l.label, l.url])).collect::<Vec<_>>(),
        })
    }
}

#[derive(Clone)]
pub struct DocumentsLookup {
    transactions: TransactionService,
    branches: BranchService,
    files: FileService,
}

impl DocumentsLookup {
    pub fn new(
        transactions: TransactionService,
        branches: BranchService,
        files: FileService,
    ) -> Self {
        Self {
            transactions,
            branches,
            files,
        }
    }

    /// The documents of the LIVE exchange `document_ref` of `branch`.
    pub async fn find(&self, branch: &BranchId, document_ref: &str) -> AppResult<Documents> {
        self.branches.require(branch).await?;
        let found = self.transactions.find(branch, document_ref).await?;
        let Some(exchange) = found.filter(|t| t.mode == Mode::Live) else {
            return Err(AppError::Domain(NO_DOCUMENTS.to_owned()));
        };
        let mut keys = vec![(PASSPORT, exchange.passport_key.clone())];
        if let Some(slip) = exchange.slip_key.clone().filter(|k| !k.is_empty()) {
            keys.push((PAYMENT_PROOF, slip));
        }
        keys.push((RECEIPT, exchange.receipt_key.clone()));
        let mut links = Vec::with_capacity(keys.len());
        for (label, key) in keys {
            let mime = self
                .files
                .mime_in_branch(&key, branch)
                .await?
                .unwrap_or_default();
            links.push(DocumentLink {
                label: label.to_owned(),
                url: format!("/files/{key}"),
                mime,
            });
        }
        Ok(Documents {
            document_ref: exchange.document_ref,
            links,
        })
    }
}
