//! `DocumentStore`: the one store for identity documents (verification design §5, owner S-1/S-2).
//!
//! The encryption lives in the implementation; no caller encrypts by itself.

use futures::future::BoxFuture;

use crate::pii::PiiError;

/// Why a document store call failed.
#[derive(Debug, thiserror::Error)]
pub enum DocumentStoreError {
    /// The ref is empty, absolute, or contains `..` or a path separator.
    #[error("invalid document ref: {0}")]
    InvalidRef(String),
    /// No object is stored under the ref.
    #[error("document not found: {0}")]
    NotFound(String),
    /// The object could not be opened: wrong AAD (ref), tampered bytes, or unknown key id.
    #[error("document decrypt failed: {0}")]
    Decrypt(#[from] PiiError),
    /// The storage backend failed.
    #[error("document store i/o: {0}")]
    Io(String),
}

/// Store for identity documents, addressed by an opaque ref.
pub trait DocumentStore: Send + Sync {
    /// Seals and stores `bytes` under `doc_ref`, replacing any previous object.
    /// Returns the `key_id` of the key that sealed it.
    fn put<'a>(
        &'a self,
        doc_ref: &'a str,
        bytes: &'a [u8],
    ) -> BoxFuture<'a, Result<String, DocumentStoreError>>;

    /// Opens the object stored under `doc_ref` and returns its plaintext.
    fn get<'a>(&'a self, doc_ref: &'a str) -> BoxFuture<'a, Result<Vec<u8>, DocumentStoreError>>;

    /// Removes the object stored under `doc_ref`.
    fn delete<'a>(&'a self, doc_ref: &'a str) -> BoxFuture<'a, Result<(), DocumentStoreError>>;
}

/// Builds an opaque ref for a document attached to `parent` (VF-3, VF-12, #1602).
pub fn document_ref(parent: &str, key: &str) -> Result<String, DocumentStoreError> {
    let is_invalid = |s: &str| {
        s.is_empty() || s.starts_with('.') || s.contains("..") || s.contains(['/', '\\', '\0'])
    };
    if is_invalid(parent) {
        return Err(DocumentStoreError::InvalidRef(parent.to_owned()));
    }
    if is_invalid(key) {
        return Err(DocumentStoreError::InvalidRef(key.to_owned()));
    }
    Ok(format!("{parent}_{key}"))
}
