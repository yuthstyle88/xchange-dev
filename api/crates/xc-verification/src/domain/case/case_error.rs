//! `CaseError`: the refusals of design §4.3 for the customer side, and the store's own failures.

use pos_error::AppError;
use xc_kernel::document_store::DocumentStoreError;

#[derive(Debug, thiserror::Error)]
pub enum CaseError {
    /// The organisation has no published policy version (V11).
    #[error("noVerificationPolicy")]
    NoVerificationPolicy,
    /// Documents change only while `DRAFT` / `NEEDS_UPDATE` (V3).
    #[error("caseLocked")]
    CaseLocked,
    /// The case's policy version does not accept this document type (V11, V16).
    #[error("documentTypeNotAccepted")]
    DocumentTypeNotAccepted,
    /// The documents do not satisfy the case's policy version (V11).
    #[error("documentsMissing")]
    DocumentsMissing,
    /// Another customer's case, another organisation, or a state the actor may not reach (§4.3).
    #[error("outOfScope")]
    OutOfScope,
    /// The same `Idempotency-Key` with another body (Blueprint §09, V14).
    #[error("idempotencyConflict")]
    IdempotencyConflict,
    /// The upload refused with one of `file_service.rs`'s sentences (size, type, content).
    #[error("{0}")]
    File(&'static str),
    /// No such case or document.
    #[error("notFound")]
    NotFound,
    #[error(transparent)]
    Store(#[from] AppError),
    #[error(transparent)]
    Documents(#[from] DocumentStoreError),
}

impl CaseError {
    /// The v3 sentence key: exactly the `#[error]` text above for the refusals; `validationFailed` for a `File`
    /// refusal, `serverError` for a store failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoVerificationPolicy => "noVerificationPolicy",
            Self::CaseLocked => "caseLocked",
            Self::DocumentTypeNotAccepted => "documentTypeNotAccepted",
            Self::DocumentsMissing => "documentsMissing",
            Self::OutOfScope => "outOfScope",
            Self::IdempotencyConflict => "idempotencyConflict",
            Self::File(_) => "validationFailed",
            Self::NotFound => "notFound",
            Self::Store(_) => "serverError",
            Self::Documents(_) => "serverError",
        }
    }
}
