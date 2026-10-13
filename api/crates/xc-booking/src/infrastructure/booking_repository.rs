//! `PgBookingRepository` (design §5): PostgreSQL implementation of `BookingRepository`.

use chrono::{DateTime, NaiveDate, Utc};
use futures::future::BoxFuture;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;

use crate::domain::booking::booking::Booking;
use crate::domain::booking::booking_error::BookingError;
use crate::domain::booking::booking_event::BookingEventKind;
use crate::domain::booking::booking_no::BookingNo;
use crate::domain::booking::booking_status::BookingStatus;
use crate::domain::booking::repository::BookingRepository;
use crate::domain::booking::source_channel::SourceChannel;

#[derive(sqlx::FromRow)]
struct BookingRow {
    id: String,
    organization_id: Uuid,
    branch_id: String,
    booking_no: String,
    customer_id: String,
    slot_id: Uuid,
    quote_id: Option<Uuid>,
    shop_side: String,
    requested_currency: String,
    requested_quantity: rust_decimal::Decimal,
    source_channel: String,
    status: String,
    qr_token_hash: Vec<u8>,
    qr_version: i32,
    confirm_expires_at: Option<DateTime<Utc>>,
    checked_in_at: Option<DateTime<Utc>>,
    transaction_id: Option<Uuid>,
    partner_attribution_id: Option<Uuid>,
    ended_reason: Option<String>,
    created_by: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl BookingRow {
    fn into_booking(self) -> Booking {
        Booking {
            id: self.id,
            organization_id: self.organization_id,
            branch_id: BranchId::new(self.branch_id),
            booking_no: self.booking_no,
            customer_id: self.customer_id,
            slot_id: self.slot_id,
            quote_id: self.quote_id,
            shop_side: match self.shop_side.as_str() {
                "SELL" => Side::Sell,
                _ => Side::Buy,
            },
            requested_currency: self.requested_currency,
            requested_quantity: self.requested_quantity,
            source_channel: SourceChannel::parse(&self.source_channel)
                .unwrap_or(SourceChannel::App),
            status: BookingStatus::parse(&self.status).unwrap_or(BookingStatus::Confirmed),
            qr_token_hash: self.qr_token_hash,
            qr_version: self.qr_version as u32,
            confirm_expires_at: self.confirm_expires_at,
            checked_in_at: self.checked_in_at,
            transaction_id: self.transaction_id,
            partner_attribution_id: self.partner_attribution_id,
            ended_reason: self.ended_reason,
            created_by: self.created_by,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

pub struct PgBookingRepository {
    #[allow(dead_code)]
    pool: PgPool,
}

impl PgBookingRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl BookingRepository for PgBookingRepository {
    fn insert_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        booking: &'a Booking,
    ) -> BoxFuture<'a, Result<(), BookingError>> {
        Box::pin(async move {
            let side_str = match booking.shop_side {
                Side::Buy => "BUY",
                Side::Sell => "SELL",
            };
            sqlx::query(
                "INSERT INTO bookings ( \
                    id, organization_id, branch_id, booking_no, customer_id, slot_id, quote_id, \
                    shop_side, requested_currency, requested_quantity, source_channel, status, \
                    qr_token_hash, qr_version, confirm_expires_at, checked_in_at, transaction_id, \
                    partner_attribution_id, ended_reason, created_by, created_at, updated_at \
                ) VALUES ( \
                    $1, $2, $3, $4, $5, $6, $7, \
                    $8, $9, $10, $11, $12, \
                    $13, $14, $15, $16, $17, \
                    $18, $19, $20, $21, $22 \
                )",
            )
            .bind(&booking.id)
            .bind(booking.organization_id)
            .bind(booking.branch_id.as_str())
            .bind(&booking.booking_no)
            .bind(&booking.customer_id)
            .bind(booking.slot_id)
            .bind(booking.quote_id)
            .bind(side_str)
            .bind(&booking.requested_currency)
            .bind(booking.requested_quantity)
            .bind(booking.source_channel.as_str())
            .bind(booking.status.as_str())
            .bind(&booking.qr_token_hash)
            .bind(booking.qr_version as i32)
            .bind(booking.confirm_expires_at)
            .bind(booking.checked_in_at)
            .bind(booking.transaction_id)
            .bind(booking.partner_attribution_id)
            .bind(&booking.ended_reason)
            .bind(&booking.created_by)
            .bind(booking.created_at)
            .bind(booking.updated_at)
            .execute(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            sqlx::query(
                "INSERT INTO booking_events ( \
                    id, organization_id, booking_id, kind, from_status, to_status, actor_id, at, reason, detail, created_at \
                ) VALUES ($1, $2, $3, 'CREATE', NULL, $4, $5, $6, $7, $8, $6)",
            )
            .bind(Uuid::new_v4())
            .bind(booking.organization_id)
            .bind(&booking.id)
            .bind(booking.status.as_str())
            .bind(&booking.created_by)
            .bind(booking.created_at)
            .bind(booking.ended_reason.as_deref())
            .bind(serde_json::json!({}))
            .execute(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            Ok(())
        })
    }

    fn transition_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        booking_id: &'a str,
        expected_status: BookingStatus,
        new_status: BookingStatus,
        actor_id: &'a str,
        event_kind: BookingEventKind,
        reason: Option<&'a str>,
        detail: Option<serde_json::Value>,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<Booking, BookingError>> {
        Box::pin(async move {
            let row: Option<BookingRow> = sqlx::query_as(
                "UPDATE bookings \
                 SET status = $1, \
                     ended_reason = COALESCE($2, ended_reason), \
                     checked_in_at = CASE WHEN $1 = 'CHECKED_IN' THEN COALESCE(checked_in_at, $3) ELSE checked_in_at END, \
                     updated_at = $3 \
                 WHERE id = $4 AND status = $5 \
                 RETURNING \
                     id, organization_id, branch_id, booking_no, customer_id, slot_id, quote_id, \
                     shop_side, requested_currency, requested_quantity, source_channel, status, \
                     qr_token_hash, qr_version, confirm_expires_at, checked_in_at, transaction_id, \
                     partner_attribution_id, ended_reason, created_by, created_at, updated_at",
            )
            .bind(new_status.as_str())
            .bind(reason)
            .bind(now)
            .bind(booking_id)
            .bind(expected_status.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            let booking = match row {
                Some(r) => r.into_booking(),
                None => {
                    let existing: Option<String> =
                        sqlx::query_scalar("SELECT status FROM bookings WHERE id = $1")
                            .bind(booking_id)
                            .fetch_optional(&mut **tx)
                            .await
                            .map_err(pos_error::AppError::Database)?;
                    return match existing {
                        Some(_) => Err(BookingError::StaleVersion),
                        None => Err(BookingError::NotFound),
                    };
                }
            };

            sqlx::query(
                "INSERT INTO booking_events ( \
                    id, organization_id, booking_id, kind, from_status, to_status, actor_id, at, reason, detail, created_at \
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $8)",
            )
            .bind(Uuid::new_v4())
            .bind(booking.organization_id)
            .bind(&booking.id)
            .bind(event_kind.as_str())
            .bind(expected_status.as_str())
            .bind(new_status.as_str())
            .bind(actor_id)
            .bind(now)
            .bind(reason)
            .bind(detail.unwrap_or_else(|| serde_json::json!({})))
            .execute(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            Ok(booking)
        })
    }

    fn by_qr_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        hash: &'a [u8],
    ) -> BoxFuture<'a, Result<Option<Booking>, BookingError>> {
        Box::pin(async move {
            let row: Option<BookingRow> = sqlx::query_as(
                "SELECT \
                     id, organization_id, branch_id, booking_no, customer_id, slot_id, quote_id, \
                     shop_side, requested_currency, requested_quantity, source_channel, status, \
                     qr_token_hash, qr_version, confirm_expires_at, checked_in_at, transaction_id, \
                     partner_attribution_id, ended_reason, created_by, created_at, updated_at \
                 FROM bookings WHERE qr_token_hash = $1",
            )
            .bind(hash)
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            Ok(row.map(BookingRow::into_booking))
        })
    }

    fn by_id<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<Booking>, BookingError>> {
        Box::pin(async move {
            let row: Option<BookingRow> = sqlx::query_as(
                "SELECT \
                     id, organization_id, branch_id, booking_no, customer_id, slot_id, quote_id, \
                     shop_side, requested_currency, requested_quantity, source_channel, status, \
                     qr_token_hash, qr_version, confirm_expires_at, checked_in_at, transaction_id, \
                     partner_attribution_id, ended_reason, created_by, created_at, updated_at \
                 FROM bookings WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            Ok(row.map(BookingRow::into_booking))
        })
    }

    fn next_booking_no<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        org_id: Uuid,
        issued_on: NaiveDate,
    ) -> BoxFuture<'a, Result<String, BookingError>> {
        Box::pin(async move {
            let seq: i32 = sqlx::query_scalar(
                "INSERT INTO booking_sequences (organization_id, issued_on, last_sequence, created_at) \
                 VALUES ($1, $2, 1, now()) \
                 ON CONFLICT (organization_id, issued_on) \
                 DO UPDATE SET last_sequence = booking_sequences.last_sequence + 1 \
                 RETURNING last_sequence",
            )
            .bind(org_id)
            .bind(issued_on)
            .fetch_one(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            Ok(BookingNo::format(issued_on, seq))
        })
    }

    fn update_slot_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        booking_id: &'a str,
        expected_status: BookingStatus,
        new_slot_id: Uuid,
        actor_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<Booking, BookingError>> {
        Box::pin(async move {
            let current_booking = self
                .by_id(tx, booking_id)
                .await?
                .ok_or(BookingError::NotFound)?;
            let old_slot_id = current_booking.slot_id;

            let row: Option<BookingRow> = sqlx::query_as(
                "UPDATE bookings \
                 SET slot_id = $1, updated_at = $2 \
                 WHERE id = $3 AND status = $4 \
                 RETURNING \
                     id, organization_id, branch_id, booking_no, customer_id, slot_id, quote_id, \
                     shop_side, requested_currency, requested_quantity, source_channel, status, \
                     qr_token_hash, qr_version, confirm_expires_at, checked_in_at, transaction_id, \
                     partner_attribution_id, ended_reason, created_by, created_at, updated_at",
            )
            .bind(new_slot_id)
            .bind(now)
            .bind(booking_id)
            .bind(expected_status.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            let updated = match row {
                Some(r) => r.into_booking(),
                None => {
                    let existing: Option<String> =
                        sqlx::query_scalar("SELECT status FROM bookings WHERE id = $1")
                            .bind(booking_id)
                            .fetch_optional(&mut **tx)
                            .await
                            .map_err(pos_error::AppError::Database)?;
                    return match existing {
                        Some(_) => Err(BookingError::StaleVersion),
                        None => Err(BookingError::NotFound),
                    };
                }
            };

            let detail = serde_json::json!({
                "fromSlot": old_slot_id,
                "toSlot": new_slot_id,
            });

            sqlx::query(
                "INSERT INTO booking_events ( \
                    id, organization_id, booking_id, kind, from_status, to_status, actor_id, at, reason, detail, created_at \
                ) VALUES ($1, $2, $3, 'RESCHEDULE', $4, $4, $5, $6, NULL, $7, $6)",
            )
            .bind(Uuid::new_v4())
            .bind(updated.organization_id)
            .bind(&updated.id)
            .bind(updated.status.as_str())
            .bind(actor_id)
            .bind(now)
            .bind(detail)
            .execute(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            Ok(updated)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn reissue_qr_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        booking_id: &'a str,
        expected_status: BookingStatus,
        new_qr_token_hash: &'a [u8],
        new_qr_version: u32,
        actor_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<Booking, BookingError>> {
        Box::pin(async move {
            let row: Option<BookingRow> = sqlx::query_as(
                "UPDATE bookings \
                 SET qr_token_hash = $1, qr_version = $2, updated_at = $3 \
                 WHERE id = $4 AND status = $5 \
                 RETURNING \
                     id, organization_id, branch_id, booking_no, customer_id, slot_id, quote_id, \
                     shop_side, requested_currency, requested_quantity, source_channel, status, \
                     qr_token_hash, qr_version, confirm_expires_at, checked_in_at, transaction_id, \
                     partner_attribution_id, ended_reason, created_by, created_at, updated_at",
            )
            .bind(new_qr_token_hash)
            .bind(new_qr_version as i32)
            .bind(now)
            .bind(booking_id)
            .bind(expected_status.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            let updated = match row {
                Some(r) => r.into_booking(),
                None => {
                    let existing: Option<String> =
                        sqlx::query_scalar("SELECT status FROM bookings WHERE id = $1")
                            .bind(booking_id)
                            .fetch_optional(&mut **tx)
                            .await
                            .map_err(pos_error::AppError::Database)?;
                    return match existing {
                        Some(_) => Err(BookingError::StaleVersion),
                        None => Err(BookingError::NotFound),
                    };
                }
            };

            let detail = serde_json::json!({
                "qr_version": new_qr_version,
            });

            sqlx::query(
                "INSERT INTO booking_events ( \
                    id, organization_id, booking_id, kind, from_status, to_status, actor_id, at, reason, detail, created_at \
                ) VALUES ($1, $2, $3, 'QR_REISSUE', $4, $4, $5, $6, NULL, $7, $6)",
            )
            .bind(Uuid::new_v4())
            .bind(updated.organization_id)
            .bind(&updated.id)
            .bind(updated.status.as_str())
            .bind(actor_id)
            .bind(now)
            .bind(detail)
            .execute(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            Ok(updated)
        })
    }
}
