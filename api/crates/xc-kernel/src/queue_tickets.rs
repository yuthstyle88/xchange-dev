//! `QueueTickets` — the queue's port (design `docs/design/blueprint/queue-tickets.md` §5).
//!
//! The kiosk's `confirm` and the booking's check-in issue a ticket inside their own transaction
//! through this trait, so neither depends on `xc-queue`. `xc_queue::application::issue::
//! ticket_issuer::TicketIssuer` implements it; the root package wires it (QU-B).

use chrono::{DateTime, NaiveDate, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;

use crate::ids::BranchId;

/// Where a ticket issued through the port comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueTicketSource {
    /// A kiosk request (`kiosk_requests.id`) that has just become WAITING.
    Kiosk { request_id: String },
    /// A booking (`bookings.id`) that has just checked in.
    Booking { booking_id: String },
}

/// The ticket the port issued (or found again on a retry).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedTicket {
    /// `queue_tickets.id`.
    pub id: String,
    pub service_date: NaiveDate,
    /// `"WALK_IN"` or `"BOOKING"`.
    pub service_type: String,
    pub sequence: i32,
    /// `W001`, `B001`, …
    pub display_number: String,
}

pub trait QueueTickets: Send + Sync {
    /// Issues one ticket for `source` inside `tx`; the same key, or the same source, returns the
    /// same ticket. A closed queue refuses with `AppError::Validation("queueClosed")`.
    fn issue_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        source: &'a QueueTicketSource,
        idempotency_key: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<IssuedTicket>>;
}
