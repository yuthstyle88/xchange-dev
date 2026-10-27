//! `KioskQueueEvents` — the kiosk's port for a staff skip or cancel of a kiosk-linked ticket
//! (design `kiosk-cancel.md` §3.7, XC-BE-GAPS-A row 6).
//!
//! `QueueDesk::skip_in` / `cancel_in` call it inside the desk's transaction, after the ticket moved, so
//! the request it came from does not stay `WAITING` behind an ended ticket. `xc-queue` never names
//! `xc-kiosk`; `xc_kiosk::application::kiosk::queue_ended::KioskQueueEnded` implements it.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};

use crate::ids::BranchId;

/// How staff ended a kiosk-linked ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KioskTicketEnd {
    /// `skip`: CALLED → NO_SHOW.
    Skipped,
    /// `cancel`: WAITING | CALLED → CANCELLED, with the reason staff typed (`ticketCancel`'s `reason`).
    Cancelled { reason: String },
}

pub trait KioskQueueEvents: Send + Sync {
    /// Closes the kiosk request `request_id` of `branch` after staff `actor_id` ended its ticket, inside
    /// `tx`. A request already ended (or with nothing to cancel) is left as it is; an error rolls the
    /// queue action back.
    fn on_ticket_ended_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        request_id: &'a str,
        end: &'a KioskTicketEnd,
        actor_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>>;
}
