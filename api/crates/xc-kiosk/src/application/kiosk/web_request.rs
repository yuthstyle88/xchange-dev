//! `WebRequestService` (design `kiosk-shared-phone.md` §4.2 D2, D3, D4, D8, D12; §4.3): the web
//! source of a request — a signed-in member's request made on the phone (or a borrowed branch
//! tablet, D16), reached later only by its request token. It calls the request core in
//! `customer.rs` and copies none of its rules.
//!
//! Every method runs on the caller's transaction (`tx`), which holds the branch's lock: the v3
//! command seam opens it, so these never open a second branch transaction.
//!
//! Every method is for the member's **own** request: an id that is not the member's, or does not
//! exist in `branch`, answers `token_not_found()` (`404 notFound`), one answer for both (no oracle).
//! The request token is minted by the caller (as the kiosk route mints the cancel token) and
//! answered once; only its SHA-256 is stored.

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::ids::BranchId;
use xc_kernel::queue_tickets::QueueTickets;

use crate::domain::kiosk::commands::new_quote::NewQuote;
use crate::domain::kiosk::entities::kiosk_machine::KioskMachine;
use crate::domain::kiosk::entities::kiosk_request::KioskRequest;
use crate::domain::kiosk::repositories::kiosk_repository::KioskRepository;
use crate::domain::kiosk::value_objects::audit_line::{AuditLine, CUSTOMER_KIOSK, KioskAudit};
use crate::domain::kiosk::value_objects::cancel_reason::{CancelReason, Cancellation};
use crate::domain::kiosk::value_objects::customer_quote::CustomerQuote;
use crate::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use crate::domain::kiosk::value_objects::kiosk_error::KioskError;
use crate::domain::kiosk::value_objects::kiosk_status::KioskStatus;

use super::customer::token_not_found;
use super::service::{KioskService, cut_to_millis, refuse, time};

/// What `webRequestCreate` answers besides the token (§4.3a W3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebRequestCreated {
    pub request_id: String,
    /// When the unscanned request is purged: created + `KioskMachine::WEB_REQUEST_LIFE_MS` (D4).
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct WebRequestService {
    kiosk: KioskService,
    queue: Arc<dyn QueueTickets>,
}

impl WebRequestService {
    pub fn new(kiosk: KioskService, queue: Arc<dyn QueueTickets>) -> Self {
        Self { kiosk, queue }
    }

    /// `webRequestCreate` (D2, D12; B3, B4): the member's open web request at this branch
    /// (`KioskRepository::open_web_request_in`) ⇒ `AppError::Conflict("alreadyQueued:<its id>")`;
    /// otherwise `KioskService::quote_for_in(tx, branch, cmd, Some(CustomerQuote{customer_id, token}), now)`.
    /// No ticket, no number.
    pub async fn create_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        customer_id: &str,
        cmd: NewQuote,
        token: &str,
        now: DateTime<Utc>,
    ) -> AppResult<WebRequestCreated> {
        while let Some(open_id) = self
            .kiosk
            .repo
            .open_web_request_in(tx, branch, customer_id)
            .await?
        {
            if let Some(open) = self.kiosk.repo.find_in(tx, branch, &open_id).await?
                && KioskMachine::is_purgeable_for(&open, true, time(now))
            {
                self.kiosk.repo.delete_in(tx, branch, &open_id).await?;
                continue;
            }
            return Err(AppError::Conflict(format!("alreadyQueued:{open_id}")));
        }
        let identified = self
            .kiosk
            .quote_for_in(
                tx,
                branch,
                cmd,
                Some(CustomerQuote {
                    customer_id: customer_id.to_owned(),
                    token: token.to_owned(),
                }),
                now,
            )
            .await?;
        let expires_at = expires_at_of(&identified.request)?;
        Ok(WebRequestCreated {
            request_id: identified.request.id,
            expires_at,
        })
    }

    /// `webRequestQrReissue` (D3; B12): replaces the stored token hash of the member's own
    /// `QUOTED` or `WAITING` request with `token`'s (`set_cancel_token_in`); the old token then
    /// leads nowhere. Answers the request's `expires_at` (unchanged by a reissue). Any other
    /// status ⇒ `token_not_found()`.
    pub async fn reissue_qr_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        customer_id: &str,
        id: &str,
        token: &str,
        now: DateTime<Utc>,
    ) -> AppResult<DateTime<Utc>> {
        let req = self.own_request_in(tx, branch, customer_id, id).await?;
        if KioskMachine::is_purgeable_for(&req, true, time(now))
            || !matches!(req.status, KioskStatus::Quoted | KioskStatus::Waiting)
        {
            return Err(token_not_found());
        }
        self.kiosk
            .repo
            .set_cancel_token_in(tx, branch, id, token)
            .await?;
        expires_at_of(&req)
    }

    /// `webRequestCancel` (D8; B11a): the member cancels their own request, one press, no approval
    /// (R-11). `WAITING` ⇒ today's cancel path with `KioskActor::Customer` and
    /// `CancelReason::CustomerCancelled` (`desk::cancel_in`, which also ends the ticket through
    /// `self.queue`). `QUOTED` (no ticket yet) ⇒ the same stored outcome: `CANCELLED`, reason
    /// `CUSTOMER_CANCELLED`, one `KIOSK_CANCELLED` audit line. Any other status ⇒
    /// `AppError::Domain` with `KioskError::CustomerCancelRefused`'s sentence.
    pub async fn cancel_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        customer_id: &str,
        id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let req = self.own_request_in(tx, branch, customer_id, id).await?;
        let at = cut_to_millis(now);
        let now_kiosk = time(now);
        match req.status {
            KioskStatus::Waiting => {
                super::desk::cancel_in(
                    tx,
                    &self.kiosk.repo,
                    &*self.kiosk.ports,
                    &*self.queue,
                    branch,
                    Some(req),
                    &KioskActor::Customer,
                    CancelReason::CustomerCancelled,
                    at,
                    now_kiosk,
                )
                .await?;
                Ok(())
            }
            KioskStatus::Quoted => {
                let reason = CancelReason::CustomerCancelled;
                let cancel = Cancellation {
                    at,
                    by: CUSTOMER_KIOSK.to_owned(),
                    reason: reason.clone(),
                };
                if !self
                    .kiosk
                    .repo
                    .mark_cancelled_in(tx, branch, &req.id, KioskStatus::Quoted, &cancel)
                    .await?
                {
                    return Err(refuse(KioskError::StaleVersion));
                }
                let line = AuditLine {
                    actor: CUSTOMER_KIOSK.to_owned(),
                    subject: req.id,
                    what: KioskAudit::Cancelled {
                        from: KioskStatus::Quoted,
                        reason,
                    },
                };
                self.kiosk.ports.record_in(tx, branch, &line, at).await?;
                Ok(())
            }
            _ => Err(refuse(KioskError::CustomerCancelRefused)),
        }
    }

    async fn own_request_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        customer_id: &str,
        id: &str,
    ) -> AppResult<KioskRequest> {
        let req = self
            .kiosk
            .repo
            .find_in(tx, branch, id)
            .await?
            .ok_or_else(token_not_found)?;
        let owner = self.kiosk.repo.customer_in(tx, branch, id).await?;
        if owner.as_deref() != Some(customer_id) {
            return Err(token_not_found());
        }
        Ok(req)
    }
}

fn expires_at_of(req: &KioskRequest) -> AppResult<DateTime<Utc>> {
    let expires_ms = req.created.epoch_millis() + KioskMachine::WEB_REQUEST_LIFE_MS;
    Utc.timestamp_millis_opt(expires_ms)
        .single()
        .ok_or_else(|| AppError::Internal(format!("invalid expires_at millis: {expires_ms}")))
}
