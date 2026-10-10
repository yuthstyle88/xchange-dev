//! `QueueTicket` (design §4.1, §4.2, §5): a place in a line, and its state machine.

use chrono::{DateTime, NaiveDate, Utc};
use xc_kernel::ids::BranchId;

use super::queue_error::QueueError;
use super::queue_event::EventKind;
use super::service_type::ServiceType;

/// `queue_tickets.status` (§4.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TicketStatus {
    Waiting,
    Called,
    Serving,
    Completed,
    Cancelled,
    NoShow,
}

impl TicketStatus {
    /// The stored value: `WAITING`, `CALLED`, `SERVING`, `COMPLETED`, `CANCELLED`, `NO_SHOW`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Waiting => "WAITING",
            Self::Called => "CALLED",
            Self::Serving => "SERVING",
            Self::Completed => "COMPLETED",
            Self::Cancelled => "CANCELLED",
            Self::NoShow => "NO_SHOW",
        }
    }

    /// Parses the stored value; `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "WAITING" => Some(Self::Waiting),
            "CALLED" => Some(Self::Called),
            "SERVING" => Some(Self::Serving),
            "COMPLETED" => Some(Self::Completed),
            "CANCELLED" => Some(Self::Cancelled),
            "NO_SHOW" => Some(Self::NoShow),
            _ => None,
        }
    }
}

/// A command on one ticket (§4.2). The desk (QU-A2) checks who may send it; `apply` checks only
/// the state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TicketCommand {
    /// WAITING → CALLED by this teller (`users.id`).
    Call { teller_id: String },
    /// CALLED → CALLED; event only, `called_at` unchanged.
    Recall,
    /// CALLED → NO_SHOW, final (owner Q-A3).
    Skip,
    /// CALLED → SERVING.
    Start,
    /// SERVING → COMPLETED.
    Finish,
    /// WAITING | CALLED → CANCELLED with this reason.
    Cancel { reason: String },
}

/// The ticket after a command, and the event to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    pub ticket: QueueTicket,
    pub event: EventKind,
}

/// One row of `queue_tickets`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueTicket {
    pub id: String,
    pub organization_id: String,
    pub branch_id: BranchId,
    /// The branch's local date when the ticket was issued (Q3).
    pub service_date: NaiveDate,
    pub service_type: ServiceType,
    pub sequence: i32,
    pub display_number: String,
    pub status: TicketStatus,
    pub customer_id: Option<String>,
    pub booking_id: Option<String>,
    pub kiosk_request_id: Option<String>,
    /// `users.id` of the teller who called / serves it.
    pub counter_teller_id: Option<String>,
    pub issued_at: DateTime<Utc>,
    pub called_at: Option<DateTime<Utc>>,
    pub served_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub ended_reason: Option<String>,
    pub idempotency_key: String,
}

impl QueueTicket {
    /// The state machine of §4.2. A command from any state it does not list answers
    /// `QueueError::StaleVersion` and changes nothing.
    pub fn apply(&self, cmd: &TicketCommand, now: DateTime<Utc>) -> Result<Transition, QueueError> {
        let mut ticket = self.clone();
        let event = match (self.status, cmd) {
            (TicketStatus::Waiting, TicketCommand::Call { teller_id }) => {
                ticket.status = TicketStatus::Called;
                ticket.called_at = Some(now);
                ticket.counter_teller_id = Some(teller_id.clone());
                EventKind::Call
            }
            (TicketStatus::Called, TicketCommand::Recall) => EventKind::Recall,
            (TicketStatus::Called, TicketCommand::Skip) => {
                ticket.status = TicketStatus::NoShow;
                EventKind::Skip
            }
            (TicketStatus::Called, TicketCommand::Start) => {
                ticket.status = TicketStatus::Serving;
                ticket.served_at = Some(now);
                EventKind::Start
            }
            (TicketStatus::Serving, TicketCommand::Finish) => {
                ticket.status = TicketStatus::Completed;
                ticket.completed_at = Some(now);
                EventKind::Finish
            }
            (TicketStatus::Waiting | TicketStatus::Called, TicketCommand::Cancel { reason }) => {
                ticket.status = TicketStatus::Cancelled;
                ticket.ended_reason = Some(reason.clone());
                EventKind::Cancel
            }
            _ => return Err(QueueError::StaleVersion),
        };
        Ok(Transition { ticket, event })
    }
}
