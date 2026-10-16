//! `CaseService` (design §5, §4.3, §4.4 C10): open, add/remove a document, submit. Every write appends one
//! `audit_events` row through `AuditLog` in the same transaction, with no bytes and no identity value (V6).

use std::sync::Arc;

use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_kernel::document_store::DocumentStore;
use xc_kernel::pii::identity_type::IdentityType;

use crate::domain::case::case_actor::CaseActor;
use crate::domain::case::case_error::CaseError;
use crate::domain::case::case_machine::CaseMachine;
use crate::domain::case::case_status::CaseStatus;
use crate::domain::case::provider::Provider;
use crate::domain::case::reviewer_actor::ReviewerScope;
use crate::domain::case::verification_case::VerificationCase;
use crate::domain::case_repository::CaseRepository;
use crate::domain::document::access_class::AccessClass;
use crate::domain::document::verification_document::VerificationDocument;

const MAX_UPLOAD_BYTES: usize = 15 * 1024 * 1024;
const TOO_LARGE: &str = "ไฟล์ใหญ่เกิน 15 MB";
const UNSUPPORTED: &str = "ชนิดไฟล์ไม่รองรับ";

pub struct CaseService {
    pool: PgPool,
    cases: Arc<dyn CaseRepository>,
    store: Arc<dyn DocumentStore>,
    audit: Arc<dyn AuditLog>,
}

impl CaseService {
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

    /// The customer's open case, or a new `DRAFT` under the organisation's newest policy version;
    /// `NoVerificationPolicy` when it has none. Another customer or organisation: `OutOfScope`.
    pub async fn open(
        &self,
        actor: &CaseActor,
        customer_id: &str,
    ) -> Result<VerificationCase, CaseError> {
        if let CaseActor::Customer(c) = actor {
            if c.customer_id != customer_id {
                return Err(CaseError::OutOfScope);
            }
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

        let (org_id, _user_id) = self
            .cases
            .find_customer(&mut tx, customer_id)
            .await?
            .ok_or(CaseError::OutOfScope)?;

        if let CaseActor::Reviewer(r) = actor {
            let rev_org = match &r.scope {
                ReviewerScope::Organization { organization_id } => *organization_id,
                ReviewerScope::Branch {
                    organization_id, ..
                } => *organization_id,
            };
            if rev_org != org_id {
                return Err(CaseError::OutOfScope);
            }
        }

        let policy = self
            .cases
            .find_latest_policy(&mut tx, org_id)
            .await?
            .ok_or(CaseError::NoVerificationPolicy)?;

        if let Some(existing) = self
            .cases
            .find_open_case_by_customer(&mut tx, customer_id)
            .await?
        {
            tx.commit()
                .await
                .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;
            return Ok(existing);
        }

        let branch_id = match actor {
            CaseActor::Reviewer(r) => match &r.scope {
                ReviewerScope::Branch { branch_id, .. } => Some(branch_id.clone()),
                _ => None,
            },
            _ => None,
        };

        let new_case = VerificationCase {
            id: Uuid::new_v4(),
            organization_id: org_id,
            customer_id: customer_id.to_string(),
            provider: Provider::Staff,
            status: CaseStatus::Draft,
            policy_version: policy.version,
            branch_id,
            submitted_at: None,
            decided_at: None,
            expires_at: None,
            reviewed_by: None,
            decision_reason: None,
            version: 1,
        };

        let inserted = self.cases.insert_case(&mut tx, &new_case).await?;
        match inserted {
            Some(case) => {
                let (actor_id, actor_role, branch_str) = match actor {
                    CaseActor::Customer(c) => (c.customer_id.clone(), "CUSTOMER".to_string(), None),
                    CaseActor::Reviewer(r) => {
                        let b = match &r.scope {
                            ReviewerScope::Branch { branch_id, .. } => {
                                Some(branch_id.as_str().to_string())
                            }
                            _ => None,
                        };
                        (
                            r.user_id.to_string(),
                            "VERIFICATION_REVIEWER".to_string(),
                            b,
                        )
                    }
                };
                let event = AuditEvent {
                    actor_id,
                    actor_role,
                    scope: org_id.to_string(),
                    action: "verification_case.open".to_string(),
                    object_ref: case.id.to_string(),
                    branch_id: branch_str,
                    before: None,
                    after: Some(serde_json::json!({
                        "status": "DRAFT",
                        "policy_version": case.policy_version,
                    })),
                };
                self.audit
                    .append(&mut tx, &event)
                    .await
                    .map_err(CaseError::Store)?;
                tx.commit()
                    .await
                    .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;
                Ok(case)
            }
            None => {
                let existing = self
                    .cases
                    .find_open_case_by_customer(&mut tx, customer_id)
                    .await?
                    .ok_or(CaseError::NotFound)?;
                tx.commit()
                    .await
                    .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;
                Ok(existing)
            }
        }
    }

    /// Judges the bytes by content (JPEG, PNG, PDF; ≤ 15 MB, `file_service.rs`'s sentences), stores them in
    /// `DocumentStore` under a ref keyed by (case, `idempotency_key`), and records the metadata. A retry with the
    /// same key and bytes answers the same document and writes nothing new; with other bytes, `IdempotencyConflict`.
    pub async fn add_document(
        &self,
        actor: &CaseActor,
        case_id: Uuid,
        document_type: IdentityType,
        bytes: &[u8],
        idempotency_key: &str,
    ) -> Result<VerificationDocument, CaseError> {
        if bytes.is_empty() || bytes.len() > MAX_UPLOAD_BYTES {
            return Err(CaseError::File(TOO_LARGE));
        }

        let mime = if bytes.len() >= 8
            && bytes[0..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        {
            "image/png"
        } else if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
            "image/jpeg"
        } else if bytes.len() >= 5 && &bytes[0..5] == b"%PDF-" {
            "application/pdf"
        } else {
            return Err(CaseError::File(UNSUPPORTED));
        };

        let doc_ref =
            xc_kernel::document_store::document_ref(&case_id.to_string(), idempotency_key)?;
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let checksum = hasher.finalize().to_vec();

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

        if let Some(existing) = self.cases.find_document_by_ref(&mut tx, &doc_ref).await? {
            if existing.checksum == checksum {
                tx.commit()
                    .await
                    .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;
                return Ok(existing);
            } else {
                return Err(CaseError::IdempotencyConflict);
            }
        }

        let case = self
            .cases
            .find_case_by_id_for_update(&mut tx, case_id)
            .await?
            .ok_or(CaseError::NotFound)?;

        let uploaded_by = match actor {
            CaseActor::Customer(c) => {
                if c.customer_id != case.customer_id {
                    return Err(CaseError::OutOfScope);
                }
                let (_org_id, user_id) = self
                    .cases
                    .find_customer(&mut tx, &case.customer_id)
                    .await?
                    .ok_or(CaseError::NotFound)?;
                user_id.ok_or(CaseError::OutOfScope)?
            }
            CaseActor::Reviewer(r) => {
                let rev_org = match &r.scope {
                    ReviewerScope::Organization { organization_id } => *organization_id,
                    ReviewerScope::Branch {
                        organization_id, ..
                    } => *organization_id,
                };
                if rev_org != case.organization_id {
                    return Err(CaseError::OutOfScope);
                }
                r.user_id
            }
        };

        if case.status != CaseStatus::Draft && case.status != CaseStatus::NeedsUpdate {
            return Err(CaseError::CaseLocked);
        }

        let policy = self
            .cases
            .find_policy(&mut tx, case.organization_id, case.policy_version)
            .await?
            .ok_or(CaseError::NoVerificationPolicy)?;

        if !policy.accepted_document_types.contains(&document_type) {
            return Err(CaseError::DocumentTypeNotAccepted);
        }

        let key_id = self
            .store
            .put(&doc_ref, bytes)
            .await
            .map_err(CaseError::Documents)?;

        let now = Utc::now();
        let doc = VerificationDocument {
            id: Uuid::new_v4(),
            case_id,
            document_type,
            encrypted_object_ref: Some(doc_ref),
            key_id: Some(key_id),
            checksum,
            mime: mime.to_string(),
            size_bytes: bytes.len() as i32,
            uploaded_at: now,
            uploaded_by,
            retention_until: None,
            access_class: AccessClass::IdentityDocument,
            removed_at: None,
        };

        self.cases
            .insert_document(&mut tx, case.organization_id, &doc)
            .await?;

        let (actor_id, actor_role, branch_str) = match actor {
            CaseActor::Customer(c) => (c.customer_id.clone(), "CUSTOMER".to_string(), None),
            CaseActor::Reviewer(r) => {
                let b = match &r.scope {
                    ReviewerScope::Branch { branch_id, .. } => Some(branch_id.as_str().to_string()),
                    _ => None,
                };
                (
                    r.user_id.to_string(),
                    "VERIFICATION_REVIEWER".to_string(),
                    b,
                )
            }
        };

        let event = AuditEvent {
            actor_id,
            actor_role,
            scope: case.organization_id.to_string(),
            action: "verification_document.add".to_string(),
            object_ref: case.id.to_string(),
            branch_id: branch_str,
            before: None,
            after: Some(serde_json::json!({
                "document_id": doc.id.to_string(),
                "document_type": document_type.as_str(),
            })),
        };
        self.audit
            .append(&mut tx, &event)
            .await
            .map_err(CaseError::Store)?;

        tx.commit()
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

        Ok(doc)
    }

    /// Marks the document removed (its ref becomes `NULL`) while the case is `DRAFT` / `NEEDS_UPDATE`.
    pub async fn remove_document(
        &self,
        actor: &CaseActor,
        document_id: Uuid,
    ) -> Result<(), CaseError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

        let doc = self
            .cases
            .find_document_by_id(&mut tx, document_id)
            .await?
            .ok_or(CaseError::NotFound)?;

        if doc.removed_at.is_some() {
            return Err(CaseError::NotFound);
        }

        let case = self
            .cases
            .find_case_by_id_for_update(&mut tx, doc.case_id)
            .await?
            .ok_or(CaseError::NotFound)?;

        match actor {
            CaseActor::Customer(c) => {
                if c.customer_id != case.customer_id {
                    return Err(CaseError::OutOfScope);
                }
            }
            CaseActor::Reviewer(r) => {
                let rev_org = match &r.scope {
                    ReviewerScope::Organization { organization_id } => *organization_id,
                    ReviewerScope::Branch {
                        organization_id, ..
                    } => *organization_id,
                };
                if rev_org != case.organization_id {
                    return Err(CaseError::OutOfScope);
                }
            }
        }

        if case.status != CaseStatus::Draft && case.status != CaseStatus::NeedsUpdate {
            return Err(CaseError::CaseLocked);
        }

        if let Some(ref_str) = &doc.encrypted_object_ref {
            let _ = self.store.delete(ref_str).await;
        }

        let now = Utc::now();
        self.cases
            .remove_document(&mut tx, document_id, now)
            .await?;

        let (actor_id, actor_role, branch_str) = match actor {
            CaseActor::Customer(c) => (c.customer_id.clone(), "CUSTOMER".to_string(), None),
            CaseActor::Reviewer(r) => {
                let b = match &r.scope {
                    ReviewerScope::Branch { branch_id, .. } => Some(branch_id.as_str().to_string()),
                    _ => None,
                };
                (
                    r.user_id.to_string(),
                    "VERIFICATION_REVIEWER".to_string(),
                    b,
                )
            }
        };

        let event = AuditEvent {
            actor_id,
            actor_role,
            scope: case.organization_id.to_string(),
            action: "verification_document.remove".to_string(),
            object_ref: case.id.to_string(),
            branch_id: branch_str,
            before: None,
            after: Some(serde_json::json!({
                "document_id": document_id.to_string(),
            })),
        };
        self.audit
            .append(&mut tx, &event)
            .await
            .map_err(CaseError::Store)?;

        tx.commit()
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

        Ok(())
    }

    /// `CaseMachine::submit` on the case and its documents, under the case's own policy version.
    pub async fn submit(
        &self,
        actor: &CaseActor,
        case_id: Uuid,
    ) -> Result<VerificationCase, CaseError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

        let case = self
            .cases
            .find_case_by_id_for_update(&mut tx, case_id)
            .await?
            .ok_or(CaseError::NotFound)?;

        match actor {
            CaseActor::Customer(c) => {
                if c.customer_id != case.customer_id {
                    return Err(CaseError::OutOfScope);
                }
            }
            CaseActor::Reviewer(r) => {
                let rev_org = match &r.scope {
                    ReviewerScope::Organization { organization_id } => *organization_id,
                    ReviewerScope::Branch {
                        organization_id, ..
                    } => *organization_id,
                };
                if rev_org != case.organization_id {
                    return Err(CaseError::OutOfScope);
                }
            }
        }

        let docs = self
            .cases
            .list_active_documents_by_case(&mut tx, case_id)
            .await?;

        let policy = self
            .cases
            .find_policy(&mut tx, case.organization_id, case.policy_version)
            .await?
            .ok_or(CaseError::NoVerificationPolicy)?;

        let now = Utc::now();
        let submitted_case = CaseMachine::submit(&case, actor, &docs, &policy, now)?;

        self.cases
            .update_case_submitted(&mut tx, case_id, now, submitted_case.version)
            .await?;

        let (actor_id, actor_role, branch_str) = match actor {
            CaseActor::Customer(c) => (c.customer_id.clone(), "CUSTOMER".to_string(), None),
            CaseActor::Reviewer(r) => {
                let b = match &r.scope {
                    ReviewerScope::Branch { branch_id, .. } => Some(branch_id.as_str().to_string()),
                    _ => None,
                };
                (
                    r.user_id.to_string(),
                    "VERIFICATION_REVIEWER".to_string(),
                    b,
                )
            }
        };

        let event = AuditEvent {
            actor_id,
            actor_role,
            scope: case.organization_id.to_string(),
            action: "verification_case.submit".to_string(),
            object_ref: case.id.to_string(),
            branch_id: branch_str,
            before: Some(serde_json::json!({
                "status": case.status.as_str(),
            })),
            after: Some(serde_json::json!({
                "status": "SUBMITTED",
            })),
        };
        self.audit
            .append(&mut tx, &event)
            .await
            .map_err(CaseError::Store)?;

        tx.commit()
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

        Ok(submitted_case)
    }
}
