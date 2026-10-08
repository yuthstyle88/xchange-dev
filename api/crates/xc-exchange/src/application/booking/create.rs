//! Create booking use case.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::ids::BranchId;

use crate::application::customer::service::NO_SUCH_BRANCH;
use crate::domain::booking::commands::create_booking::CreateBooking;
use crate::domain::booking::entities::booking::Booking;
use crate::domain::booking::entities::booking_machine::BookingMachine;
use crate::domain::booking::entities::booking_steps::CreateFacts;
use crate::domain::booking::value_objects::booking_error::BookingError;
use crate::domain::booking::value_objects::booking_rules::BookingRules;

use super::service::BookingService;

impl BookingService {
    pub async fn create(
        &self,
        branch: &BranchId,
        cmd: CreateBooking,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Booking> {
        if !self.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_owned()));
        }

        let b = branch.clone();
        let actor_str = actor.to_string();
        let bookings = self.bookings.clone();
        let members = self.members.clone();
        let ports = self.ports.clone();
        let ids = self.ids.clone();

        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let request_hash = cmd.request_hash();
                    if let Some(stored) = bookings.find_request_in(tx, &b, &cmd.request_id).await? {
                        if stored.hash == request_hash {
                            return Ok(stored.booking);
                        }
                        return Err(AppError::Domain(BookingError::Changed.to_string()));
                    }

                    let member_id = if let Some(ref customer_id) = cmd.customer_id {
                        let live = ports.live_customer(&b, customer_id).await?;
                        if live.is_some() {
                            let member = members.by_customer_in(tx, &b, customer_id).await?;
                            member.map(|m| m.id)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let rules = ports
                        .rules_in(tx, &b)
                        .await?
                        .unwrap_or(BookingRules::DEFAULT);
                    let open_shift = ports.has_open_shift_in(tx, &b).await?;
                    let counter = ports
                        .counter_rate_in(tx, &b, &cmd.currency, cmd.side)
                        .await?;

                    let facts = CreateFacts {
                        rules,
                        open_shift,
                        counter,
                        member_id,
                    };

                    let new_booking = BookingMachine::create(&cmd, &facts, now)
                        .map_err(|e| AppError::Domain(e.to_string()))?;

                    let id = ids.booking_id();
                    let booking = Booking {
                        id,
                        branch_id: b.clone(),
                        name: cmd.name.clone(),
                        phone: cmd.phone.clone(),
                        member_id: new_booking.member_id,
                        side: cmd.side,
                        currency: cmd.currency,
                        amount: cmd.amount,
                        rate: new_booking.rate,
                        status: new_booking.status,
                        created_at: now,
                        created_by: actor_str,
                        expires_at: new_booking.expires_at,
                        confirmed_at: None,
                        confirmed_by: None,
                        submission_id: None,
                        transaction_ref: None,
                    };

                    bookings
                        .insert_in(tx, &booking, &cmd.request_id, &request_hash)
                        .await?;

                    Ok(booking)
                })
            })
            .await
    }
}
