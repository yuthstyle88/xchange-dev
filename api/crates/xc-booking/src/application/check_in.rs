//! `CheckInService` (design §4.3, §4.4, §5, §7, §8): check-in by QR or teller identity check.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppError;
use sqlx::PgPool;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::PiiKeys;
use xc_kernel::queue_tickets::{IssuedTicket, QueueTickets};

use crate::domain::booking::booking::Booking;
use crate::domain::booking::booking_event::BookingEventKind;
use crate::domain::booking::booking_status::BookingStatus;
use crate::domain::booking::repository::BookingRepository;
use crate::domain::slot::repository::SlotRepository;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckInBy {
    QrToken(String),
    IdentityChecked,
}

#[derive(Debug, thiserror::Error)]
pub enum CheckInError {
    #[error("qrMismatch")]
    QrMismatch,
    #[error("checkInWindow")]
    CheckInWindow,
    #[error("queueClosed")]
    QueueClosed,
    #[error("STALE_VERSION")]
    StaleVersion,
    #[error("notFound")]
    NotFound,
    #[error(transparent)]
    Store(#[from] AppError),
}

pub struct CheckInService {
    pool: PgPool,
    branch_tx: BranchTx,
    bookings: Arc<dyn BookingRepository>,
    slots: Arc<dyn SlotRepository>,
    queue: Arc<dyn QueueTickets>,
    pii_keys: PiiKeys,
}

impl CheckInService {
    pub fn new(
        pool: PgPool,
        branch_tx: BranchTx,
        bookings: Arc<dyn BookingRepository>,
        slots: Arc<dyn SlotRepository>,
        queue: Arc<dyn QueueTickets>,
        pii_keys: PiiKeys,
    ) -> Self {
        Self {
            pool,
            branch_tx,
            bookings,
            slots,
            queue,
            pii_keys,
        }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn branch_tx(&self) -> &BranchTx {
        &self.branch_tx
    }

    pub fn bookings(&self) -> &Arc<dyn BookingRepository> {
        &self.bookings
    }

    pub fn slots(&self) -> &Arc<dyn SlotRepository> {
        &self.slots
    }

    pub fn queue(&self) -> &Arc<dyn QueueTickets> {
        &self.queue
    }

    pub fn pii_keys(&self) -> &PiiKeys {
        &self.pii_keys
    }

    /// Check in a booking by QR token or teller identity verification inside an existing transaction.
    pub async fn check_in_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        booking_id: &str,
        by: CheckInBy,
        actor_id: &str,
        now: DateTime<Utc>,
    ) -> Result<(Booking, IssuedTicket), CheckInError> {
        let booking = self
            .bookings
            .by_id(tx, booking_id)
            .await
            .map_err(pos_error::AppError::from)?;
        let booking = booking.ok_or(CheckInError::NotFound)?;

        if booking.branch_id != *branch {
            return Err(CheckInError::NotFound);
        }

        match booking.status {
            BookingStatus::Confirmed => {}
            BookingStatus::CheckedIn => {
                let last_ticket: Option<(String,)> = sqlx::query_as(
                    "SELECT status FROM queue_tickets WHERE branch_id = $1 AND booking_id = $2 ORDER BY issued_at DESC LIMIT 1",
                )
                .bind(branch.as_str())
                .bind(booking_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(pos_error::AppError::Database)?;

                match last_ticket {
                    Some((status,)) if status == "NO_SHOW" => {}
                    _ => return Err(CheckInError::StaleVersion),
                }
            }
            _ => return Err(CheckInError::StaleVersion),
        }

        match by {
            CheckInBy::QrToken(token) => {
                let token_hash = crate::domain::booking::qr_token::QrToken::hash(&token);
                if token_hash != booking.qr_token_hash {
                    return Err(CheckInError::QrMismatch);
                }
            }
            CheckInBy::IdentityChecked => {}
        }

        #[derive(sqlx::FromRow)]
        struct SlotCheckRow {
            service_date: chrono::NaiveDate,
            start_at: DateTime<Utc>,
        }

        let slot: SlotCheckRow =
            sqlx::query_as("SELECT service_date, start_at FROM booking_slots WHERE id = $1")
                .bind(booking.slot_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(pos_error::AppError::Database)?;

        let tz_str: String = sqlx::query_scalar("SELECT timezone FROM branches WHERE id = $1")
            .bind(branch.as_str())
            .fetch_one(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

        let tz: chrono_tz::Tz = tz_str.parse().map_err(|e| {
            pos_error::AppError::Internal(format!("invalid timezone {tz_str}: {e}"))
        })?;
        let today_at_branch = now.with_timezone(&tz).date_naive();
        if slot.service_date != today_at_branch {
            return Err(CheckInError::CheckInWindow);
        }

        let grace_minutes: i32 = sqlx::query_scalar(
            "SELECT no_show_grace_minutes FROM booking_policies WHERE organization_id = $1",
        )
        .bind(booking.organization_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?
        .unwrap_or(15);

        let grace_until = slot.start_at + chrono::Duration::minutes(grace_minutes as i64);
        if now > grace_until {
            return Err(CheckInError::CheckInWindow);
        }

        let idempotency_key = format!("checkin:{}:{}", booking_id, uuid::Uuid::new_v4());
        let ticket = match self
            .queue
            .issue_in(
                tx,
                branch,
                &xc_kernel::queue_tickets::QueueTicketSource::Booking {
                    booking_id: booking_id.to_string(),
                },
                &idempotency_key,
                now,
            )
            .await
        {
            Ok(t) => t,
            Err(pos_error::AppError::Validation(msg)) if msg == "queueClosed" => {
                return Err(CheckInError::QueueClosed);
            }
            Err(pos_error::AppError::Domain(msg)) if msg == "queueClosed" => {
                return Err(CheckInError::QueueClosed);
            }
            Err(e) => return Err(CheckInError::Store(e)),
        };

        let updated_booking = self
            .bookings
            .transition_in(
                tx,
                booking_id,
                booking.status,
                BookingStatus::CheckedIn,
                actor_id,
                BookingEventKind::CheckIn,
                None,
                None,
                now,
            )
            .await
            .map_err(|e| match e {
                crate::domain::booking::booking_error::BookingError::StaleVersion => {
                    CheckInError::StaleVersion
                }
                crate::domain::booking::booking_error::BookingError::NotFound => {
                    CheckInError::NotFound
                }
                other => CheckInError::Store(other.into()),
            })?;

        Ok((updated_booking, ticket))
    }

    /// Check in a booking by QR token or teller identity verification.
    /// Issues a BOOKING queue ticket and transitions the booking to CHECKED_IN in one transaction.
    pub async fn check_in(
        &self,
        branch: &BranchId,
        booking_id: &str,
        by: CheckInBy,
        actor_id: &str,
        now: DateTime<Utc>,
    ) -> Result<(Booking, IssuedTicket), CheckInError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(pos_error::AppError::Database)?;

        BranchTx::lock_also(&mut tx, branch).await?;

        let res = self
            .check_in_in(&mut tx, branch, booking_id, by, actor_id, now)
            .await?;

        tx.commit().await.map_err(pos_error::AppError::Database)?;

        Ok(res)
    }
}
