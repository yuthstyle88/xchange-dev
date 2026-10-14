//! `BookingQueueEvents` — the booking's port for queue lifecycle events (design `docs/design/blueprint/booking-slots.md` §5, B9).
//!
//! When a ticket starts serving or ends, the queue notifies the booking machine inside the same
//! transaction through this trait, so `xc-queue` never depends on `xc-booking`.
//! `xc_booking::application::lifecycle::BookingLifecycle` implements it.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};

use crate::ids::BranchId;

pub trait BookingQueueEvents: Send + Sync {
    /// Called inside the desk's transaction when a ticket for `booking_id` moves to SERVING.
    /// Moves the booking to `IN_SERVICE`.
    fn on_ticket_serving_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        booking_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// Called inside the queue's transaction when a ticket for `booking_id` ends.
    /// If `reason == "dayClosed"`, moves the booking to `CANCELLED (dayClosed)`.
    /// Other reasons (e.g. `NO_SHOW`) leave the booking unchanged.
    fn on_ticket_ended_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        booking_id: &'a str,
        reason: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>>;
}
