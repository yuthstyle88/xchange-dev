//! `ApprovalService`: core service orchestrating requests, decisions, and audit events.

use std::sync::Arc;

use serde_json::Value;
use sqlx::{Postgres, Transaction};
use xc_auth::domain::access::{Actor, Role};

fn requester_branch(by: &Actor) -> Option<String> {
    let has_org_exec = by
        .roles
        .iter()
        .any(|r| r.role == Role::Executive && r.branch.is_none());
    let has_org_admin = by
        .roles
        .iter()
        .any(|r| r.role == Role::Admin && r.branch.is_none());
    if has_org_exec && has_org_admin {
        return None;
    }
    const PRIORITY: [Role; 7] = [
        Role::Teller,
        Role::BranchManager,
        Role::RateManager,
        Role::Finance,
        Role::Admin,
        Role::VerificationReviewer,
        Role::Executive,
    ];
    for role in PRIORITY {
        if let Some(r) = by.roles.iter().find(|r| r.role == role) {
            return Some(
                r.branch
                    .as_ref()
                    .map(|b| b.as_str().to_string())
                    .unwrap_or_else(|| "MAIN".to_string()),
            );
        }
    }
    if let Some(r) = by.roles.first() {
        return Some(
            r.branch
                .as_ref()
                .map(|b| b.as_str().to_string())
                .unwrap_or_else(|| "MAIN".to_string()),
        );
    }
    None
}

fn approver_role(who: &Actor) -> &'static str {
    const DECIDER_PRIORITY: [Role; 9] = [
        Role::Executive,
        Role::Finance,
        Role::BranchManager,
        Role::RateManager,
        Role::Admin,
        Role::Teller,
        Role::VerificationReviewer,
        Role::Marketing,
        Role::Customer,
    ];
    for role in DECIDER_PRIORITY {
        if who.roles.iter().any(|r| r.role == role) {
            return role.as_str();
        }
    }
    "UNKNOWN"
}

use crate::application::registry::ApprovalRegistry;
use crate::domain::approval_repository::ApprovalRepository;
use crate::domain::audit_log::{AuditEvent, AuditLog};
use crate::domain::redactor::Redactor;
use crate::domain::request::{ApprovalRequest, Outcome, RequestId};
use crate::domain::subject::SubjectError;

/// Errors produced by the approval core.
#[derive(Debug, thiserror::Error)]
pub enum ApprovalError {
    #[error("out of scope: {0}")]
    OutOfScope(String),
    #[error("stale version: {0}")]
    StaleVersion(String),
    #[error("idempotency conflict: {0}")]
    IdempotencyConflict(String),
    #[error("policy refused: {code}: {message}")]
    PolicyRefused { code: &'static str, message: String },
    #[error("dependency unavailable: {code}: {message}")]
    DependencyUnavailable { code: &'static str, message: String },
    #[error("another request is already pending for this object: {0}")]
    AlreadyPending(String),
    #[error("request not found: {0}")]
    NotFound(String),
    #[error("unknown approval type: {0}")]
    UnknownType(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl ApprovalError {
    /// Returns the v3 error code string.
    pub fn code(&self) -> &'static str {
        match self {
            Self::OutOfScope(_) => "outOfScope",
            Self::StaleVersion(_) => "staleVersion",
            Self::IdempotencyConflict(_) => "idempotencyConflict",
            Self::PolicyRefused { code, .. } => code,
            Self::DependencyUnavailable { code, .. } => code,
            Self::AlreadyPending(_) => "alreadyPending",
            Self::NotFound(_) => "notFound",
            Self::UnknownType(_) => "unknownType",
            Self::Database(_) => "databaseError",
            Self::Internal(_) => "internalError",
        }
    }
}

/// The approval service coordinating requests, decisions, and audit events.
pub struct ApprovalService {
    registry: ApprovalRegistry,
    repository: Arc<dyn ApprovalRepository>,
    audit_log: Arc<dyn AuditLog>,
}

impl ApprovalService {
    /// Creates a new `ApprovalService`.
    pub fn new(
        registry: ApprovalRegistry,
        repository: Arc<dyn ApprovalRepository>,
        audit_log: Arc<dyn AuditLog>,
    ) -> Self {
        Self {
            registry,
            repository,
            audit_log,
        }
    }

    /// Opens an approval request in the caller's transaction.
    ///
    /// - Checks idempotency on `(type_name, key)`. Replays if payload and object match; conflicts otherwise.
    /// - Checks if a request for `(type_name, object_id)` is already PENDING.
    /// - Redacts payload before storing.
    /// - Binds the current `version_hash` and `policy_version`.
    pub async fn request(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        type_name: &str,
        object_id: &str,
        payload: &Value,
        by: &Actor,
        key: &str,
    ) -> Result<ApprovalRequest, ApprovalError> {
        if let Some(existing) = self
            .repository
            .find_by_type_and_key(tx, type_name, key)
            .await
            .map_err(|e| ApprovalError::Database(e.to_string()))?
        {
            let redacted = Redactor::redact(payload);
            if existing.object_id == object_id && existing.proposed_payload == redacted {
                return Ok(existing);
            } else {
                return Err(ApprovalError::IdempotencyConflict(
                    "idempotency key reused with different payload or object".into(),
                ));
            }
        }

        if self
            .repository
            .has_pending_for_object(tx, type_name, object_id)
            .await
            .map_err(|e| ApprovalError::Database(e.to_string()))?
        {
            return Err(ApprovalError::AlreadyPending(format!(
                "request already pending for type {type_name} and object {object_id}"
            )));
        }

        let (subject, policy) = self.registry.get(type_name).ok_or_else(|| {
            ApprovalError::UnknownType(format!("unknown approval type: {type_name}"))
        })?;

        let version_hash = subject
            .version_hash_or_unavailable(tx, object_id)
            .await
            .map_err(|e| match e {
                SubjectError::DependencyUnavailable { code, message } => {
                    ApprovalError::DependencyUnavailable { code, message }
                }
                SubjectError::Failed(err) => {
                    ApprovalError::Internal(format!("failed to fetch version hash: {err}"))
                }
            })?;

        let policy_version = policy.policy_version();
        let proposed_payload = Redactor::redact(payload);
        let branch_id = requester_branch(by);
        let requested_by = by.user_id.clone();

        let req = self
            .repository
            .insert_request(
                tx,
                type_name,
                branch_id,
                object_id,
                &version_hash,
                &policy_version,
                proposed_payload,
                &requested_by,
                key,
            )
            .await
            .map_err(|e| match e {
                pos_error::AppError::Conflict(_) => ApprovalError::AlreadyPending(
                    "a pending request already exists for this object".into(),
                ),
                _ => ApprovalError::Database(e.to_string()),
            })?;

        Ok(req)
    }

    /// Makes a decision on an approval request by a person.
    ///
    /// - Locks the request row with `FOR UPDATE`.
    /// - Checks status is PENDING (otherwise `staleVersion`).
    /// - Checks that `who` is not the proposer (otherwise `outOfScope`).
    /// - Checks policy allows `who` to decide.
    /// - Checks `version_hash` has not moved (otherwise `staleVersion`).
    /// - Updates request status, inserts decision row, calls `on_decided`, and logs audit event.
    pub async fn decide(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        request_id: &RequestId,
        who: &Actor,
        outcome: Outcome,
        reason: &str,
        _key: &str,
    ) -> Result<(), ApprovalError> {
        let request = self
            .repository
            .find_by_id_for_update(tx, request_id)
            .await
            .map_err(|e| ApprovalError::Database(e.to_string()))?
            .ok_or_else(|| ApprovalError::NotFound(format!("request not found: {request_id}")))?;

        if request.status != "PENDING" {
            return Err(ApprovalError::StaleVersion(format!(
                "request is no longer pending: status is {}",
                request.status
            )));
        }

        if request.requested_by == who.user_id {
            return Err(ApprovalError::OutOfScope(format!(
                "proposer {} cannot decide own request",
                who.user_id
            )));
        }

        if let Some(ref branch_id) = request.branch_id {
            let branch_org: Option<String> =
                sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = $1")
                    .bind(branch_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| ApprovalError::Database(e.to_string()))?;

            if branch_org.as_deref() != Some(who.organization_id.as_str()) {
                return Err(ApprovalError::OutOfScope(format!(
                    "actor {} is outside the organisation of branch {branch_id}",
                    who.user_id
                )));
            }
        }

        let (subject, policy) = self.registry.get(&request.type_name).ok_or_else(|| {
            ApprovalError::UnknownType(format!("unknown approval type: {}", request.type_name))
        })?;

        policy
            .may_decide(who, &request)
            .map_err(|refusal| ApprovalError::PolicyRefused {
                code: refusal.code(),
                message: refusal.message().to_string(),
            })?;

        let current_hash = subject
            .version_hash_or_unavailable(tx, &request.object_id)
            .await
            .map_err(|e| match e {
                SubjectError::DependencyUnavailable { code, message } => {
                    ApprovalError::DependencyUnavailable { code, message }
                }
                SubjectError::Failed(err) => {
                    ApprovalError::Internal(format!("failed to fetch version hash: {err}"))
                }
            })?;
        if current_hash != request.object_version_hash {
            return Err(ApprovalError::StaleVersion(format!(
                "object version hash moved from {} to {current_hash}",
                request.object_version_hash
            )));
        }

        self.repository
            .update_status(tx, request_id, outcome.as_str())
            .await
            .map_err(|e| ApprovalError::Database(e.to_string()))?;

        self.repository
            .insert_decision(
                tx,
                request_id,
                &who.user_id,
                approver_role(who),
                outcome,
                reason,
            )
            .await
            .map_err(|e| ApprovalError::Database(e.to_string()))?;

        subject
            .on_decided_or_unavailable(tx, &request, outcome)
            .await
            .map_err(|e| match e {
                SubjectError::DependencyUnavailable { code, message } => {
                    ApprovalError::DependencyUnavailable { code, message }
                }
                SubjectError::Failed(err) => {
                    ApprovalError::Internal(format!("on_decided failed: {err}"))
                }
            })?;

        let event = AuditEvent {
            actor_id: who.user_id.clone(),
            actor_role: approver_role(who).to_string(),
            scope: request.type_name.clone(),
            action: format!("approval.{}", outcome.as_str().to_lowercase()),
            object_ref: request.object_id.clone(),
            branch_id: request.branch_id.clone(),
            before: Some(serde_json::json!({ "status": "PENDING" })),
            after: Some(serde_json::json!({
                "status": outcome.as_str(),
                "reason": reason,
            })),
        };
        self.audit_log
            .append(tx, &event)
            .await
            .map_err(|e| ApprovalError::Internal(format!("audit log failed: {e}")))?;

        Ok(())
    }

    /// Writes an in-bound approval decision generated by policy evaluation.
    ///
    /// Writes approver as `policy:<version>` and reason as `in_bound`.
    pub async fn decide_by_policy(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        request_id: &RequestId,
        policy_version: &str,
    ) -> Result<(), ApprovalError> {
        let request = self
            .repository
            .find_by_id_for_update(tx, request_id)
            .await
            .map_err(|e| ApprovalError::Database(e.to_string()))?
            .ok_or_else(|| ApprovalError::NotFound(format!("request not found: {request_id}")))?;

        if request.status != "PENDING" {
            return Err(ApprovalError::StaleVersion(format!(
                "request is no longer pending: status is {}",
                request.status
            )));
        }

        let (subject, _policy) = self.registry.get(&request.type_name).ok_or_else(|| {
            ApprovalError::UnknownType(format!("unknown approval type: {}", request.type_name))
        })?;

        let current_hash = subject
            .version_hash(tx, &request.object_id)
            .await
            .map_err(|e| ApprovalError::Internal(format!("failed to fetch version hash: {e}")))?;
        if current_hash != request.object_version_hash {
            return Err(ApprovalError::StaleVersion(format!(
                "object version hash moved from {} to {current_hash}",
                request.object_version_hash
            )));
        }

        let outcome = Outcome::Approved;
        let reason = "in_bound";
        let approver_id = format!("policy:{policy_version}");
        let approver_role = "POLICY";

        self.repository
            .update_status(tx, request_id, outcome.as_str())
            .await
            .map_err(|e| ApprovalError::Database(e.to_string()))?;

        self.repository
            .insert_decision(tx, request_id, &approver_id, approver_role, outcome, reason)
            .await
            .map_err(|e| ApprovalError::Database(e.to_string()))?;

        subject
            .on_decided(tx, &request.object_id, outcome)
            .await
            .map_err(|e| ApprovalError::Internal(format!("on_decided failed: {e}")))?;

        let event = AuditEvent {
            actor_id: approver_id,
            actor_role: approver_role.to_string(),
            scope: request.type_name.clone(),
            action: format!("approval.{}", outcome.as_str().to_lowercase()),
            object_ref: request.object_id.clone(),
            branch_id: request.branch_id.clone(),
            before: Some(serde_json::json!({ "status": "PENDING" })),
            after: Some(serde_json::json!({
                "status": outcome.as_str(),
                "reason": reason,
            })),
        };
        self.audit_log
            .append(tx, &event)
            .await
            .map_err(|e| ApprovalError::Internal(format!("audit log failed: {e}")))?;

        Ok(())
    }

    /// Allows a proposer to self-decide at submit when permitted by policy (A5).
    pub async fn decide_self(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        request_id: &RequestId,
        who: &Actor,
        reason: &str,
    ) -> Result<(), ApprovalError> {
        let request = self
            .repository
            .find_by_id_for_update(tx, request_id)
            .await
            .map_err(|e| ApprovalError::Database(e.to_string()))?
            .ok_or_else(|| ApprovalError::NotFound(format!("request not found: {request_id}")))?;

        if request.status != "PENDING" {
            return Err(ApprovalError::StaleVersion(format!(
                "request is no longer pending: status is {}",
                request.status
            )));
        }

        if request.requested_by != who.user_id {
            return Err(ApprovalError::OutOfScope(format!(
                "only the proposer can self-decide, but proposer is {}",
                request.requested_by
            )));
        }

        let (subject, policy) = self.registry.get(&request.type_name).ok_or_else(|| {
            ApprovalError::UnknownType(format!("unknown approval type: {}", request.type_name))
        })?;

        if !policy.allows_self_decision(who) {
            return Err(ApprovalError::OutOfScope(
                "policy does not allow self-decision for this actor".into(),
            ));
        }

        let current_hash = subject
            .version_hash(tx, &request.object_id)
            .await
            .map_err(|e| ApprovalError::Internal(format!("failed to fetch version hash: {e}")))?;
        if current_hash != request.object_version_hash {
            return Err(ApprovalError::StaleVersion(format!(
                "object version hash moved from {} to {current_hash}",
                request.object_version_hash
            )));
        }

        let outcome = Outcome::Approved;

        self.repository
            .update_status(tx, request_id, outcome.as_str())
            .await
            .map_err(|e| ApprovalError::Database(e.to_string()))?;

        self.repository
            .insert_decision(
                tx,
                request_id,
                &who.user_id,
                approver_role(who),
                outcome,
                reason,
            )
            .await
            .map_err(|e| ApprovalError::Database(e.to_string()))?;

        subject
            .on_decided(tx, &request.object_id, outcome)
            .await
            .map_err(|e| ApprovalError::Internal(format!("on_decided failed: {e}")))?;

        let event = AuditEvent {
            actor_id: who.user_id.clone(),
            actor_role: approver_role(who).to_string(),
            scope: request.type_name.clone(),
            action: format!("approval.{}", outcome.as_str().to_lowercase()),
            object_ref: request.object_id.clone(),
            branch_id: request.branch_id.clone(),
            before: Some(serde_json::json!({ "status": "PENDING" })),
            after: Some(serde_json::json!({
                "status": outcome.as_str(),
                "reason": reason,
            })),
        };
        self.audit_log
            .append(tx, &event)
            .await
            .map_err(|e| ApprovalError::Internal(format!("audit log failed: {e}")))?;

        Ok(())
    }
}
