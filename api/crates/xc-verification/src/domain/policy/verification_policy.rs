//! `VerificationPolicy` (design V11, `verification_policies`): a published version, never edited.

use uuid::Uuid;
use xc_kernel::pii::identity_type::IdentityType;

use crate::domain::document::verification_document::VerificationDocument;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationPolicy {
    pub organization_id: Uuid,
    pub version: i32,
    pub accepted_document_types: Vec<IdentityType>,
    pub documents_required: i32,
    pub branch_review_allowed: bool,
}

impl VerificationPolicy {
    /// At least `documents_required` documents of an accepted type that are neither removed nor purged.
    pub fn satisfied_by(&self, documents: &[VerificationDocument]) -> bool {
        let count = documents
            .iter()
            .filter(|d| {
                d.removed_at.is_none() && self.accepted_document_types.contains(&d.document_type)
            })
            .count();
        count >= self.documents_required as usize
    }

    /// OQ-5, V17.
    pub fn branch_review_allowed(&self) -> bool {
        self.branch_review_allowed
    }
}
