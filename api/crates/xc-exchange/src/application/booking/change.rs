//! Booking changes (confirm, cancel).

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::ids::BranchId;

use crate::application::customer::service::NO_SUCH_BRANCH;
use crate::domain::booking::commands::confirm_booking::ConfirmBooking;
use crate::domain::booking::entities::booking_machine::BookingMachine;
use crate::domain::booking::entities::booking_steps::{CancelStep, ConfirmStep};
use crate::domain::booking::value_objects::booking_error::BookingError;

use super::service::BookingService;

impl BookingService {
    pub async fn confirm(
        &self,
        branch: &BranchId,
        cmd: ConfirmBooking,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        if !self.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_owned()));
        }

        let b = branch.clone();
        let actor_str = actor.to_string();
        let bookings = self.bookings.clone();
        let ports = self.ports.clone();

        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let booking = bookings
                        .find_in(tx, &b, &cmd.id)
                        .await?
                        .ok_or_else(|| AppError::Domain(BookingError::NotFound.to_string()))?;

                    let counter = match booking.published_rate_line_id {
                        Some(id) => {
                            ports
                                .stored_line_in(tx, &b, &booking.currency, booking.side, id)
                                .await?
                        }
                        None => None,
                    };

                    let step = BookingMachine::confirm(&booking, &cmd, counter, now)
                        .map_err(|e| AppError::Domain(e.to_string()))?;

                    match step {
                        ConfirmStep::Unchanged {} => Ok(()),
                        ConfirmStep::Confirm { expires_at } => {
                            let changed = bookings
                                .confirm_in(tx, &b, &cmd.id, expires_at, now, &actor_str)
                                .await?;
                            if !changed {
                                return Err(AppError::Internal(format!(
                                    "booking {} could not be confirmed: status changed",
                                    cmd.id
                                )));
                            }
                            Ok(())
                        }
                    }
                })
            })
            .await
    }

    pub async fn cancel(&self, branch: &BranchId, id: &str) -> AppResult<()> {
        if !self.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_owned()));
        }

        let b = branch.clone();
        let booking_id = id.to_string();
        let bookings = self.bookings.clone();

        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let booking = bookings
                        .find_in(tx, &b, &booking_id)
                        .await?
                        .ok_or_else(|| AppError::Domain(BookingError::NotFound.to_string()))?;

                    let step = BookingMachine::cancel(&booking)
                        .map_err(|e| AppError::Domain(e.to_string()))?;

                    match step {
                        CancelStep::Unchanged {} => Ok(()),
                        CancelStep::Cancel {} => {
                            let changed = bookings.cancel_in(tx, &b, &booking_id).await?;
                            if !changed {
                                return Err(AppError::Internal(format!(
                                    "booking {} could not be cancelled: status changed",
                                    booking_id
                                )));
                            }
                            Ok(())
                        }
                    }
                })
            })
            .await
    }
}
