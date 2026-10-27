//! `KioskQueueEnded` (design `kiosk-cancel.md` §3.7, XC-BE-GAPS-A row 6): a staff skip or cancel of a
//! kiosk-linked ticket cancels its request through the same `cancel_in` as a staff cancel, so the request
//! ends `CANCELLED` with a reason and one `KIOSK_CANCELLED` line names the staff member.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;
use xc_kernel::kiosk_queue_events::{KioskQueueEvents, KioskTicketEnd};
use xc_kernel::queue_tickets::QueueTickets;

use crate::domain::kiosk::repositories::kiosk_ports::KioskPorts;
use crate::domain::kiosk::value_objects::cancel_reason::CancelReason;
use crate::infrastructure::kiosk::kiosk_repository::PgKioskRepository;

pub struct KioskQueueEnded {
    repo: PgKioskRepository,
    ports: Arc<dyn KioskPorts>,
    queue: Arc<dyn QueueTickets>,
}

impl KioskQueueEnded {
    pub fn new(
        repo: PgKioskRepository,
        ports: Arc<dyn KioskPorts>,
        queue: Arc<dyn QueueTickets>,
    ) -> Self {
        Self { repo, ports, queue }
    }

    /// The request's cancel reason for how the ticket ended (`kiosk-cancel.md` §3.7):
    /// - `Skipped` ⇒ `CUSTOMER_LEFT` with no note (a skip is the queue's `NO_SHOW`, §2);
    /// - `Cancelled` whose reason is a staff code (`CUSTOMER_LEFT`, `CUSTOMER_CHANGED_MIND`, `DUPLICATE`) ⇒
    ///   that code with no note;
    /// - any other `Cancelled` reason ⇒ `OTHER` with the reason, trimmed, as its note.
    pub fn reason_for(end: &KioskTicketEnd) -> CancelReason {
        let _ = end;
        todo!("XC-BE-GAPS-A row 6")
    }
}

impl KioskQueueEvents for KioskQueueEnded {
    fn on_ticket_ended_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        request_id: &'a str,
        end: &'a KioskTicketEnd,
        actor_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        let _ = (
            &self.repo,
            &self.ports,
            &self.queue,
            tx,
            branch,
            request_id,
            end,
            actor_id,
            now,
        );
        todo!("XC-BE-GAPS-A row 6")
    }
}
