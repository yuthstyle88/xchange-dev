//! `QueueRepository` (design §5): what the queue reads and writes, always inside the caller's
//! transaction. QU-A2 adds `lock_next_waiting_in` and the counter/status reads here.

use chrono::NaiveDate;
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;

use super::ticket::queue_event::QueueEvent;
use super::ticket::queue_ticket::QueueTicket;
use super::ticket::service_type::ServiceType;

/// What a branch row says the queue needs to know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchFacts {
    pub organization_id: String,
    /// `branches.timezone` (IANA name, OR-1).
    pub timezone: String,
}

pub trait QueueRepository: Send + Sync {
    /// The branch's organisation and timezone; `None` when there is no such branch.
    fn branch_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Option<BranchFacts>>>;

    /// `branch_service_status.queue_open`; `false` when the branch has no status row.
    fn queue_open_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<bool>>;

    /// Q11: one `INSERT … ON CONFLICT (branch_id, service_date, service_type) DO UPDATE SET
    /// last_sequence = queue_sequences.last_sequence + 1 RETURNING last_sequence`; the first is 1.
    fn next_sequence_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: &'a str,
        branch: &'a BranchId,
        service_date: NaiveDate,
        service_type: ServiceType,
    ) -> BoxFuture<'a, AppResult<i32>>;

    /// The ticket stored under this idempotency key in the branch.
    fn find_by_key_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        idempotency_key: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<QueueTicket>>>;

    /// The ticket of this kiosk request, whatever its status.
    fn find_by_kiosk_request_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        request_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<QueueTicket>>>;

    /// The booking's active ticket (WAITING, CALLED or SERVING).
    fn find_active_by_booking_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        booking_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<QueueTicket>>>;

    /// Inserts a new ticket.
    fn insert_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        ticket: &'a QueueTicket,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// Appends one event (`queue_events` is never updated).
    fn insert_event_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        event: &'a QueueEvent,
    ) -> BoxFuture<'a, AppResult<()>>;
}
