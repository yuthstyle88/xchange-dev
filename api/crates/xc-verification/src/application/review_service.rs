//! `ReviewService` (design §5, §4.3, §4.5 F02; ticket #1391): the reviewer side of a verification case.
//! Handles `start_review`, `approve`, `reject`, `request_update`, and `revoke`.
//!
//! Enforces:
//! - Optimistic concurrency via `expected_version` (design §10 `STALE_VERSION`) -> `ReviewError::StaleVersion`.
//! - Scope validation: reviewer organisation must match case organisation, branch-scoped reviewers
//!   only act on their branch (V17), and self-review (reviewer is customer) is forbidden -> `ReviewError::OutOfScope`.
//! - State machine transitions (design §4.2, §4.3) via `CaseMachine`.
//! - Reason code validation (design §12 S-5) for `reject`, `request_update`, and `revoke`.
//! - Expiry date validation: `approve` requires `expires_at` strictly after today in organisation timezone -> `ReviewError::ValidationFailed`.
//! - Every mutation writes one `audit_events` row in the same transaction.

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::AppError;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};

use crate::domain::case::case_error::CaseError;
use crate::domain::case::case_machine::CaseMachine;
use crate::domain::case::case_status::CaseStatus;
use crate::domain::case::decision_reason::DecisionReason;
use crate::domain::case::reviewer_actor::{ReviewerActor, ReviewerScope};
use crate::domain::case::verification_case::VerificationCase;
use crate::domain::case_repository::CaseRepository;

#[derive(Debug, thiserror::Error)]
pub enum ReviewError {
    /// `expectedVersion` does not match the case's current version (design §10 `STALE_VERSION`).
    #[error("staleVersion")]
    StaleVersion,
    /// Another customer, another organisation, another branch (V17), self-review, or wrong state (design §4.3, §10 `OUT_OF_SCOPE`).
    #[error("outOfScope")]
    OutOfScope,
    /// Invalid input: approve with a date not after today, or unlisted reason (design §4.3).
    #[error("validationFailed")]
    ValidationFailed,
    /// The requested verification case was not found.
    #[error("notFound")]
    NotFound,
    #[error(transparent)]
    Store(#[from] AppError),
}

impl ReviewError {
    /// The v3 sentence key for this refusal.
    pub fn code(&self) -> &'static str {
        match self {
            Self::StaleVersion => "staleVersion",
            Self::OutOfScope => "outOfScope",
            Self::ValidationFailed => "validationFailed",
            Self::NotFound => "notFound",
            Self::Store(_) => "serverError",
        }
    }
}

impl PartialEq for ReviewError {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::StaleVersion, Self::StaleVersion)
                | (Self::OutOfScope, Self::OutOfScope)
                | (Self::ValidationFailed, Self::ValidationFailed)
                | (Self::NotFound, Self::NotFound)
        )
    }
}

impl Eq for ReviewError {}

impl From<sqlx::Error> for ReviewError {
    fn from(err: sqlx::Error) -> Self {
        Self::Store(AppError::Database(err))
    }
}

impl From<CaseError> for ReviewError {
    fn from(err: CaseError) -> Self {
        match err {
            CaseError::OutOfScope | CaseError::CaseLocked => Self::OutOfScope,
            CaseError::NotFound => Self::NotFound,
            CaseError::Store(e) => Self::Store(e),
            _ => Self::ValidationFailed,
        }
    }
}

pub struct ReviewService {
    pool: PgPool,
    cases: Arc<dyn CaseRepository>,
    audit: Arc<dyn AuditLog>,
}

impl ReviewService {
    pub fn new(pool: PgPool, cases: Arc<dyn CaseRepository>, audit: Arc<dyn AuditLog>) -> Self {
        Self { pool, cases, audit }
    }

    /// The pool this service writes through.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    async fn check_reviewer_scope(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        reviewer: &ReviewerActor,
        case: &VerificationCase,
    ) -> Result<(), ReviewError> {
        let rev_org = match &reviewer.scope {
            ReviewerScope::Organization { organization_id } => *organization_id,
            ReviewerScope::Branch {
                organization_id, ..
            } => *organization_id,
        };
        if rev_org != case.organization_id {
            return Err(ReviewError::OutOfScope);
        }

        if let ReviewerScope::Branch { branch_id, .. } = &reviewer.scope {
            if case.branch_id.as_ref() != Some(branch_id) {
                return Err(ReviewError::OutOfScope);
            }
        }

        let (_, customer_user_id) = self
            .cases
            .find_customer(tx, &case.customer_id)
            .await?
            .ok_or(ReviewError::OutOfScope)?;

        if customer_user_id == Some(reviewer.user_id) {
            return Err(ReviewError::OutOfScope);
        }

        Ok(())
    }

    fn audit_branch_id(reviewer: &ReviewerActor) -> Option<String> {
        match &reviewer.scope {
            ReviewerScope::Branch { branch_id, .. } => Some(branch_id.as_str().to_string()),
            _ => None,
        }
    }

    /// Moves a `SUBMITTED` case to `UNDER_REVIEW` (design §4.5 `verificationCaseStartReview`).
    pub async fn start_review(
        &self,
        reviewer: &ReviewerActor,
        case_id: Uuid,
        expected_version: i32,
    ) -> Result<VerificationCase, ReviewError> {
        let mut tx = self.pool.begin().await?;

        let case = self
            .cases
            .find_case_by_id_for_update(&mut tx, case_id)
            .await?
            .ok_or(ReviewError::NotFound)?;

        self.check_reviewer_scope(&mut tx, reviewer, &case).await?;

        if case.version != expected_version {
            return Err(ReviewError::StaleVersion);
        }

        let now = Utc::now();
        let updated = CaseMachine::start_review(&case, reviewer, now)?;

        sqlx::query(
            "UPDATE verification_cases \
             SET status = 'UNDER_REVIEW', version = $2, updated_at = $3 \
             WHERE id = $1 AND version = $4",
        )
        .bind(case_id)
        .bind(updated.version)
        .bind(now)
        .bind(expected_version)
        .execute(&mut *tx)
        .await?;

        let event = AuditEvent {
            actor_id: reviewer.user_id.to_string(),
            actor_role: "VERIFICATION_REVIEWER".to_string(),
            scope: case.organization_id.to_string(),
            action: "verification_case.start_review".to_string(),
            object_ref: case.id.to_string(),
            branch_id: Self::audit_branch_id(reviewer),
            before: Some(serde_json::json!({
                "status": case.status.as_str(),
            })),
            after: Some(serde_json::json!({
                "status": "UNDER_REVIEW",
            })),
        };
        self.audit
            .append(&mut tx, &event)
            .await
            .map_err(ReviewError::Store)?;

        tx.commit().await?;

        Ok(updated)
    }

    /// Moves an `UNDER_REVIEW` case to `VERIFIED` with `expires_at` strictly after today (V4, design §4.5 `verificationCaseApprove`).
    pub async fn approve(
        &self,
        reviewer: &ReviewerActor,
        case_id: Uuid,
        expected_version: i32,
        expires_at: NaiveDate,
    ) -> Result<VerificationCase, ReviewError> {
        let mut tx = self.pool.begin().await?;

        let case = self
            .cases
            .find_case_by_id_for_update(&mut tx, case_id)
            .await?
            .ok_or(ReviewError::NotFound)?;

        self.check_reviewer_scope(&mut tx, reviewer, &case).await?;

        if case.version != expected_version {
            return Err(ReviewError::StaleVersion);
        }

        if case.status != CaseStatus::UnderReview {
            return Err(ReviewError::OutOfScope);
        }

        let org_row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT timezone FROM organizations WHERE id = $1")
                .bind(case.organization_id)
                .fetch_optional(&mut *tx)
                .await?;

        let tz_str = org_row
            .and_then(|(tz,)| tz)
            .unwrap_or_else(|| "Asia/Bangkok".to_string());

        let (today, expires_at_dt): (NaiveDate, DateTime<Utc>) = sqlx::query_as(
            "SELECT \
                (now() AT TIME ZONE $1)::date AS today, \
                (($2::date + time '23:59:59.999999') AT TIME ZONE $1)::timestamptz AS expires_at_dt",
        )
        .bind(&tz_str)
        .bind(expires_at)
        .fetch_one(&mut *tx)
        .await?;

        if expires_at <= today {
            return Err(ReviewError::ValidationFailed);
        }

        let now = Utc::now();
        let updated = CaseMachine::approve(&case, reviewer, expires_at_dt, now)?;

        sqlx::query(
            "UPDATE verification_cases \
             SET status = 'VERIFIED', reviewed_by = $2, decided_at = $3, \
                 expires_at = $4, decision_reason_code = NULL, version = $5, updated_at = $3 \
             WHERE id = $1 AND version = $6",
        )
        .bind(case_id)
        .bind(reviewer.user_id)
        .bind(now)
        .bind(expires_at_dt)
        .bind(updated.version)
        .bind(expected_version)
        .execute(&mut *tx)
        .await?;

        let event = AuditEvent {
            actor_id: reviewer.user_id.to_string(),
            actor_role: "VERIFICATION_REVIEWER".to_string(),
            scope: case.organization_id.to_string(),
            action: "verification_case.approve".to_string(),
            object_ref: case.id.to_string(),
            branch_id: Self::audit_branch_id(reviewer),
            before: Some(serde_json::json!({
                "status": case.status.as_str(),
            })),
            after: Some(serde_json::json!({
                "status": "VERIFIED",
            })),
        };
        self.audit
            .append(&mut tx, &event)
            .await
            .map_err(ReviewError::Store)?;

        tx.commit().await?;

        Ok(updated)
    }

    /// Moves an `UNDER_REVIEW` case to `REJECTED` with a decided reason code (S-5, design §4.5 `verificationCaseReject`).
    pub async fn reject(
        &self,
        reviewer: &ReviewerActor,
        case_id: Uuid,
        expected_version: i32,
        reason: DecisionReason,
    ) -> Result<VerificationCase, ReviewError> {
        let mut tx = self.pool.begin().await?;

        let case = self
            .cases
            .find_case_by_id_for_update(&mut tx, case_id)
            .await?
            .ok_or(ReviewError::NotFound)?;

        self.check_reviewer_scope(&mut tx, reviewer, &case).await?;

        if case.version != expected_version {
            return Err(ReviewError::StaleVersion);
        }

        let now = Utc::now();
        let updated = CaseMachine::reject(&case, reviewer, reason, now)?;

        sqlx::query(
            "UPDATE verification_cases \
             SET status = 'REJECTED', reviewed_by = $2, decided_at = $3, \
                 decision_reason_code = $4, version = $5, updated_at = $3 \
             WHERE id = $1 AND version = $6",
        )
        .bind(case_id)
        .bind(reviewer.user_id)
        .bind(now)
        .bind(reason.as_str())
        .bind(updated.version)
        .bind(expected_version)
        .execute(&mut *tx)
        .await?;

        // VF-6 seam: ending a case (REJECTED) will call RetentionClock here

        let event = AuditEvent {
            actor_id: reviewer.user_id.to_string(),
            actor_role: "VERIFICATION_REVIEWER".to_string(),
            scope: case.organization_id.to_string(),
            action: "verification_case.reject".to_string(),
            object_ref: case.id.to_string(),
            branch_id: Self::audit_branch_id(reviewer),
            before: Some(serde_json::json!({
                "status": case.status.as_str(),
            })),
            after: Some(serde_json::json!({
                "status": "REJECTED",
            })),
        };
        self.audit
            .append(&mut tx, &event)
            .await
            .map_err(ReviewError::Store)?;

        tx.commit().await?;

        Ok(updated)
    }

    /// Moves an `UNDER_REVIEW` case to `NEEDS_UPDATE` with a decided reason code (S-5, design §4.5 `verificationCaseRequestUpdate`).
    pub async fn request_update(
        &self,
        reviewer: &ReviewerActor,
        case_id: Uuid,
        expected_version: i32,
        reason: DecisionReason,
    ) -> Result<VerificationCase, ReviewError> {
        let mut tx = self.pool.begin().await?;

        let case = self
            .cases
            .find_case_by_id_for_update(&mut tx, case_id)
            .await?
            .ok_or(ReviewError::NotFound)?;

        self.check_reviewer_scope(&mut tx, reviewer, &case).await?;

        if case.version != expected_version {
            return Err(ReviewError::StaleVersion);
        }

        let now = Utc::now();
        let updated = CaseMachine::request_update(&case, reviewer, reason, now)?;

        sqlx::query(
            "UPDATE verification_cases \
             SET status = 'NEEDS_UPDATE', reviewed_by = $2, decided_at = $3, \
                 decision_reason_code = $4, version = $5, updated_at = $3 \
             WHERE id = $1 AND version = $6",
        )
        .bind(case_id)
        .bind(reviewer.user_id)
        .bind(now)
        .bind(reason.as_str())
        .bind(updated.version)
        .bind(expected_version)
        .execute(&mut *tx)
        .await?;

        let event = AuditEvent {
            actor_id: reviewer.user_id.to_string(),
            actor_role: "VERIFICATION_REVIEWER".to_string(),
            scope: case.organization_id.to_string(),
            action: "verification_case.request_update".to_string(),
            object_ref: case.id.to_string(),
            branch_id: Self::audit_branch_id(reviewer),
            before: Some(serde_json::json!({
                "status": case.status.as_str(),
            })),
            after: Some(serde_json::json!({
                "status": "NEEDS_UPDATE",
            })),
        };
        self.audit
            .append(&mut tx, &event)
            .await
            .map_err(ReviewError::Store)?;

        tx.commit().await?;

        Ok(updated)
    }

    /// Moves a `VERIFIED` case to `REVOKED` with a decided reason code (S-5, design §4.5 `verificationCaseRevoke`).
    pub async fn revoke(
        &self,
        reviewer: &ReviewerActor,
        case_id: Uuid,
        expected_version: i32,
        reason: DecisionReason,
    ) -> Result<VerificationCase, ReviewError> {
        let mut tx = self.pool.begin().await?;

        let case = self
            .cases
            .find_case_by_id_for_update(&mut tx, case_id)
            .await?
            .ok_or(ReviewError::NotFound)?;

        self.check_reviewer_scope(&mut tx, reviewer, &case).await?;

        if case.version != expected_version {
            return Err(ReviewError::StaleVersion);
        }

        let now = Utc::now();
        let updated = CaseMachine::revoke(&case, reviewer, reason, now)?;

        sqlx::query(
            "UPDATE verification_cases \
             SET status = 'REVOKED', reviewed_by = $2, decided_at = $3, \
                 decision_reason_code = $4, version = $5, updated_at = $3 \
             WHERE id = $1 AND version = $6",
        )
        .bind(case_id)
        .bind(reviewer.user_id)
        .bind(now)
        .bind(reason.as_str())
        .bind(updated.version)
        .bind(expected_version)
        .execute(&mut *tx)
        .await?;

        // VF-6 seam: ending a case (REVOKED) will call RetentionClock here

        let event = AuditEvent {
            actor_id: reviewer.user_id.to_string(),
            actor_role: "VERIFICATION_REVIEWER".to_string(),
            scope: case.organization_id.to_string(),
            action: "verification_case.revoke".to_string(),
            object_ref: case.id.to_string(),
            branch_id: Self::audit_branch_id(reviewer),
            before: Some(serde_json::json!({
                "status": case.status.as_str(),
            })),
            after: Some(serde_json::json!({
                "status": "REVOKED",
            })),
        };
        self.audit
            .append(&mut tx, &event)
            .await
            .map_err(ReviewError::Store)?;

        tx.commit().await?;

        Ok(updated)
    }
}
