//! QU-7 (#1266): the `queue_board` view (design `queue-tickets.md` §4.4): the branch's WAITING
//! tickets in call order, its CALLED and SERVING tickets with their teller, and the caller's counter.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A WAITING ticket, in call order (BOOKING first, then the lowest sequence; Q6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "queue_board.ts")]
pub struct QueueBoardWaiting {
    pub id: String,
    pub display_number: String,
    /// `WALK_IN` or `BOOKING`.
    pub service_type: String,
    pub sequence: i32,
    pub issued_at: String,
}

/// A CALLED or SERVING ticket and the teller whose counter holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "queue_board.ts")]
pub struct QueueBoardCalled {
    pub id: String,
    pub display_number: String,
    pub service_type: String,
    /// `CALLED` or `SERVING`.
    pub status: String,
    /// The teller's `users.id`.
    pub teller_id: Option<String>,
    pub called_at: Option<String>,
}

/// The caller's own counter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "queue_board.ts")]
pub struct QueueBoardCounter {
    /// `ACTIVE` or `PAUSED`.
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "queue_board.ts")]
pub struct QueueBoardData {
    pub waiting: Vec<QueueBoardWaiting>,
    pub called: Vec<QueueBoardCalled>,
    /// `null` when the caller has no counter at the branch (no OPEN shift).
    pub my_counter: Option<QueueBoardCounter>,
}
