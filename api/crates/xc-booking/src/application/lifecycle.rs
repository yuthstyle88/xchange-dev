//! `BookingLifecycle` (design §5, B9, B10): implements `BookingQueueEvents`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
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
