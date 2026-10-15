//! `BookingLifecycle` (design §5, B9, B10): implements `BookingQueueEvents`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Row, Transaction};
use xc_kernel::booking_queue_events::BookingQueueEvents;
use xc_kernel::ids::BranchId;

use crate::domain::booking::booking_error::BookingError;
use crate::domain::booking::booking_event::BookingEventKind;
use crate::domain::booking::booking_status::BookingStatus;
use crate::domain::booking::repository::BookingRepository;
use crate::domain::slot::repository::SlotRepository;

pub struct BookingLifecycle {
    bookings: Arc<dyn BookingRepository>,
    slots: Option<Arc<dyn SlotRepository>>,
}

/// Information about a usable booking verified by `check_usable_in`.
#[derive(Debug, Clone)]
pub struct UsableBooking {
    pub organization_id: uuid::Uuid,
    pub status: String,
    pub already_completed: bool,
}

impl BookingLifecycle {
    pub fn new(bookings: Arc<dyn BookingRepository>) -> Self {
        Self {
            bookings,
            slots: None,
        }
    }

    pub fn with_slots(mut self, slots: Arc<dyn SlotRepository>) -> Self {
        self.slots = Some(slots);
        self
    }

    pub fn bookings(&self) -> &Arc<dyn BookingRepository> {
        &self.bookings
    }

    /// Checks whether a booking is usable for a transaction save (design B10, §4.3, rulings R15).
    ///
    /// Locks the booking row FOR UPDATE, verifies branch, customer, status (CHECKED_IN / IN_SERVICE,
    /// or COMPLETED repeat with matching prior transaction), and returns `UsableBooking`.
    pub fn check_usable_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        booking_id: &'a str,
        customer_id: &'a str,
        prior_tx_id: Option<uuid::Uuid>,
    ) -> BoxFuture<'a, AppResult<UsableBooking>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT organization_id, branch_id, customer_id, status, transaction_id \
                 FROM bookings WHERE id = $1 FOR UPDATE",
            )
            .bind(booking_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            let Some(row) = row else {
                return Err(pos_error::AppError::from(BookingError::BookingNotUsable));
            };

            let org_id: uuid::Uuid = row.get("organization_id");
            let b_branch: String = row.get("branch_id");
            let b_customer: String = row.get("customer_id");
            let b_status: String = row.get("status");
            let b_tx_id: Option<uuid::Uuid> = row.get("transaction_id");

            if b_branch != branch.as_str() {
                return Err(pos_error::AppError::from(BookingError::BookingNotUsable));
            }

            if b_customer != customer_id {
                return Err(pos_error::AppError::from(
                    BookingError::BookingNotThisCustomer,
                ));
            }

            if b_status == "COMPLETED" {
                if prior_tx_id.is_some() && b_tx_id == prior_tx_id {
                    return Ok(UsableBooking {
                        organization_id: org_id,
                        status: b_status,
                        already_completed: true,
                    });
                } else {
                    return Err(pos_error::AppError::from(BookingError::BookingNotUsable));
                }
            }

            if b_status != "CHECKED_IN" && b_status != "IN_SERVICE" {
                return Err(pos_error::AppError::from(BookingError::BookingNotUsable));
            }

            Ok(UsableBooking {
                organization_id: org_id,
                status: b_status,
                already_completed: false,
            })
        })
    }

    /// Completes a booking for a saved transaction (design B10, §4.3, BK-7 #1405).
    ///
    /// The booking must be CHECKED_IN or IN_SERVICE, in the same branch, and belonging to the
    /// same customer. Transitions booking to COMPLETED with transaction_id and writes a COMPLETE
    /// event. Repeat completion with the same transaction is a no-op.
    #[allow(clippy::too_many_arguments)]
    pub fn complete_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        booking_id: &'a str,
        customer_id: &'a str,
        transaction_id: uuid::Uuid,
        actor_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let usable = self
                .check_usable_in(tx, branch, booking_id, customer_id, Some(transaction_id))
                .await?;

            if usable.already_completed {
                return Ok(());
            }

            sqlx::query(
                "UPDATE bookings \
                 SET status = 'COMPLETED', \
                     transaction_id = $1, \
                     updated_at = $2 \
                 WHERE id = $3",
            )
            .bind(transaction_id)
            .bind(now)
            .bind(booking_id)
            .execute(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            sqlx::query(
                "INSERT INTO booking_events ( \
                    id, organization_id, booking_id, kind, from_status, to_status, actor_id, at, reason, detail, created_at \
                 ) VALUES ($1, $2, $3, 'COMPLETE', $4, 'COMPLETED', $5, $6, NULL, $7, $6)",
            )
            .bind(uuid::Uuid::new_v4())
            .bind(usable.organization_id)
            .bind(booking_id)
            .bind(&usable.status)
            .bind(actor_id)
            .bind(now)
            .bind(serde_json::json!({ "transaction_id": transaction_id }))
            .execute(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            Ok(())
        })
    }
}

impl BookingQueueEvents for BookingLifecycle {
    fn on_ticket_serving_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        booking_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let booking = self
                .bookings
                .by_id(tx, booking_id)
                .await
                .map_err(pos_error::AppError::from)?;
            let Some(booking) = booking else {
                return Ok(());
            };

            if booking.status == BookingStatus::CheckedIn {
                self.bookings
                    .transition_in(
                        tx,
                        booking_id,
                        BookingStatus::CheckedIn,
                        BookingStatus::InService,
                        "system:queue",
                        BookingEventKind::Start,
                        None,
                        None,
                        now,
                    )
                    .await
                    .map_err(pos_error::AppError::from)?;
            }

            Ok(())
        })
    }

    fn on_ticket_ended_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        booking_id: &'a str,
        reason: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            if reason != "dayClosed" {
                return Ok(());
            }

            let booking = self
                .bookings
                .by_id(tx, booking_id)
                .await
                .map_err(pos_error::AppError::from)?;
            let Some(booking) = booking else {
                return Ok(());
            };

            if booking.status == BookingStatus::CheckedIn
                || booking.status == BookingStatus::InService
            {
                if let Some(slots) = &self.slots {
                    slots
                        .release_in(tx, booking.slot_id)
                        .await
                        .map_err(|e| pos_error::AppError::from(BookingError::from(e)))?;
                } else {
                    sqlx::query(
                        "UPDATE booking_slots SET reserved = GREATEST(reserved - 1, 0), updated_at = $2 WHERE id = $1",
                    )
                    .bind(booking.slot_id)
                    .bind(now)
                    .execute(&mut **tx)
                    .await
                    .map_err(pos_error::AppError::Database)?;
                }

                self.bookings
                    .transition_in(
                        tx,
                        booking_id,
                        booking.status,
                        BookingStatus::Cancelled,
                        "system:queue",
                        BookingEventKind::Cancel,
                        Some(reason),
                        None,
                        now,
                    )
                    .await
                    .map_err(pos_error::AppError::from)?;
            }

            Ok(())
        })
    }
}
