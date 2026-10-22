//! `RequestIssueService` (design `kiosk-shared-phone.md` §4.2 D5, D7, D9; §4.3): the staff scan
//! of a request QR at the queue machine and at the desk. It calls the request core in
//! `customer.rs` and copies none of its rules; it adds only the printed ticket.
//!
//! Authorisation (`QueueOperate` at the branch) is the route's: `POST /api/kiosk` authorises the
//! staff session for `branch` before calling here (§4.3a W8).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use pos_error::{AppError, AppResult};
use xc_kernel::ids::BranchId;
use xc_kernel::queue_tickets::QueueTickets;
use xc_kernel::tablet_sessions::{NoTabletSessions, TabletSessions};

use crate::domain::kiosk::value_objects::ticket_print_job::TicketPrintJob;

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
    tablet_sessions: Arc<dyn TabletSessions>,
}

impl RequestIssueService {
    pub fn new(kiosk: KioskService, queue: Arc<dyn QueueTickets>) -> Self {
        Self {
            kiosk,
            queue,
            tablet_sessions: Arc::new(NoTabletSessions),
        }
    }

    pub fn with_tablet_sessions(mut self, s: Arc<dyn TabletSessions>) -> Self {
        self.tablet_sessions = s;
        self
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
        let issued = self
            .kiosk
            .issue_by_token_with_sessions(
                branch,
                token,
                now,
                self.queue.clone(),
                self.tablet_sessions.clone(),
            )
            .await?;
        let tz: Tz = issued.timezone.parse().map_err(|_| {
            AppError::Internal(format!("invalid branch timezone: {}", issued.timezone))
        })?;
        let ticket_escpos = TicketPrintJob {
            display_number: issued.ticket.display_number.clone(),
            branch_name: issued.branch_name,
            issued_at: issued.ticket.issued_at.with_timezone(&tz).fixed_offset(),
            qr_url: format!("{origin}/q#{token}"),
        }
        .render();
        Ok(Issued {
            display_number: issued.ticket.display_number,
            ticket_id: issued.ticket.id,
            ticket_escpos,
        })
    }

    /// `findByToken` (D7, B13): the request id for the desk's `?kiosk=<id>` entry.
    pub async fn find_by_token(
        &self,
        branch: &BranchId,
        token: &str,
        now: DateTime<Utc>,
    ) -> AppResult<String> {
        self.kiosk.find_by_token(branch, token, now).await
    }
}
