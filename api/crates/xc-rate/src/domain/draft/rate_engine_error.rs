//! Rate engine errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RateEngineError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("validation failed: {0}")]
    ValidationFailed(String),

    #[error("stale version: {0}")]
    StaleVersion(String),

    #[error("approval required: {0}")]
    ApprovalRequired(String),

    #[error("rate unavailable: {0}")]
    RateUnavailable(String),

    #[error("out of scope: {0}")]
    OutOfScope(String),

    #[error("idempotency conflict: {0}")]
    IdempotencyConflict(String),

    #[error("{message}")]
    Refused { code: &'static str, message: String },

    #[error("database error: {0}")]
    Database(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl RateEngineError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "notFound",
            Self::ValidationFailed(_) => "validationFailed",
            Self::StaleVersion(_) => "staleVersion",
            Self::ApprovalRequired(_) => "approvalRequired",
            Self::RateUnavailable(_) => "rateUnavailable",
            Self::OutOfScope(_) => "outOfScope",
            Self::IdempotencyConflict(_) => "idempotencyConflict",
            Self::Refused { code, .. } => code,
            Self::Database(_) => "databaseError",
            Self::Internal(_) => "internalError",
        }
    }
}
