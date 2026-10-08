//! Booking state machine.

use chrono::{DateTime, Duration, Utc};
use xc_kernel::money::Rate;

use crate::domain::booking::commands::confirm_booking::ConfirmBooking;
use crate::domain::booking::commands::create_booking::CreateBooking;
use crate::domain::booking::commands::redeem_booking::RedeemBooking;
use crate::domain::booking::entities::booking::Booking;
use crate::domain::booking::entities::booking_steps::{
    CancelStep, ConfirmStep, CreateFacts, NewBooking, RedeemStep,
};
use crate::domain::booking::entities::member::Member;
use crate::domain::booking::value_objects::booking_error::BookingError;
use crate::domain::booking::value_objects::booking_status::BookingStatus;

pub struct BookingMachine;

impl BookingMachine {
    /// Creates a new booking.
    ///
    /// Refusal order:
    /// 8: customer_id given but no member_id -> NoMember
    /// 9: rules not enabled -> Closed
    /// 10: open_shift is false -> NoShift
    /// 11: counter rate is None or does not equal expected_rate -> RateMoved
    pub fn create(
        cmd: &CreateBooking,
        facts: &CreateFacts,
        now: DateTime<Utc>,
    ) -> Result<NewBooking, BookingError> {
        if cmd.customer_id.is_some() && facts.member_id.is_none() {
            return Err(BookingError::NoMember);
        }
        if !facts.rules.enabled {
            return Err(BookingError::Closed);
        }
        if !facts.open_shift {
            return Err(BookingError::NoShift);
        }
        let counter_rate = match facts.counter {
            Some(rate) if rate == cmd.expected_rate => rate,
            _ => return Err(BookingError::RateMoved),
        };

        if cmd.customer_id.is_some() {
            Ok(NewBooking {
                status: BookingStatus::Pending,
                rate: counter_rate,
                expires_at: None,
                member_id: facts.member_id.clone(),
            })
        } else {
            Ok(NewBooking {
                status: BookingStatus::Confirmed,
                rate: counter_rate,
                expires_at: Some(now + Duration::minutes(facts.rules.minutes as i64)),
                member_id: None,
            })
        }
    }

    /// Confirms a pending member booking.
    ///
    /// Refusal order:
    /// 3: expires_at is None or <= now -> ExpiryInFuture
    /// 4: double confirm (stored Confirmed, same expires_at to millisecond) -> Unchanged
    /// 5: not Pending, or no member -> OnlyPendingMember
    /// 6: counter is None or does not equal expected_rate -> RateMoved
    pub fn confirm(
        booking: &Booking,
        cmd: &ConfirmBooking,
        counter: Option<Rate>,
        now: DateTime<Utc>,
    ) -> Result<ConfirmStep, BookingError> {
        let sent_expiry = match cmd.expires_at {
            Some(exp) if exp > now => exp,
            _ => return Err(BookingError::ExpiryInFuture),
        };

        if booking.status == BookingStatus::Confirmed {
            if let Some(existing_expiry) = booking.expires_at {
                if existing_expiry.timestamp_millis() == sent_expiry.timestamp_millis() {
                    return Ok(ConfirmStep::Unchanged {});
                }
            }
        }

        if booking.status != BookingStatus::Pending || !booking.is_member() {
            return Err(BookingError::OnlyPendingMember);
        }

        let locked_rate = match (counter, cmd.expected_rate) {
            (Some(c), Some(exp_r)) if c == exp_r => c,
            _ => return Err(BookingError::RateMoved),
        };

        Ok(ConfirmStep::Confirm {
            rate: locked_rate,
            expires_at: sent_expiry,
        })
    }

    /// Cancels a booking.
    ///
    /// - Used -> AlreadyUsed
    /// - Cancelled -> Unchanged
    /// - Pending / Confirmed -> Cancel
    pub fn cancel(booking: &Booking) -> Result<CancelStep, BookingError> {
        match booking.status {
            BookingStatus::Used => Err(BookingError::AlreadyUsed),
            BookingStatus::Cancelled => Ok(CancelStep::Unchanged {}),
            BookingStatus::Pending | BookingStatus::Confirmed => Ok(CancelStep::Cancel {}),
        }
    }

    /// Redeems a booking at the desk.
    ///
    /// Rows 2-5 of Using a booking:
    /// 2: stored Used: if submission_id matches -> passes row 2-3 as repeat, else UsedByAnother
    /// 3: not Used, and (not Confirmed or expires_at <= now) -> NotUsable
    /// 4: !live || side != || currency != || amount != || rate != || form_booking != booking.id || !customer_confirmed -> FormMismatch
    /// 5: booking has member_id: member is None or member.customer_id != cmd.customer_id -> MemberMismatch
    pub fn redeem(
        booking: &Booking,
        member: Option<&Member>,
        cmd: &RedeemBooking,
        now: DateTime<Utc>,
    ) -> Result<RedeemStep, BookingError> {
        let is_repeat = if booking.status == BookingStatus::Used {
            if booking.submission_id.as_deref() == Some(&cmd.submission_id) {
                true
            } else {
                return Err(BookingError::UsedByAnother);
            }
        } else {
            false
        };

        if !is_repeat {
            if booking.status != BookingStatus::Confirmed {
                return Err(BookingError::NotUsable);
            }
            match booking.expires_at {
                Some(exp) if exp > now => {}
                _ => return Err(BookingError::NotUsable),
            }
        }

        if !cmd.live
            || cmd.side != booking.side
            || cmd.currency != booking.currency
            || cmd.amount != booking.amount
            || cmd.rate != booking.rate
            || cmd.form_booking != booking.id
            || !cmd.customer_confirmed
        {
            return Err(BookingError::FormMismatch);
        }

        if booking.is_member() {
            match member {
                Some(m) if m.customer_id == cmd.customer_id => {}
                _ => return Err(BookingError::MemberMismatch),
            }
        }

        if is_repeat {
            Ok(RedeemStep::Repeat {})
        } else {
            Ok(RedeemStep::Use {})
        }
    }
}
