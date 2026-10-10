//! Refusals of the rate-alert commands, in the existing v3 codes (`AppError` / `V3Error` codes).

/// Why a rate-alert command was refused.
#[derive(Debug, thiserror::Error)]
pub enum RateAlertError {
    /// `validationFailed`, with a reason when the design names one (`alertSettingsMissing`, `alertLimitReached`).
    #[error("validation failed: {message}")]
    ValidationFailed {
        reason: Option<&'static str>,
        message: String,
    },
    /// `outOfScope`: the alert is another customer's (R15).
    #[error("out of scope: {0}")]
    OutOfScope(String),
    /// `staleVersion`: `expectedVersion` is not the alert's version.
    #[error("stale version: {0}")]
    StaleVersion(String),
    /// `idempotencyConflict`: the key was used with another body.
    #[error("idempotency conflict: {0}")]
    IdempotencyConflict(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl RateAlertError {
    /// The v3 code: `validationFailed`, `outOfScope`, `staleVersion`, `idempotencyConflict`, `databaseError`,
    /// `internalError`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::ValidationFailed { .. } => "validationFailed",
            Self::OutOfScope(_) => "outOfScope",
            Self::StaleVersion(_) => "staleVersion",
            Self::IdempotencyConflict(_) => "idempotencyConflict",
            Self::Database(_) => "databaseError",
            Self::Internal(_) => "internalError",
        }
    }

    /// The reason of a `validationFailed` the design names (`alertSettingsMissing`, `alertLimitReached`), else `None`.
    pub fn reason(&self) -> Option<&'static str> {
        match self {
            Self::ValidationFailed { reason, .. } => *reason,
            _ => None,
        }
    }
}
