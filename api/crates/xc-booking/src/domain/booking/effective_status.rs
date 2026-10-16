//! `effective_status` (design `docs/design/blueprint/booking-slots.md` B12, §5.2):
//! pure reader-side overdue status calculation matching the sweeper and machine predicates.

use chrono::{DateTime, NaiveDate, Utc};

use super::booking_status::BookingStatus;

/// Facts needed to decide whether a booking row is overdue at `now`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverdueFacts {
    pub confirm_expires_at: Option<DateTime<Utc>>,
    pub grace_until: DateTime<Utc>,
    pub service_date: NaiveDate,
    pub branch_today: NaiveDate,
    pub has_active_ticket: bool,
}

/// Computes the effective status of a booking row at `now` without mutating stored state.
pub fn effective_status(
    status: BookingStatus,
    facts: &OverdueFacts,
    now: DateTime<Utc>,
) -> BookingStatus {
    match status {
        BookingStatus::PendingCustomerConfirmation => {
            if facts.confirm_expires_at.is_none_or(|exp| now > exp) {
                BookingStatus::Expired
            } else {
                BookingStatus::PendingCustomerConfirmation
            }
        }
        BookingStatus::Confirmed => {
            if now > facts.grace_until {
                BookingStatus::NoShow
            } else {
                BookingStatus::Confirmed
            }
        }
        BookingStatus::CheckedIn => {
            if facts.service_date < facts.branch_today && !facts.has_active_ticket {
                BookingStatus::Cancelled
            } else {
                BookingStatus::CheckedIn
            }
        }
        other => other,
    }
}
