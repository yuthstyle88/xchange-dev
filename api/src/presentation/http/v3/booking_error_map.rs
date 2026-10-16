//! `From` conversions from `xc-booking` domain/application errors to [`V3Error`] (#2072).

use actix_web::http::StatusCode;
use xc_booking::application::check_in::CheckInError;
use xc_booking::domain::booking::booking_error::BookingError;
use xc_booking::domain::slot::slot_error::SlotError;

use crate::presentation::http::v3::error::V3Error;

impl From<BookingError> for V3Error {
    fn from(err: BookingError) -> Self {
        match err {
            BookingError::BookingsClosed => V3Error::new(
                "bookingsClosed",
                None,
                "BOOKINGS_CLOSED",
                StatusCode::BAD_REQUEST,
            ),
            BookingError::SlotUnavailable | BookingError::CapacityFull => V3Error::new(
                "capacityFull",
                None,
                "CAPACITY_FULL",
                StatusCode::BAD_REQUEST,
            ),
            BookingError::StaleVersion => {
                V3Error::new("staleVersion", None, "STALE_VERSION", StatusCode::CONFLICT)
            }
            BookingError::CancelCutoffPassed => V3Error::new(
                "cancelCutoffPassed",
                None,
                "CANCEL_CUTOFF_PASSED",
                StatusCode::BAD_REQUEST,
            ),
            BookingError::BookingExpired => V3Error::new(
                "bookingExpired",
                None,
                "BOOKING_EXPIRED",
                StatusCode::BAD_REQUEST,
            ),
            BookingError::ValidationFailed { field } => V3Error::new(
                "validationFailed",
                Some(field),
                "VALIDATION_FAILED",
                StatusCode::BAD_REQUEST,
            ),
            BookingError::NotFound => {
                V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND)
            }
            BookingError::TellerCancelReasonRequired => V3Error::new(
                "badRequest",
                None,
                "tellerCancelReasonRequired",
                StatusCode::BAD_REQUEST,
            ),
            BookingError::IdempotencyConflict => V3Error::new(
                "badRequest",
                None,
                "idempotencyConflict",
                StatusCode::BAD_REQUEST,
            ),
            BookingError::BookingNotUsable => V3Error::new(
                "badRequest",
                None,
                "bookingNotUsable",
                StatusCode::BAD_REQUEST,
            ),
            BookingError::BookingNotThisCustomer => V3Error::new(
                "badRequest",
                None,
                "bookingNotThisCustomer",
                StatusCode::BAD_REQUEST,
            ),
            BookingError::Store(e) => V3Error::new(
                "database",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        }
    }
}

impl From<CheckInError> for V3Error {
    fn from(err: CheckInError) -> Self {
        match err {
            CheckInError::QrMismatch => {
                V3Error::new("qrMismatch", None, "QR_MISMATCH", StatusCode::BAD_REQUEST)
            }
            CheckInError::CheckInWindow => V3Error::new(
                "checkInWindow",
                None,
                "CHECK_IN_WINDOW",
                StatusCode::BAD_REQUEST,
            ),
            CheckInError::QueueClosed => {
                V3Error::new("queueClosed", None, "QUEUE_CLOSED", StatusCode::BAD_REQUEST)
            }
            CheckInError::StaleVersion => {
                V3Error::new("staleVersion", None, "STALE_VERSION", StatusCode::CONFLICT)
            }
            CheckInError::NotFound => {
                V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND)
            }
            CheckInError::Store(e) => V3Error::new(
                "database",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        }
    }
}

impl From<SlotError> for V3Error {
    fn from(err: SlotError) -> Self {
        match err {
            SlotError::CapacityBelowReserved => V3Error::new(
                "capacityBelowReserved",
                None,
                "CAPACITY_BELOW_RESERVED",
                StatusCode::BAD_REQUEST,
            ),
            SlotError::SlotHasBookings => V3Error::new(
                "slotHasBookings",
                None,
                "SLOT_HAS_BOOKINGS",
                StatusCode::BAD_REQUEST,
            ),
            SlotError::NotFound => {
                V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND)
            }
            SlotError::CapacityFull => V3Error::new(
                "capacityFull",
                None,
                "CAPACITY_FULL",
                StatusCode::BAD_REQUEST,
            ),
            SlotError::StaleVersion => {
                V3Error::new("staleVersion", None, "STALE_VERSION", StatusCode::CONFLICT)
            }
            SlotError::Store(e) => V3Error::new(
                "database",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        }
    }
}
