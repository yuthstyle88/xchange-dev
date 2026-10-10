//! `QueueError`: the queue's refusals (design §4.3). Every variant QU-A2 needs is declared here
//! already, so QU-A2 does not edit this file.

use pos_error::AppError;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum QueueError {
    /// The branch's queue is closed (or has no status row): no new ticket (§4.3 row 1).
    #[error("queueClosed")]
    QueueClosed,
    /// The ticket is not in the state the command needs; a lost race (§4.3, Q11).
    #[error("staleVersion")]
    StaleVersion,
    /// QU-A2: the caller's counter is PAUSED (Q-A4).
    #[error("counterPaused")]
    CounterPaused,
    /// QU-A2: the caller already holds a CALLED or SERVING ticket.
    #[error("alreadyServing")]
    AlreadyServing,
    /// QU-A2: the caller has no OPEN shift in the branch (Q7).
    #[error("noOpenShift")]
    NoOpenShift,
    /// QU-A2: start/finish/recall on another teller's ticket by a non-manager.
    #[error("notYourTicket")]
    NotYourTicket,
    /// QU-A2: call next with no WAITING ticket.
    #[error("nothingWaiting")]
    NothingWaiting,
    /// QU-A2: no such ticket in the branch.
    #[error("notFound")]
    NotFound,
}

impl QueueError {
    /// The v3 error code: exactly the `#[error]` text above (`"queueClosed"`, `"staleVersion"`, …).
    pub fn code(&self) -> &'static str {
        match self {
            Self::QueueClosed => "queueClosed",
            Self::StaleVersion => "staleVersion",
            Self::CounterPaused => "counterPaused",
            Self::AlreadyServing => "alreadyServing",
            Self::NoOpenShift => "noOpenShift",
            Self::NotYourTicket => "notYourTicket",
            Self::NothingWaiting => "nothingWaiting",
            Self::NotFound => "notFound",
        }
    }
}

impl From<QueueError> for AppError {
    /// [required] `QueueClosed` → `AppError::Validation("queueClosed")`, `StaleVersion` →
    /// `AppError::Conflict("staleVersion")`; [proposed] every other variant → `AppError::Validation(code)`.
    fn from(err: QueueError) -> Self {
        match err {
            QueueError::StaleVersion => {
                AppError::Conflict(QueueError::StaleVersion.code().to_owned())
            }
            other => AppError::Validation(other.code().to_owned()),
        }
    }
}
