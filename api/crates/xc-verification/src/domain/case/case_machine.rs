//! `CaseMachine` (design §4.2, §4.3): pure transitions. The customer-side steps take a [`CaseActor`] (a customer,
//! or a branch reviewer, V17); the reviewer-state steps take a [`ReviewerActor`], so a customer cannot call them.

use chrono::{DateTime, Utc};

use super::case_actor::CaseActor;
use super::case_error::CaseError;
use super::case_status::CaseStatus;
use super::decision_reason::DecisionReason;
use super::reviewer_actor::ReviewerActor;
use super::verification_case::VerificationCase;
use crate::domain::document::verification_document::VerificationDocument;
use crate::domain::policy::verification_policy::VerificationPolicy;

pub struct CaseMachine;

impl CaseMachine {
    /// `DRAFT` / `NEEDS_UPDATE` → `SUBMITTED` when `documents` satisfy `policy`; else `DocumentsMissing`; any
    /// other state is `CaseLocked`.
    pub fn submit(
        case: &VerificationCase,
        actor: &CaseActor,
        documents: &[VerificationDocument],
        policy: &VerificationPolicy,
        now: DateTime<Utc>,
    ) -> Result<VerificationCase, CaseError> {
        let _ = actor;
        if case.status != CaseStatus::Draft && case.status != CaseStatus::NeedsUpdate {
            return Err(CaseError::CaseLocked);
        }
        if !policy.satisfied_by(documents) {
            return Err(CaseError::DocumentsMissing);
        }
        let mut updated = case.clone();
        updated.status = CaseStatus::Submitted;
        updated.submitted_at = Some(now);
        updated.reviewed_by = None;
        updated.decided_at = None;
        updated.decision_reason = None;
        updated.version = case.version + 1;
        Ok(updated)
    }

    /// VF-4: `SUBMITTED` → `UNDER_REVIEW`.
    pub fn start_review(
        case: &VerificationCase,
        reviewer: &ReviewerActor,
        now: DateTime<Utc>,
    ) -> Result<VerificationCase, CaseError> {
        let _ = (reviewer, now);
        if case.status != CaseStatus::Submitted {
            return Err(CaseError::CaseLocked);
        }
        let mut updated = case.clone();
        updated.status = CaseStatus::UnderReview;
        updated.version = case.version + 1;
        Ok(updated)
    }

    /// VF-4: `UNDER_REVIEW` → `VERIFIED` with the document's expiry (V4).
    pub fn approve(
        case: &VerificationCase,
        reviewer: &ReviewerActor,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<VerificationCase, CaseError> {
        if case.status != CaseStatus::UnderReview {
            return Err(CaseError::CaseLocked);
        }
        let mut updated = case.clone();
        updated.status = CaseStatus::Verified;
        updated.reviewed_by = Some(reviewer.user_id);
        updated.decided_at = Some(now);
        updated.expires_at = Some(expires_at);
        updated.decision_reason = None;
        updated.version = case.version + 1;
        Ok(updated)
    }

    /// VF-4: `UNDER_REVIEW` → `REJECTED`.
    pub fn reject(
        case: &VerificationCase,
        reviewer: &ReviewerActor,
        reason: DecisionReason,
        now: DateTime<Utc>,
    ) -> Result<VerificationCase, CaseError> {
        if case.status != CaseStatus::UnderReview {
            return Err(CaseError::CaseLocked);
        }
        let mut updated = case.clone();
        updated.status = CaseStatus::Rejected;
        updated.reviewed_by = Some(reviewer.user_id);
        updated.decided_at = Some(now);
        updated.decision_reason = Some(reason);
        updated.version = case.version + 1;
        Ok(updated)
    }

    /// VF-4: `UNDER_REVIEW` → `NEEDS_UPDATE`.
    pub fn request_update(
        case: &VerificationCase,
        reviewer: &ReviewerActor,
        reason: DecisionReason,
        now: DateTime<Utc>,
    ) -> Result<VerificationCase, CaseError> {
        if case.status != CaseStatus::UnderReview {
            return Err(CaseError::CaseLocked);
        }
        let mut updated = case.clone();
        updated.status = CaseStatus::NeedsUpdate;
        updated.reviewed_by = Some(reviewer.user_id);
        updated.decided_at = Some(now);
        updated.decision_reason = Some(reason);
        updated.version = case.version + 1;
        Ok(updated)
    }

    /// VF-4: `VERIFIED` → `REVOKED`.
    pub fn revoke(
        case: &VerificationCase,
        reviewer: &ReviewerActor,
        reason: DecisionReason,
        now: DateTime<Utc>,
    ) -> Result<VerificationCase, CaseError> {
        if case.status != CaseStatus::Verified {
            return Err(CaseError::CaseLocked);
        }
        let mut updated = case.clone();
        updated.status = CaseStatus::Revoked;
        updated.reviewed_by = Some(reviewer.user_id);
        updated.decided_at = Some(now);
        updated.decision_reason = Some(reason);
        updated.version = case.version + 1;
        Ok(updated)
    }

    /// VF-6: `VERIFIED` past `expires_at` → `EXPIRED` (the only timed move, V4).
    pub fn expire(
        case: &VerificationCase,
        now: DateTime<Utc>,
    ) -> Result<VerificationCase, CaseError> {
        if case.status != CaseStatus::Verified {
            return Err(CaseError::CaseLocked);
        }
        match case.expires_at {
            Some(t) if now >= t => {
                let mut updated = case.clone();
                updated.status = CaseStatus::Expired;
                updated.version = case.version + 1;
                Ok(updated)
            }
            _ => Err(CaseError::CaseLocked),
        }
    }
}
