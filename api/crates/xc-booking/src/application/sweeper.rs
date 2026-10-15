//! `BookingSweeper` (design §2 B12, §5): background sweeper that marks overdue
//! bookings `EXPIRED` or `NO_SHOW`, and releases their slot places.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::booking::booking_error::BookingError;
use crate::domain::booking::booking_event::BookingEventKind;
use crate::domain::booking::booking_machine::BookingMachine;
use crate::domain::booking::booking_status::BookingStatus;
use crate::domain::booking::repository::BookingRepository;
use crate::domain::policy::booking_policy::BookingPolicy;
use crate::domain::slot::booking_slot::{BookingSlot, SlotStatus};
use crate::domain::slot::repository::SlotRepository;
use crate::domain::slot::slot_error::SlotError;
use crate::infrastructure::booking_repository::PgBookingRepository;
use crate::infrastructure::slot_repository::PgSlotRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

/// Counts of transitions made by a single sweeper run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SweepCounts {
    pub expired: usize,
    pub no_show: usize,
    pub day_ended: usize,
}

#[derive(sqlx::FromRow)]
struct PolicyRecord {
    cancel_cutoff_minutes: i32,
    no_show_grace_minutes: i32,
    confirm_expiry_minutes: i32,
}

#[derive(sqlx::FromRow)]
struct SlotRecord {
    id: Uuid,
    organization_id: Uuid,
    branch_id: String,
    service_date: chrono::NaiveDate,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    capacity: i32,
    reserved: i32,
    status: String,
}

pub struct BookingSweeper {
    pool: PgPool,
    slots: Arc<dyn SlotRepository>,
    bookings: Arc<dyn BookingRepository>,
}

impl BookingSweeper {
    pub fn new(pool: PgPool) -> Self {
        Self {
            slots: Arc::new(PgSlotRepository::new(pool.clone())),
            bookings: Arc::new(PgBookingRepository::new(pool.clone())),
            pool,
        }
    }

    pub fn with_repositories(
        pool: PgPool,
        slots: Arc<dyn SlotRepository>,
        bookings: Arc<dyn BookingRepository>,
    ) -> Self {
        Self {
            pool,
            slots,
            bookings,
        }
    }

    pub async fn run(&self, now: DateTime<Utc>) -> Result<SweepCounts, BookingError> {
        let mut counts = SweepCounts::default();

        // 1. Expire: PENDING_CUSTOMER_CONFIRMATION past confirm_expires_at -> EXPIRED
        let overdue_pending: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM bookings \
             WHERE status = 'PENDING_CUSTOMER_CONFIRMATION' \
               AND (confirm_expires_at IS NULL OR confirm_expires_at < $1) \
             ORDER BY id",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(pos_error::AppError::Database)?;

        for id in overdue_pending {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(pos_error::AppError::Database)?;
            let booking = match self.bookings.by_id(&mut tx, &id).await? {
                Some(b) => b,
                None => continue,
            };
            if booking.status != BookingStatus::PendingCustomerConfirmation {
                continue;
            }
            if BookingMachine::expire(&booking, now).is_err() {
                continue;
            }
            BranchTx::lock_also(&mut tx, &booking.branch_id).await?;
            match self
                .bookings
                .transition_in(
                    &mut tx,
                    &booking.id,
                    BookingStatus::PendingCustomerConfirmation,
                    BookingStatus::Expired,
                    "system",
                    BookingEventKind::Expire,
                    None,
                    None,
                    now,
                )
                .await
            {
                Ok(_) => {
                    if let Err(e) = self.slots.release_in(&mut tx, booking.slot_id).await {
                        match e {
                            SlotError::StaleVersion => continue,
                            err => return Err(err.into()),
                        }
                    }
                    tx.commit().await.map_err(pos_error::AppError::Database)?;
                    counts.expired += 1;
                }
                Err(BookingError::StaleVersion) => continue,
                Err(e) => return Err(e),
            }
        }

        // 2. No-show: CONFIRMED past slot.start_at + grace -> NO_SHOW
        let overdue_confirmed: Vec<String> = sqlx::query_scalar(
            "SELECT b.id FROM bookings b \
             JOIN booking_slots s ON b.slot_id = s.id \
             JOIN booking_policies p ON b.organization_id = p.organization_id \
             WHERE b.status = 'CONFIRMED' \
               AND s.start_at + (p.no_show_grace_minutes * interval '1 minute') < $1 \
             ORDER BY b.id",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(pos_error::AppError::Database)?;

        for id in overdue_confirmed {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(pos_error::AppError::Database)?;
            let booking = match self.bookings.by_id(&mut tx, &id).await? {
                Some(b) => b,
                None => continue,
            };
            if booking.status != BookingStatus::Confirmed {
                continue;
            }

            let policy_row: Option<PolicyRecord> = sqlx::query_as(
                "SELECT cancel_cutoff_minutes, no_show_grace_minutes, confirm_expiry_minutes \
                 FROM booking_policies WHERE organization_id = $1",
            )
            .bind(booking.organization_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            let policy = match policy_row {
                Some(p) => BookingPolicy {
                    cancel_cutoff_minutes: p.cancel_cutoff_minutes,
                    no_show_grace_minutes: p.no_show_grace_minutes,
                    confirm_expiry_minutes: p.confirm_expiry_minutes,
                },
                None => continue,
            };

            let slot_row: Option<SlotRecord> = sqlx::query_as(
                "SELECT id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status \
                 FROM booking_slots WHERE id = $1 FOR UPDATE",
            )
            .bind(booking.slot_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            let slot = match slot_row {
                Some(s) => BookingSlot {
                    id: s.id,
                    organization_id: s.organization_id,
                    branch_id: BranchId::new(s.branch_id),
                    service_date: s.service_date,
                    start_at: s.start_at,
                    end_at: s.end_at,
                    capacity: s.capacity,
                    reserved: s.reserved,
                    status: match s.status.as_str() {
                        "CLOSED" => SlotStatus::Closed,
                        _ => SlotStatus::Open,
                    },
                },
                None => continue,
            };

            if BookingMachine::no_show(&booking, now, &policy, &slot).is_err() {
                continue;
            }

            BranchTx::lock_also(&mut tx, &booking.branch_id).await?;

            match self
                .bookings
                .transition_in(
                    &mut tx,
                    &booking.id,
                    BookingStatus::Confirmed,
                    BookingStatus::NoShow,
                    "system",
                    BookingEventKind::NoShow,
                    None,
                    None,
                    now,
                )
                .await
            {
                Ok(_) => {
                    if let Err(e) = self.slots.release_in(&mut tx, booking.slot_id).await {
                        match e {
                            SlotError::StaleVersion => continue,
                            err => return Err(err.into()),
                        }
                    }
                    tx.commit().await.map_err(pos_error::AppError::Database)?;
                    counts.no_show += 1;
                }
                Err(BookingError::StaleVersion) => continue,
                Err(e) => return Err(e),
            }
        }

        // 3. Day-ended cancel: CHECKED_IN after slot's service date with no active ticket -> CANCELLED (dayEnded)
        let overdue_checked_in: Vec<String> = sqlx::query_scalar(
            "SELECT b.id FROM bookings b \
             JOIN booking_slots s ON b.slot_id = s.id \
             JOIN branches br ON b.branch_id = br.id \
             WHERE b.status = 'CHECKED_IN' \
               AND s.service_date < ($1 AT TIME ZONE COALESCE(NULLIF(br.timezone, ''), 'Asia/Bangkok'))::date \
               AND NOT EXISTS ( \
                   SELECT 1 FROM queue_tickets qt \
                   WHERE qt.booking_id = b.id \
                     AND qt.status IN ('WAITING', 'CALLED', 'SERVING') \
               ) \
             ORDER BY b.id",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(pos_error::AppError::Database)?;

        for id in overdue_checked_in {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(pos_error::AppError::Database)?;
            let booking = match self.bookings.by_id(&mut tx, &id).await? {
                Some(b) => b,
                None => continue,
            };
            if booking.status != BookingStatus::CheckedIn {
                continue;
            }

            let active_ticket_exists: Option<i32> = sqlx::query_scalar(
                "SELECT 1 FROM queue_tickets WHERE booking_id = $1 AND status IN ('WAITING', 'CALLED', 'SERVING')",
            )
            .bind(&booking.id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            if active_ticket_exists.is_some() {
                continue;
            }

            let policy_row: Option<PolicyRecord> = sqlx::query_as(
                "SELECT cancel_cutoff_minutes, no_show_grace_minutes, confirm_expiry_minutes \
                 FROM booking_policies WHERE organization_id = $1",
            )
            .bind(booking.organization_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            let policy = match policy_row {
                Some(p) => BookingPolicy {
                    cancel_cutoff_minutes: p.cancel_cutoff_minutes,
                    no_show_grace_minutes: p.no_show_grace_minutes,
                    confirm_expiry_minutes: p.confirm_expiry_minutes,
                },
                None => continue,
            };

            let slot_row: Option<SlotRecord> = sqlx::query_as(
                "SELECT id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status \
                 FROM booking_slots WHERE id = $1 FOR UPDATE",
            )
            .bind(booking.slot_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            let slot = match slot_row {
                Some(s) => BookingSlot {
                    id: s.id,
                    organization_id: s.organization_id,
                    branch_id: BranchId::new(s.branch_id),
                    service_date: s.service_date,
                    start_at: s.start_at,
                    end_at: s.end_at,
                    capacity: s.capacity,
                    reserved: s.reserved,
                    status: match s.status.as_str() {
                        "CLOSED" => SlotStatus::Closed,
                        _ => SlotStatus::Open,
                    },
                },
                None => continue,
            };

            if BookingMachine::cancel(
                &booking,
                "system",
                true,
                Some("dayEnded"),
                now,
                &policy,
                &slot,
            )
            .is_err()
            {
                continue;
            }

            BranchTx::lock_also(&mut tx, &booking.branch_id).await?;

            match self
                .bookings
                .transition_in(
                    &mut tx,
                    &booking.id,
                    BookingStatus::CheckedIn,
                    BookingStatus::Cancelled,
                    "system",
                    BookingEventKind::Cancel,
                    Some("dayEnded"),
                    None,
                    now,
                )
                .await
            {
                Ok(_) => {
                    if let Err(e) = self.slots.release_in(&mut tx, booking.slot_id).await {
                        match e {
                            SlotError::StaleVersion => continue,
                            err => return Err(err.into()),
                        }
                    }
                    tx.commit().await.map_err(pos_error::AppError::Database)?;
                    counts.day_ended += 1;
                }
                Err(BookingError::StaleVersion) => continue,
                Err(e) => return Err(e),
            }
        }

        Ok(counts)
    }
}
