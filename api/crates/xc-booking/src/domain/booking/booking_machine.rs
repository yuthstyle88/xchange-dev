//! `BookingMachine` (design §4.2, §4.3, §5): pure state transitions for bookings.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::policy::booking_policy::BookingPolicy;
use crate::domain::slot::booking_slot::BookingSlot;

use super::booking::Booking;
use super::booking_error::BookingError;
use super::booking_status::BookingStatus;

pub struct BookingMachine;

impl BookingMachine {
    pub fn confirm(
        booking: &Booking,
        now: DateTime<Utc>,
        _policy: &BookingPolicy,
    ) -> Result<BookingStatus, BookingError> {
        if booking.status != BookingStatus::PendingCustomerConfirmation {
            return Err(BookingError::StaleVersion);
        }
        if let Some(expires_at) = booking.confirm_expires_at {
            if now > expires_at {
                return Err(BookingError::BookingExpired);
            }
        }
        Ok(BookingStatus::Confirmed)
    }

    pub fn cancel(
        booking: &Booking,
        _actor: &str,
        is_staff: bool,
        reason: Option<&str>,
        _now: DateTime<Utc>,
        _policy: &BookingPolicy,
        _slot: &BookingSlot,
    ) -> Result<BookingStatus, BookingError> {
        match booking.status {
            BookingStatus::PendingCustomerConfirmation
            | BookingStatus::Confirmed
            | BookingStatus::CheckedIn
            | BookingStatus::InService => {}
            _ => return Err(BookingError::StaleVersion),
        }

        if is_staff {
            if reason.is_none_or(|r| r.trim().is_empty()) {
                return Err(BookingError::TellerCancelReasonRequired);
            }
        } else if booking.status != BookingStatus::PendingCustomerConfirmation
            && booking.status != BookingStatus::Confirmed
        {
            return Err(BookingError::StaleVersion);
        }

        Ok(BookingStatus::Cancelled)
    }

    pub fn reschedule(
        booking: &Booking,
        _actor: &str,
        is_staff: bool,
        _new_slot: &BookingSlot,
        now: DateTime<Utc>,
        policy: &BookingPolicy,
        current_slot: &BookingSlot,
    ) -> Result<BookingStatus, BookingError> {
        if booking.status != BookingStatus::PendingCustomerConfirmation
            && booking.status != BookingStatus::Confirmed
        {
            return Err(BookingError::StaleVersion);
        }

        if !is_staff && now > policy.cutoff_at(current_slot) {
            return Err(BookingError::CancelCutoffPassed);
        }

        Ok(booking.status)
    }

    pub fn check_in(
        booking: &Booking,
        now: DateTime<Utc>,
        policy: &BookingPolicy,
        slot: &BookingSlot,
    ) -> Result<BookingStatus, BookingError> {
        if booking.status != BookingStatus::Confirmed && booking.status != BookingStatus::CheckedIn
        {
            return Err(BookingError::StaleVersion);
        }
        if now > policy.grace_until(slot) {
            return Err(BookingError::ValidationFailed {
                field: "gracePeriod",
            });
        }
        Ok(BookingStatus::CheckedIn)
    }

    pub fn start(booking: &Booking) -> Result<BookingStatus, BookingError> {
        if booking.status != BookingStatus::CheckedIn {
            return Err(BookingError::StaleVersion);
        }
        Ok(BookingStatus::InService)
    }

    pub fn complete(
        booking: &Booking,
        _transaction_id: Uuid,
    ) -> Result<BookingStatus, BookingError> {
        if booking.status != BookingStatus::CheckedIn && booking.status != BookingStatus::InService
        {
            return Err(BookingError::StaleVersion);
        }
        Ok(BookingStatus::Completed)
    }

    pub fn expire(booking: &Booking, now: DateTime<Utc>) -> Result<BookingStatus, BookingError> {
        if booking.status != BookingStatus::PendingCustomerConfirmation {
            return Err(BookingError::StaleVersion);
        }
        if let Some(expires_at) = booking.confirm_expires_at {
            if now <= expires_at {
                return Err(BookingError::ValidationFailed {
                    field: "confirmExpiresAt",
                });
            }
        }
        Ok(BookingStatus::Expired)
    }

    pub fn no_show(
        booking: &Booking,
        now: DateTime<Utc>,
        policy: &BookingPolicy,
        slot: &BookingSlot,
    ) -> Result<BookingStatus, BookingError> {
        if booking.status != BookingStatus::Confirmed {
            return Err(BookingError::StaleVersion);
        }
        if now <= policy.grace_until(slot) {
            return Err(BookingError::ValidationFailed {
                field: "gracePeriod",
            });
        }
        Ok(BookingStatus::NoShow)
    }
}
