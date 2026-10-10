//! `NotifyError` — what `NotificationService`'s customer-facing calls refuse with.

use pos_error::AppError;

#[derive(Debug, thiserror::Error)]
pub enum NotifyError {
    /// The notification belongs to another user (design §9.1 APP-3).
    #[error("out of scope")]
    OutOfScope,
    /// No notification has this id.
    #[error("not found")]
    NotFound,
    #[error(transparent)]
    Store(#[from] AppError),
}

impl NotifyError {
    /// The v3 error code (`rates.md` §6).
    pub fn code(&self) -> &'static str {
        match self {
            NotifyError::OutOfScope => "outOfScope",
            NotifyError::NotFound => "notFound",
            NotifyError::Store(_) => "serverError",
        }
    }
}
