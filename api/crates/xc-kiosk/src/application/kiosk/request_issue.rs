//! `RequestIssueService` (design `kiosk-shared-phone.md` §4.2 D5, D7, D9; §4.3): the staff scan
//! of a request QR at the queue machine and at the desk. It calls the request core in
//! `customer.rs` and copies none of its rules; it adds only the printed ticket.
//!
//! Authorisation (`QueueOperate` at the branch) is the route's: `POST /api/kiosk` authorises the
//! staff session for `branch` before calling here (§4.3a W8).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;
use xc_kernel::queue_tickets::QueueTickets;

use super::service::KioskService;

/// What `issueByToken` answers (§4.3a W8): `{displayNumber, ticketId, ticketEscpos}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issued {
    pub display_number: String,
    pub ticket_id: String,
    /// `TicketPrintJob::render` of this ticket; base64 on the wire.
    pub ticket_escpos: Vec<u8>,
}

#[derive(Clone)]
pub struct RequestIssueService {
    kiosk: KioskService,
    queue: Arc<dyn QueueTickets>,
}

impl RequestIssueService {
    pub fn new(kiosk: KioskService, queue: Arc<dyn QueueTickets>) -> Self {
        Self { kiosk, queue }
    }

    /// `issueByToken` (D5, D9; B6, B7, B10): `KioskService::issue_by_token`, then the ticket
    /// rendered by `TicketPrintJob` with the ticket's own `issued_at` in the branch's timezone,
    /// and `qr_url` = `<origin>/q#<token>`. A repeat scan answers the same ticket and the same
    /// bytes. The core's refusals pass through unchanged (`token_not_found()` ⇒ 404).
    pub async fn issue_by_token(
        &self,
        branch: &BranchId,
        token: &str,
        origin: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Issued> {
        let _ = (branch, token, origin, now, &self.kiosk, &self.queue);
        todo!("KWQ-A4: core issue_by_token + TicketPrintJob")
    }

    /// `findByToken` (D7, B13): the request id for the desk's `?kiosk=<id>` entry.
    pub async fn find_by_token(
        &self,
        branch: &BranchId,
        token: &str,
        now: DateTime<Utc>,
    ) -> AppResult<String> {
        let _ = (branch, token, now);
        todo!("KWQ-A4: core find_by_token")
    }
}
