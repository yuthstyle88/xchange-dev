//! Booking redeem use cases (redeem_in, use_in).

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;

use crate::domain::booking::commands::redeem_booking::RedeemBooking;
use crate::domain::booking::entities::booking_machine::BookingMachine;
use crate::domain::booking::entities::booking_steps::RedeemStep;

use super::service::BookingService;

pub const NOT_FOUND_HERE: &str = "ไม่พบการจองของสาขานี้";

impl BookingService {
    pub async fn redeem_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        cmd: RedeemBooking,
        now: DateTime<Utc>,
    ) -> AppResult<RedeemStep> {
        let booking = self
            .bookings
            .find_in(tx, branch, &cmd.booking_id)
            .await?
            .ok_or_else(|| AppError::Domain(NOT_FOUND_HERE.to_owned()))?;

        let member = if let Some(ref member_id) = booking.member_id {
            self.members.find_in(tx, branch, member_id).await?
        } else {
            None
        };

        BookingMachine::redeem(&booking, member.as_ref(), &cmd, now)
            .map_err(|e| AppError::Domain(e.to_string()))
    }

    pub async fn use_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        id: &str,
        submission_id: &str,
        document_ref: &str,
    ) -> AppResult<()> {
        let changed = self
            .bookings
            .mark_used_in(tx, branch, id, submission_id, document_ref)
            .await?;

        if !changed {
            return Err(AppError::Internal(format!(
                "booking {} could not be marked used: status changed",
                id
            )));
        }

        Ok(())
    }
}
