//! `ReferralCaptureService` (design §4.2): the attribution capture, inside the caller's
//! transaction. The callers (bookings #555, the customer app #558, the desk completion #551,
//! FD-2's save step) live in their own tickets.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_outbox::application::writer::OutboxWriter;

use crate::domain::capture::{Capture, CaptureId, CaptureSource, CaptureStatus, CaptureTarget};
use crate::domain::capture_repository::CaptureRepository;
use crate::domain::events::BROKER_ATTRIBUTION;
use crate::domain::referral_code::ReferralCode;

/// Why a capture was refused. Once routed (RA-3): `AlreadyCaptured` is 409
/// `referralAlreadyCaptured`, `NotActive` is 409 `staleVersion`, `NoSuchTarget` is 400, and
/// `NoCustomer`, `BadCode` and `EmptyReason` are 400 `validationFailed` (field `referralCode` or
/// `reason`).
#[derive(Debug)]
pub enum CaptureError {
    /// The target already has an ACTIVE capture with another code: supersede instead.
    AlreadyCaptured,
    /// The booking or transaction does not exist.
    NoSuchTarget,
    /// A booking with no v3 customer (no member with a customer).
    NoCustomer,
    BadCode,
    EmptyReason,
    /// The capture being superseded is no longer ACTIVE.
    NotActive,
    Store(pos_error::AppError),
}

impl From<pos_error::AppError> for CaptureError {
    fn from(e: pos_error::AppError) -> Self {
        Self::Store(e)
    }
}

impl From<sqlx::Error> for CaptureError {
    fn from(e: sqlx::Error) -> Self {
        Self::Store(pos_error::AppError::Database(e))
    }
}

/// One capture. The customer, the branch and the organisation are read from the target:
/// `transactions.customer_id`, or the booking's member's `customer_id`.
#[derive(Debug, Clone)]
pub struct CaptureInput {
    pub code: ReferralCode,
    pub on: CaptureTarget,
    pub source: CaptureSource,
    /// The customer's Identity subject, when the caller knows it (design D-3).
    pub customer_sub: Option<String>,
    /// `captured_by`: an actor id, or `customer:<sub>`.
    pub by: String,
    pub at: DateTime<Utc>,
}

pub struct ReferralCaptureService {
    repository: Arc<dyn CaptureRepository>,
    audit: Arc<dyn AuditLog>,
}

impl ReferralCaptureService {
    pub fn new(repository: Arc<dyn CaptureRepository>, audit: Arc<dyn AuditLog>) -> Self {
        Self { repository, audit }
    }

    /// The first capture on a target writes one ACTIVE row; the same code again answers the same
    /// id; another code is `AlreadyCaptured`. With `customer_sub`, it also writes one
    /// `broker.attribution` outbox row (key `attr:<capture id>`, payload `{"captureId": …}`) and
    /// sets `pushed_at`.
    pub async fn capture_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        input: CaptureInput,
    ) -> Result<CaptureId, CaptureError> {
        let target_info = self.repository.resolve_target(tx, &input.on).await?;

        if let Some((existing_id, existing_code)) = self
            .repository
            .find_active_for_target(tx, &input.on)
            .await?
        {
            if existing_code == input.code.as_str() {
                return Ok(existing_id);
            } else {
                return Err(CaptureError::AlreadyCaptured);
            }
        }

        let capture_id = CaptureId::new(Uuid::new_v4());
        let pushed_at = if input.customer_sub.is_some() {
            Some(input.at)
        } else {
            None
        };

        let booking_id = match &input.on {
            CaptureTarget::Booking(b) => Some(b.clone()),
            CaptureTarget::Transaction(_) => None,
        };
        let transaction_id = match &input.on {
            CaptureTarget::Transaction(t) => Some(*t),
            CaptureTarget::Booking(_) => None,
        };

        let capture = Capture {
            id: capture_id,
            organization_id: target_info.organization_id,
            branch_id: target_info.branch_id,
            code: input.code.as_str().to_owned(),
            customer_id: target_info.customer_id,
            customer_sub: input.customer_sub.clone(),
            booking_id,
            transaction_id,
            source: input.source,
            captured_at: input.at,
            captured_by: input.by,
            status: CaptureStatus::Active,
            supersedes_id: None,
            reason: None,
            pushed_at,
            broker_outcome: None,
        };

        self.repository.insert(tx, &capture).await?;

        if input.customer_sub.is_some() {
            OutboxWriter::enqueue_in(
                tx,
                &target_info.organization_id.to_string(),
                &BROKER_ATTRIBUTION,
                &capture_id.to_string(),
                &format!("attr:{capture_id}"),
                &serde_json::json!({ "captureId": capture_id.to_string() }),
            )
            .await
            .map_err(CaptureError::from)?;
        }

        Ok(capture_id)
    }

    /// Replace the ACTIVE capture `capture` with `code`, for `reason` (D-2): a new ACTIVE row with
    /// `supersedes_id`, the old one `SUPERSEDED`, and one audit event (action `supersedeReferral`,
    /// object `referral_attribution:<old id>`).
    pub async fn supersede_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        capture: &CaptureId,
        code: ReferralCode,
        reason: &str,
        by: &str,
        at: DateTime<Utc>,
    ) -> Result<CaptureId, CaptureError> {
        let trimmed_reason = reason.trim();
        if trimmed_reason.is_empty() {
            return Err(CaptureError::EmptyReason);
        }

        let old = self
            .repository
            .find_by_id(tx, capture)
            .await?
            .ok_or(CaptureError::NotActive)?;

        if old.status != CaptureStatus::Active {
            return Err(CaptureError::NotActive);
        }

        let updated = self.repository.mark_superseded(tx, capture).await?;
        if !updated {
            return Err(CaptureError::NotActive);
        }

        let new_id = CaptureId::new(Uuid::new_v4());
        let pushed_at = if old.customer_sub.is_some() {
            Some(at)
        } else {
            None
        };
        let new_capture = Capture {
            id: new_id,
            organization_id: old.organization_id,
            branch_id: old.branch_id.clone(),
            code: code.as_str().to_owned(),
            customer_id: old.customer_id.clone(),
            customer_sub: old.customer_sub.clone(),
            booking_id: old.booking_id.clone(),
            transaction_id: old.transaction_id,
            source: old.source,
            captured_at: at,
            captured_by: by.to_owned(),
            status: CaptureStatus::Active,
            supersedes_id: Some(*capture),
            reason: Some(reason.to_owned()),
            pushed_at,
            broker_outcome: None,
        };

        self.repository.insert(tx, &new_capture).await?;

        if old.customer_sub.is_some() {
            OutboxWriter::enqueue_in(
                tx,
                &old.organization_id.to_string(),
                &BROKER_ATTRIBUTION,
                &new_id.to_string(),
                &format!("attr:{new_id}"),
                &serde_json::json!({ "captureId": new_id.to_string() }),
            )
            .await
            .map_err(CaptureError::from)?;
        }

        let event = AuditEvent {
            actor_id: by.to_owned(),
            actor_role: "OWNER".to_owned(),
            scope: "referral".to_owned(),
            action: "supersedeReferral".to_owned(),
            object_ref: format!("referral_attribution:{capture}"),
            branch_id: Some(old.branch_id),
            before: Some(serde_json::json!({
                "code": old.code,
                "status": old.status.as_str(),
            })),
            after: Some(serde_json::json!({
                "code": code.as_str(),
                "reason": reason,
            })),
        };
        self.audit
            .append(tx, &event)
            .await
            .map_err(CaptureError::from)?;

        Ok(new_id)
    }

    /// #554 calls this when `customer` gets an Identity subject: every ACTIVE, unpushed capture of
    /// that customer gets `customer_sub`, an outbox row and `pushed_at`. Answers how many.
    pub async fn on_customer_sub_known(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        customer: &str,
        sub: &str,
    ) -> Result<u32, CaptureError> {
        let items = self
            .repository
            .mark_pushed_for_customer(tx, customer, sub)
            .await?;

        for (capture_id, org_id) in &items {
            OutboxWriter::enqueue_in(
                tx,
                &org_id.to_string(),
                &BROKER_ATTRIBUTION,
                &capture_id.to_string(),
                &format!("attr:{capture_id}"),
                &serde_json::json!({ "captureId": capture_id.to_string() }),
            )
            .await
            .map_err(CaptureError::from)?;
        }

        Ok(items.len() as u32)
    }

    /// The booking became `transaction`: its ACTIVE capture gets `transaction_id`. One row stays
    /// one row; a booking with no capture is a no-op.
    pub async fn link_transaction_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        booking: &str,
        transaction: Uuid,
    ) -> Result<(), CaptureError> {
        self.repository
            .link_transaction(tx, booking, transaction)
            .await
    }
}
