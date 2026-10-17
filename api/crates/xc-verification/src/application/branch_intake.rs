//! `BranchIntake` (design V17, §4.3, §5, ticket #1398 VF-12): at-branch intake of a walk-in customer.
//! A VERIFICATION_REVIEWER scoped to a branch opens a case, stores the document through `DocumentStore`,
//! submits through `CaseMachine`, and appends one `audit_events` row in the same transaction.
//!
//! Up-front scaffolding (VF-12 #1398): signatures with `unimplemented!()` bodies. The implementer fills the bodies;
//! the provided tests in `tests/branch_intake_test.rs` and `api/tests/verification_branch_intake_test.rs` pin them.

use std::sync::Arc;

use chrono::Utc;
use pos_error::AppError;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_kernel::document_store::{DocumentStore, DocumentStoreError};
use xc_kernel::pii::identity_type::IdentityType;

use crate::domain::case::case_actor::CaseActor;
use crate::domain::case::case_error::CaseError;
use crate::domain::case::case_machine::CaseMachine;
use crate::domain::case::case_status::CaseStatus;
use crate::domain::case::provider::Provider;
use crate::domain::case::reviewer_actor::{ReviewerActor, ReviewerScope};
use crate::domain::case::verification_case::VerificationCase;
use crate::domain::case_repository::CaseRepository;
use crate::domain::document::access_class::AccessClass;
use crate::domain::document::verification_document::VerificationDocument;

/// Document payload for branch intake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchDocument {
    pub document_type: IdentityType,
    pub bytes: Vec<u8>,
    pub idempotency_key: String,
}

impl BranchDocument {
    pub fn new(
        document_type: IdentityType,
        bytes: Vec<u8>,
        idempotency_key: impl Into<String>,
    ) -> Self {
        Self {
            document_type,
            bytes,
            idempotency_key: idempotency_key.into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BranchIntakeError {
    /// Policy does not allow review at the branch (V17).
    #[error("branchReviewNotAllowed")]
    BranchReviewNotAllowed,
    /// Another customer, another organisation, reviewer linked to customer, or reviewer not branch-scoped (§4.3).
    #[error("outOfScope")]
    OutOfScope,
    /// The case's policy version does not accept this document type (V11, V16).
    #[error("documentTypeNotAccepted")]
    DocumentTypeNotAccepted,
    /// The organisation has no published policy version (V11).
    #[error("noVerificationPolicy")]
    NoVerificationPolicy,
    /// Documents change or open only when allowed; case is locked or customer already has open case (V14).
    #[error("caseLocked")]
    CaseLocked,
    /// The same `Idempotency-Key` with another body (Blueprint §09, V14).
    #[error("idempotencyConflict")]
    IdempotencyConflict,
    /// The upload refused with one of `file_service.rs`'s sentences (size, type, content).
    #[error("{0}")]
    File(&'static str),
    /// No such entity.
    #[error("notFound")]
    NotFound,
    #[error(transparent)]
    Case(#[from] CaseError),
    #[error(transparent)]
    Store(#[from] AppError),
    #[error(transparent)]
    Documents(#[from] DocumentStoreError),
}

impl BranchIntakeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::BranchReviewNotAllowed => "branchReviewNotAllowed",
            Self::OutOfScope => "outOfScope",
            Self::DocumentTypeNotAccepted => "documentTypeNotAccepted",
            Self::NoVerificationPolicy => "noVerificationPolicy",
            Self::CaseLocked => "caseLocked",
            Self::IdempotencyConflict => "idempotencyConflict",
            Self::File(_) => "validationFailed",
            Self::NotFound => "notFound",
            Self::Case(e) => e.code(),
            Self::Store(_) | Self::Documents(_) => "serverError",
        }
    }
}

pub struct BranchIntake {
    pool: PgPool,
    cases: Arc<dyn CaseRepository>,
    store: Arc<dyn DocumentStore>,
    audit: Arc<dyn AuditLog>,
}

impl BranchIntake {
    pub fn new(
        pool: PgPool,
        cases: Arc<dyn CaseRepository>,
        store: Arc<dyn DocumentStore>,
        audit: Arc<dyn AuditLog>,
    ) -> Self {
        Self {
            pool,
            cases,
            store,
            audit,
        }
    }

    /// Opens, attaches document, and submits in one transaction for a walk-in customer (V17).
    pub async fn open(
        &self,
        reviewer: &ReviewerActor,
        customer_id: &str,
        document: BranchDocument,
    ) -> Result<(VerificationCase, VerificationDocument), BranchIntakeError> {
        let (rev_org_id, rev_branch_id) = match &reviewer.scope {
            ReviewerScope::Branch {
                organization_id,
                branch_id,
            } => (*organization_id, branch_id.clone()),
            ReviewerScope::Organization { .. } => return Err(BranchIntakeError::OutOfScope),
        };

        const MAX_UPLOAD_BYTES: usize = 15 * 1024 * 1024;
        const TOO_LARGE: &str = "ไฟล์ใหญ่เกิน 15 MB";
        const UNSUPPORTED: &str = "ชนิดไฟล์ไม่รองรับ";

        if document.bytes.is_empty() || document.bytes.len() > MAX_UPLOAD_BYTES {
            return Err(BranchIntakeError::File(TOO_LARGE));
        }

        let mime = if document.bytes.len() >= 8
            && document.bytes[0..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        {
            "image/png"
        } else if document.bytes.len() >= 3 && document.bytes[0..3] == [0xFF, 0xD8, 0xFF] {
            "image/jpeg"
        } else if document.bytes.len() >= 5 && &document.bytes[0..5] == b"%PDF-" {
            "application/pdf"
        } else {
            return Err(BranchIntakeError::File(UNSUPPORTED));
        };

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| BranchIntakeError::Store(AppError::Database(e)))?;

        let (cust_org_id, cust_user_id) = self
            .cases
            .find_customer(&mut tx, customer_id)
            .await
            .map_err(BranchIntakeError::Case)?
            .ok_or(BranchIntakeError::NotFound)?;

        if cust_org_id != rev_org_id {
            return Err(BranchIntakeError::OutOfScope);
        }

        if cust_user_id == Some(reviewer.user_id) {
            return Err(BranchIntakeError::OutOfScope);
        }

        let policy = self
            .cases
            .find_latest_policy(&mut tx, cust_org_id)
            .await
            .map_err(BranchIntakeError::Case)?
            .ok_or(BranchIntakeError::NoVerificationPolicy)?;

        if !policy.branch_review_allowed {
            return Err(BranchIntakeError::BranchReviewNotAllowed);
        }

        if !policy
            .accepted_document_types
            .contains(&document.document_type)
        {
            return Err(BranchIntakeError::DocumentTypeNotAccepted);
        }

        let open_case = self
            .cases
            .find_open_case_by_customer(&mut tx, customer_id)
            .await
            .map_err(BranchIntakeError::Case)?;
        // Re-read the open case under its row lock (#2876): a concurrent intake at another branch
        // waits here, then sees the case already SUBMITTED and filed at that branch, and answers
        // `CaseLocked` below instead of overwriting its branch.
        let open_case = match open_case {
            Some(c) => self
                .cases
                .find_case_by_id_for_update(&mut tx, c.id)
                .await
                .map_err(BranchIntakeError::Case)?,
            None => None,
        };

        let mut case_id_hasher = Sha256::new();
        case_id_hasher
            .update(format!("{cust_org_id}:{customer_id}:{}", document.idempotency_key).as_bytes());
        let hash = case_id_hasher.finalize();
        let mut uuid_bytes = [0u8; 16];
        uuid_bytes.copy_from_slice(&hash[0..16]);
        uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x40;
        uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80;
        let derived_case_id = Uuid::from_bytes(uuid_bytes);

        let (target_case_id, is_new_case) = match &open_case {
            Some(c) => (c.id, false),
            None => (derived_case_id, true),
        };

        let doc_ref = xc_kernel::document_store::document_ref(
            &target_case_id.to_string(),
            &document.idempotency_key,
        )?;
        let mut hasher = Sha256::new();
        hasher.update(&document.bytes);
        let checksum = hasher.finalize().to_vec();

        // Check if this document ref was already recorded in DB (idempotent replay)
        if let Some(existing_doc) = self
            .cases
            .find_document_by_ref(&mut tx, &doc_ref)
            .await
            .map_err(BranchIntakeError::Case)?
        {
            if existing_doc.checksum == checksum
                && existing_doc.document_type == document.document_type
            {
                let case = self
                    .cases
                    .find_case_by_id_for_update(&mut tx, existing_doc.case_id)
                    .await
                    .map_err(BranchIntakeError::Case)?
                    .ok_or(BranchIntakeError::NotFound)?;
                tx.commit()
                    .await
                    .map_err(|e| BranchIntakeError::Store(AppError::Database(e)))?;
                return Ok((case, existing_doc));
            } else {
                return Err(BranchIntakeError::IdempotencyConflict);
            }
        }

        let case_to_submit = if let Some(mut existing) = open_case {
            if !matches!(existing.status, CaseStatus::Draft | CaseStatus::NeedsUpdate)
                || existing
                    .branch_id
                    .as_ref()
                    .is_some_and(|b| b != &rev_branch_id)
            {
                return Err(BranchIntakeError::CaseLocked);
            }
            existing.branch_id = Some(rev_branch_id.clone());
            existing
        } else {
            VerificationCase {
                id: derived_case_id,
                organization_id: cust_org_id,
                customer_id: customer_id.to_string(),
                provider: Provider::Staff,
                status: CaseStatus::Draft,
                policy_version: policy.version,
                branch_id: Some(rev_branch_id.clone()),
                submitted_at: None,
                decided_at: None,
                expires_at: None,
                reviewed_by: None,
                decision_reason: None,
                version: 0,
            }
        };

        // Write to DocumentStore
        let key_id = self
            .store
            .put(&doc_ref, &document.bytes)
            .await
            .map_err(BranchIntakeError::Documents)?;

        let now = Utc::now();
        let doc = VerificationDocument {
            id: Uuid::new_v4(),
            case_id: case_to_submit.id,
            document_type: document.document_type,
            encrypted_object_ref: Some(doc_ref),
            key_id: Some(key_id),
            checksum,
            mime: mime.to_string(),
            size_bytes: document.bytes.len() as i32,
            uploaded_at: now,
            uploaded_by: reviewer.user_id,
            retention_until: None,
            access_class: AccessClass::IdentityDocument,
            removed_at: None,
        };

        let actor = CaseActor::Reviewer(reviewer.clone());
        let mut active_docs = if is_new_case {
            vec![]
        } else {
            self.cases
                .list_active_documents_by_case(&mut tx, case_to_submit.id)
                .await
                .map_err(BranchIntakeError::Case)?
        };
        if !active_docs.iter().any(|d| d.id == doc.id) {
            active_docs.push(doc.clone());
        }

        let submitted_case =
            CaseMachine::submit(&case_to_submit, &actor, &active_docs, &policy, now)
                .map_err(BranchIntakeError::Case)?;

        let final_case = if is_new_case {
            let inserted = self
                .cases
                .insert_case(&mut tx, &submitted_case)
                .await
                .map_err(BranchIntakeError::Case)?
                .ok_or(BranchIntakeError::CaseLocked)?;
            self.cases
                .insert_document(&mut tx, cust_org_id, &doc)
                .await
                .map_err(BranchIntakeError::Case)?;
            inserted
        } else {
            self.cases
                .insert_document(&mut tx, cust_org_id, &doc)
                .await
                .map_err(BranchIntakeError::Case)?;
            self.cases
                .update_case_submitted_with_branch(
                    &mut tx,
                    submitted_case.id,
                    now,
                    submitted_case.version,
                    submitted_case.branch_id.as_ref(),
                )
                .await
                .map_err(BranchIntakeError::Case)?;
            submitted_case
        };

        let event = AuditEvent {
            actor_id: reviewer.user_id.to_string(),
            actor_role: "VERIFICATION_REVIEWER".to_string(),
            scope: cust_org_id.to_string(),
            action: "verificationBranchCaseOpen".to_string(),
            object_ref: final_case.id.to_string(),
            branch_id: Some(rev_branch_id.as_str().to_string()),
            before: None,
            after: Some(serde_json::json!({
                "case_id": final_case.id.to_string(),
                "customer_id": customer_id,
                "document_id": doc.id.to_string(),
                "document_type": document.document_type.as_str(),
                "status": "SUBMITTED",
            })),
        };
        self.audit
            .append(&mut tx, &event)
            .await
            .map_err(BranchIntakeError::Store)?;

        tx.commit()
            .await
            .map_err(|e| BranchIntakeError::Store(AppError::Database(e)))?;

        Ok((final_case, doc))
    }
}
