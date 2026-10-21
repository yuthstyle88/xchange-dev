//! `IssuedRequest` (design `kiosk-shared-phone.md` §4.2 D5, D9): what the core answers when the
//! staff scan of a request token issues (or finds again) the request's walk-in ticket.

use xc_kernel::queue_tickets::IssuedTicket;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedRequest {
    /// `kiosk_requests.id`.
    pub request_id: String,
    /// The ticket the queue's port issued, or the same ticket again on a repeat scan (B7);
    /// its `issued_at` is the time printed on the ticket.
    pub ticket: IssuedTicket,
    /// `branches.name`, printed on the ticket.
    pub branch_name: String,
    /// `branches.timezone` (IANA), for the printed time.
    pub timezone: String,
}
