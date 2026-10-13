//! `VerificationDocument` (design V9, V16, §4.1 `verification_documents`): metadata, never bytes.

use chrono::{DateTime, Utc};
use uuid::Uuid;
use xc_kernel::pii::identity_type::IdentityType;

use super::access_class::AccessClass;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationDocument {
    pub id: Uuid,
    pub case_id: Uuid,
    /// `PASSPORT` or `NATIONAL_ID`, per the case's policy (V16).
    pub document_type: IdentityType,
    /// The store's ref; `None` once removed or purged.
    pub encrypted_object_ref: Option<String>,
    pub key_id: Option<String>,
    /// SHA-256 of the uploaded bytes.
    pub checksum: Vec<u8>,
    /// Judged by content: `image/jpeg`, `image/png` or `application/pdf`.
    pub mime: String,
    pub size_bytes: i32,
    pub uploaded_at: DateTime<Utc>,
    pub uploaded_by: Uuid,
    pub retention_until: Option<DateTime<Utc>>,
    pub access_class: AccessClass,
    pub removed_at: Option<DateTime<Utc>>,
}
