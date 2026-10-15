//! `BookingError` (design §4.3, §7): domain errors and refusals.

use pos_error::AppError;

use crate::domain::slot::slot_error::SlotError;

#[derive(Debug, thiserror::Error)]
pub enum BookingError {
    #[error("bookingsClosed")]
    BookingsClosed,
    #[error("slotUnavailable")]
    SlotUnavailable,
    #[error("CAPACITY_FULL")]
    CapacityFull,
    #[error("STALE_VERSION")]
    StaleVersion,
    #[error("cancelCutoffPassed")]
    CancelCutoffPassed,
    #[error("bookingExpired")]
    BookingExpired,
    #[error("VALIDATION_FAILED: {field}")]
    ValidationFailed { field: &'static str },
    #[error("tellerCancelReasonRequired")]
    TellerCancelReasonRequired,
    #[error("idempotencyConflict")]
    IdempotencyConflict,
    #[error("notFound")]
    NotFound,
    #[error("bookingNotUsable")]
    BookingNotUsable,
    #[error("bookingNotThisCustomer")]
    BookingNotThisCustomer,
    #[error(transparent)]
    Store(#[from] AppError),
}

impl BookingError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::BookingsClosed => "bookingsClosed",
            Self::SlotUnavailable => "slotUnavailable",
            Self::CapacityFull => "CAPACITY_FULL",
            Self::StaleVersion => "STALE_VERSION",
            Self::CancelCutoffPassed => "cancelCutoffPassed",
            Self::BookingExpired => "bookingExpired",
            Self::ValidationFailed { .. } => "VALIDATION_FAILED",
            Self::TellerCancelReasonRequired => "tellerCancelReasonRequired",
            Self::IdempotencyConflict => "idempotencyConflict",
            Self::NotFound => "notFound",
            Self::BookingNotUsable => "bookingNotUsable",
            Self::BookingNotThisCustomer => "bookingNotThisCustomer",
            Self::Store(_) => "STORE_ERROR",
        }
    }

    /// Maps domain error to wire error code. On the wire, `TellerCancelReasonRequired` maps to
    /// `validationFailed` with field `"reason"` (lane controller ruling A).
    pub fn wire_code(&self) -> &'static str {
        match self {
            Self::TellerCancelReasonRequired | Self::ValidationFailed { .. } => "validationFailed",
            Self::StaleVersion => "staleVersion",
            other => other.code(),
        }
    }

    /// Optional field name for validation errors on the wire.
    pub fn wire_field(&self) -> Option<&'static str> {
        match self {
            Self::TellerCancelReasonRequired => Some("reason"),
            Self::ValidationFailed { field } => Some(field),
            _ => None,
        }
    }
}

impl From<SlotError> for BookingError {
    fn from(err: SlotError) -> Self {
        match err {
            SlotError::CapacityFull => BookingError::CapacityFull,
            SlotError::StaleVersion => BookingError::StaleVersion,
            SlotError::NotFound => BookingError::NotFound,
            SlotError::Store(e) => BookingError::Store(e),
            other => BookingError::Store(AppError::Domain(other.to_string())),
        }
    }
}

impl From<BookingError> for AppError {
    fn from(err: BookingError) -> Self {
        match err {
            BookingError::BookingNotUsable => AppError::Domain("bookingNotUsable".to_string()),
            BookingError::BookingNotThisCustomer => {
                AppError::Domain("bookingNotThisCustomer".to_string())
            }
            BookingError::TellerCancelReasonRequired => AppError::Validation("reason".to_string()),
            BookingError::ValidationFailed { field } => AppError::Validation(field.to_string()),
            BookingError::StaleVersion => AppError::Conflict("staleVersion".to_string()),
            BookingError::IdempotencyConflict => {
                AppError::Conflict("idempotencyConflict".to_string())
            }
            BookingError::NotFound => AppError::NotFound {
                entity: "booking",
                id: String::new(),
            },
            BookingError::Store(e) => e,
            other => AppError::Validation(other.code().to_string()),
        }
    }
}
