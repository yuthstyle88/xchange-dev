//! QU-7 (#1266): the `my_queue_ticket` view (design `queue-tickets.md` §4.4, #558): one ticket,
//! shown only to its own customer.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_queue_ticket.ts")]
pub struct MyQueueTicketData {
    pub ticket_id: String,
    pub branch: String,
    pub display_number: String,
    /// `WALK_IN` or `BOOKING`.
    pub service_type: String,
    /// `WAITING`, `CALLED`, `SERVING`, `COMPLETED`, `NO_SHOW` or `CANCELLED`.
    pub status: String,
    /// WAITING tickets before this one in call order; 0 once it is no longer WAITING.
    pub tickets_ahead: i32,
    /// This ticket's wait and its range (`WaitEstimator`, Q-A5/Q-A6); `null` together when there is
    /// nothing to estimate from, or once the ticket is no longer WAITING.
    pub estimate_seconds: Option<i32>,
    pub estimate_low_seconds: Option<i32>,
    pub estimate_high_seconds: Option<i32>,
    pub estimate_as_of: Option<String>,
}
