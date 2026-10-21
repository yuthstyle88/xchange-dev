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

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;
use xc_kernel::queue_tickets::QueueTickets;

use crate::domain::kiosk::commands::new_quote::NewQuote;

use super::service::KioskService;

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
        let _ = (tx, branch, customer_id, cmd, token, now, &self.kiosk);
        todo!("KWQ-C4: D12 then quote_for_in")
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
        let _ = (tx, branch, customer_id, id, token, now);
        todo!("KWQ-C4: replace the member's token")
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
        let _ = (tx, branch, customer_id, id, now, &self.queue);
        todo!("KWQ-C4: the member's one-press cancel")
    }
}
