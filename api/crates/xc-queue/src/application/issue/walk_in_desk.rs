//! `WalkInDesk` (XC-WALKIN-QUEUE, `docs/design/walkin-queue.md` §4, W3/W4/W6/W8): the queue entry a
//! desk sale is served under. It issues through `TicketIssuer::issue_in` (the only writer of new
//! tickets) and moves tickets through `QueueDesk`, so no second writer exists.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_auth::domain::access::Actor;
use xc_kernel::ids::BranchId;

use crate::application::desk::queue_desk::QueueDesk;
use crate::application::issue::ticket_issuer::TicketIssuer;
use crate::application::status::status_service::StatusService;
use crate::domain::repository::QueueRepository;
use crate::domain::ticket::queue_ticket::QueueTicket;

/// The prefix of the idempotency key a desk sale's ticket is stored under (W5): `desk-sale:<key>`.
pub const DESK_SALE_KEY_PREFIX: &str = "desk-sale:";

#[derive(Clone)]
pub struct WalkInDesk {
    issuer: TicketIssuer,
    desk: QueueDesk,
    /// The finish step writes the `FINISH` event itself (W8 skips `QueueDesk`'s checks).
    repo: Arc<dyn QueueRepository>,
    /// The estimate refresh after the finish, as `QueueDesk::finish_in` does it.
    status: StatusService,
}

impl WalkInDesk {
    pub fn new(
        issuer: TicketIssuer,
        desk: QueueDesk,
        repo: Arc<dyn QueueRepository>,
        status: StatusService,
    ) -> Self {
        Self {
            issuer,
            desk,
            repo,
            status,
        }
    }

    /// The ticket the customer at the counter is served under, SERVING and held by `actor` when it
    /// answers (W3). In this order (W4):
    /// 1. the ticket stored under `desk-sale:<sale_key>` in the branch (a replay), unchanged;
    /// 2. the actor's held CALLED/SERVING walk-in ticket with no booking, no kiosk request and no
    ///    other desk sale's key: started if CALLED, then returned;
    /// 3. `customer_id`'s active `App` ticket of today in the branch, called and started;
    /// 4. a new `IssueSource::Desk` ticket under `desk-sale:<sale_key>`, called and started.
    ///
    /// A held booking or kiosk ticket, or a held ticket of another desk sale, refuses
    /// `QueueError::AlreadyServing`. When `may_issue` is false (W6), (1) and (2) still answer and
    /// (3) and (4) refuse `QueueError::OutsideServiceHours`. A closed queue refuses
    /// `QueueError::QueueClosed` (W7) and uses no number.
    #[allow(clippy::too_many_arguments)]
    pub async fn serve_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        customer_id: Option<&str>,
        sale_key: &str,
        may_issue: bool,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        let _ = (&self.issuer, &self.desk, tx, actor, branch, customer_id);
        let _ = (sale_key, may_issue, now);
        todo!("WQ-1")
    }

    /// Finishes the sale's ticket (W8): SERVING → COMPLETED with its `FINISH` event and the
    /// estimate refresh, as `QueueDesk::finish_in` writes them, without the on-duty and owner
    /// checks. `Ok(false)` and no change when the ticket is not SERVING.
    pub async fn finish_for_sale_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        ticket_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let _ = (&self.repo, &self.status, tx, branch, ticket_id, now);
        todo!("WQ-1")
    }
}
