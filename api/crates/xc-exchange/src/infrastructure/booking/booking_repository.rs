//! Booking repository postgres implementation.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Row};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side};

use crate::domain::booking::entities::booking::Booking;
use crate::domain::booking::entities::booking_steps::StoredRequest;
use crate::domain::booking::repositories::booking_repository::BookingRepository;
use crate::domain::booking::value_objects::booking_status::BookingStatus;

#[derive(Clone)]
pub struct PgBookingRepository {
    pool: PgPool,
}

impl PgBookingRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

const LIST_BOOKINGS: &str = "\
    SELECT id, branch_id, request_id, request_hash, name, phone, member_id, \
           type, code::text AS code, amount::text AS amount, rate::text AS rate, status, \
           created_at, created_by, expires_at, confirmed_at, confirmed_by, \
           submission_id, transaction_ref \
    FROM bookings \
    WHERE branch_id = $1 \
    ORDER BY created_at DESC, id DESC \
    LIMIT $2";

const FIND_BOOKING: &str = "\
    SELECT id, branch_id, request_id, request_hash, name, phone, member_id, \
           type, code::text AS code, amount::text AS amount, rate::text AS rate, status, \
           created_at, created_by, expires_at, confirmed_at, confirmed_by, \
           submission_id, transaction_ref \
    FROM bookings \
    WHERE branch_id = $1 AND id = $2";

const FIND_REQUEST: &str = "\
    SELECT id, branch_id, request_id, request_hash, name, phone, member_id, \
           type, code::text AS code, amount::text AS amount, rate::text AS rate, status, \
           created_at, created_by, expires_at, confirmed_at, confirmed_by, \
           submission_id, transaction_ref \
    FROM bookings \
    WHERE branch_id = $1 AND request_id = $2";

const INSERT_BOOKING: &str = "\
    INSERT INTO bookings ( \
        id, branch_id, request_id, request_hash, name, phone, member_id, \
        type, code, amount, rate, status, created_at, created_by, \
        expires_at, confirmed_at, confirmed_by, submission_id, transaction_ref \
    ) VALUES ( \
        $1, $2, $3, $4, $5, $6, $7, \
        $8, $9, $10::numeric, $11::numeric, $12, $13, $14, \
        $15, $16, $17, $18, $19 \
    )";

const CONFIRM_BOOKING: &str = "\
    UPDATE bookings \
    SET rate = $3::numeric, expires_at = $4, confirmed_at = $5, confirmed_by = $6, status = 'CONFIRMED' \
    WHERE branch_id = $1 AND id = $2 AND status = 'PENDING'";

const CANCEL_BOOKING: &str = "\
    UPDATE bookings \
    SET status = 'CANCELLED' \
    WHERE branch_id = $1 AND id = $2 AND status IN ('PENDING', 'CONFIRMED')";

const MARK_USED_BOOKING: &str = "\
    UPDATE bookings \
    SET status = 'USED', submission_id = $3, transaction_ref = $4 \
    WHERE branch_id = $1 AND id = $2 AND status = 'CONFIRMED'";

fn map_booking(row: &sqlx::postgres::PgRow) -> AppResult<Booking> {
    let side_str: String = row.try_get("type").map_err(AppError::Database)?;
    let side = match side_str.as_str() {
        "BUY" => Side::Buy,
        "SELL" => Side::Sell,
        other => return Err(AppError::Internal(format!("invalid stored type: {other}"))),
    };

    let status_str: String = row.try_get("status").map_err(AppError::Database)?;
    let status = BookingStatus::parse(&status_str)
        .ok_or_else(|| AppError::Internal(format!("invalid stored status: {status_str}")))?;

    let code_str: String = row.try_get("code").map_err(AppError::Database)?;
    let currency = CurrencyCode::parse(code_str.trim())
        .map_err(|e| AppError::Internal(format!("invalid stored currency {code_str}: {e}")))?;

    let amount_str: String = row.try_get("amount").map_err(AppError::Database)?;
    let amount = Money::parse(&amount_str)
        .map_err(|e| AppError::Internal(format!("invalid stored amount {amount_str}: {e}")))?;

    let rate_str: String = row.try_get("rate").map_err(AppError::Database)?;
    let rate = Rate::parse(&rate_str)
        .map_err(|e| AppError::Internal(format!("invalid stored rate {rate_str}: {e}")))?;

    let branch_str: String = row.try_get("branch_id").map_err(AppError::Database)?;

    Ok(Booking {
        id: row.try_get("id").map_err(AppError::Database)?,
        branch_id: BranchId::new(branch_str),
        name: row.try_get("name").map_err(AppError::Database)?,
        phone: row.try_get("phone").map_err(AppError::Database)?,
        member_id: row.try_get("member_id").map_err(AppError::Database)?,
        side,
        currency,
        amount,
        rate,
        status,
        created_at: row.try_get("created_at").map_err(AppError::Database)?,
        created_by: row.try_get("created_by").map_err(AppError::Database)?,
        expires_at: row.try_get("expires_at").map_err(AppError::Database)?,
        confirmed_at: row.try_get("confirmed_at").map_err(AppError::Database)?,
        confirmed_by: row.try_get("confirmed_by").map_err(AppError::Database)?,
        submission_id: row.try_get("submission_id").map_err(AppError::Database)?,
        transaction_ref: row.try_get("transaction_ref").map_err(AppError::Database)?,
    })
}

impl BookingRepository for PgBookingRepository {
    fn list<'a>(
        &'a self,
        branch: &'a BranchId,
        limit: i64,
    ) -> BoxFuture<'a, AppResult<Vec<Booking>>> {
        Box::pin(async move {
            let rows = sqlx::query(LIST_BOOKINGS)
                .bind(branch.as_str())
                .bind(limit)
                .fetch_all(&self.pool)
                .await
                .map_err(AppError::Database)?;
            rows.iter().map(map_booking).collect()
        })
    }

    fn find<'a>(
        &'a self,
        branch: &'a BranchId,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<Booking>>> {
        Box::pin(async move {
            let row = sqlx::query(FIND_BOOKING)
                .bind(branch.as_str())
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(AppError::Database)?;
            row.as_ref().map(map_booking).transpose()
        })
    }

    fn find_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<Booking>>> {
        Box::pin(async move {
            let row = sqlx::query(FIND_BOOKING)
                .bind(branch.as_str())
                .bind(id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(AppError::Database)?;
            row.as_ref().map(map_booking).transpose()
        })
    }

    fn find_request_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        request_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<StoredRequest>>> {
        Box::pin(async move {
            let row = sqlx::query(FIND_REQUEST)
                .bind(branch.as_str())
                .bind(request_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(AppError::Database)?;
            row.as_ref()
                .map(|r| {
                    let booking = map_booking(r)?;
                    let hash: String = r.try_get("request_hash").map_err(AppError::Database)?;
                    Ok(StoredRequest { booking, hash })
                })
                .transpose()
        })
    }

    fn insert_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        booking: &'a Booking,
        request_id: &'a str,
        request_hash: &'a str,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let side = match booking.side {
                Side::Buy => "BUY",
                Side::Sell => "SELL",
            };
            sqlx::query(INSERT_BOOKING)
                .bind(&booking.id)
                .bind(booking.branch_id.as_str())
                .bind(request_id)
                .bind(request_hash)
                .bind(&booking.name)
                .bind(&booking.phone)
                .bind(booking.member_id.as_deref())
                .bind(side)
                .bind(booking.currency.as_str())
                .bind(booking.amount.to_string())
                .bind(booking.rate.to_string())
                .bind(booking.status.as_str())
                .bind(booking.created_at)
                .bind(&booking.created_by)
                .bind(booking.expires_at)
                .bind(booking.confirmed_at)
                .bind(booking.confirmed_by.as_deref())
                .bind(booking.submission_id.as_deref())
                .bind(booking.transaction_ref.as_deref())
                .execute(&mut **tx)
                .await
                .map_err(AppError::Database)?;
            Ok(())
        })
    }

    fn confirm_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        id: &'a str,
        rate: Rate,
        expires_at: DateTime<Utc>,
        at: DateTime<Utc>,
        by: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            let res = sqlx::query(CONFIRM_BOOKING)
                .bind(branch.as_str())
                .bind(id)
                .bind(rate.to_string())
                .bind(expires_at)
                .bind(at)
                .bind(by)
                .execute(&mut **tx)
                .await
                .map_err(AppError::Database)?;
            Ok(res.rows_affected() == 1)
        })
    }

    fn cancel_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            let res = sqlx::query(CANCEL_BOOKING)
                .bind(branch.as_str())
                .bind(id)
                .execute(&mut **tx)
                .await
                .map_err(AppError::Database)?;
            Ok(res.rows_affected() == 1)
        })
    }

    fn mark_used_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        id: &'a str,
        submission_id: &'a str,
        document_ref: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            let res = sqlx::query(MARK_USED_BOOKING)
                .bind(branch.as_str())
                .bind(id)
                .bind(submission_id)
                .bind(document_ref)
                .execute(&mut **tx)
                .await
                .map_err(AppError::Database)?;
            Ok(res.rows_affected() == 1)
        })
    }
}
