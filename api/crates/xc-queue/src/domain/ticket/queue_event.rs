//! `QueueEvent` (design §4.1 `queue_events`, Blueprint §16): one immutable row per transition.

use chrono::{DateTime, Utc};

use super::queue_ticket::TicketStatus;

/// What happened (`queue_events.kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Issue,
    Call,
    Recall,
    Start,
    Finish,
    Skip,
    Cancel,
}

impl EventKind {
    /// The stored value: `ISSUE`, `CALL`, `RECALL`, `START`, `FINISH`, `SKIP`, `CANCEL`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Issue => "ISSUE",
            Self::Call => "CALL",
            Self::Recall => "RECALL",
            Self::Start => "START",
            Self::Finish => "FINISH",
            Self::Skip => "SKIP",
            Self::Cancel => "CANCEL",
        }
    }
}

/// One row of `queue_events`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueEvent {
    pub id: String,
    pub organization_id: String,
    pub ticket_id: String,
    /// The ticket's status after the event.
    pub status: TicketStatus,
    pub kind: EventKind,
    /// `users.id`; `None` when no person acted (the kiosk).
    pub actor_id: Option<String>,
    pub at: DateTime<Utc>,
    pub reason: Option<String>,
}
