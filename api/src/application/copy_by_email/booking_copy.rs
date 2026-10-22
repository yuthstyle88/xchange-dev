//! `BookingCopy` (design `kiosk-shared-phone.md` §4.2 D21): the booking subject of the e-mail copy.
//! One read of `bookings` (+ its slot and branch) by member and `qr_token_hash`.

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Row};
use xc_customer::domain::sms_sender::BoxFuture;

use super::subject::{CopyDetails, CopySubject, CopySubjectKind};

#[derive(Clone)]
pub struct BookingCopy {
    pool: PgPool,
}

impl BookingCopy {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn load(
        &self,
        member: Option<&str>,
        id: &str,
        qr_hash: &[u8; 32],
    ) -> AppResult<Option<CopyDetails>> {
        let row = sqlx::query(
            "SELECT b.id, br.name, b.status, b.booking_no, s.service_date, \
                    s.start_at, s.end_at, br.timezone, \
                    btrim(b.requested_currency), b.requested_quantity::text \
             FROM bookings b \
             JOIN booking_slots s ON s.id = b.slot_id \
             JOIN branches br ON br.id = b.branch_id \
             WHERE b.id = $1 \
               AND b.qr_token_hash = $2 \
               AND ($3::text IS NULL OR b.customer_id = $3)",
        )
        .bind(id)
        .bind(&qr_hash[..])
        .bind(member)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let booking_id: String = row.get(0);
        let branch_name: String = row.get(1);
        let status: String = row.get(2);
        let booking_no: String = row.get(3);
        let service_date: NaiveDate = row.get(4);
        let start_at: DateTime<Utc> = row.get(5);
        let end_at: DateTime<Utc> = row.get(6);
        let timezone: String = row.get(7);
        let currency: String = row.get(8);
        let amount: String = row.get(9);

        let tz: Tz = timezone
            .parse()
            .map_err(|_| AppError::Internal(format!("invalid branch timezone: {timezone}")))?;
        let slot = format!(
            "{}–{}",
            start_at.with_timezone(&tz).format("%H:%M"),
            end_at.with_timezone(&tz).format("%H:%M"),
        );

        Ok(Some(CopyDetails {
            kind: CopySubjectKind::Booking,
            id: booking_id,
            branch_name,
            rows: vec![
                ("bookingNo".to_owned(), booking_no),
                (
                    "date".to_owned(),
                    service_date.format("%Y-%m-%d").to_string(),
                ),
                ("slot".to_owned(), slot),
                ("currency".to_owned(), currency),
                ("amount".to_owned(), amount),
            ],
            status,
        }))
    }
}

impl CopySubject for BookingCopy {
    fn kind(&self) -> CopySubjectKind {
        CopySubjectKind::Booking
    }

    /// `bookings` row `id` with `customer_id = member` and `qr_token_hash = qr_hash` (bytea, the
    /// raw SHA-256). Rows (`[proposed]`, in order): `bookingNo` (`booking_no`), `date` (the slot's
    /// `service_date`, `YYYY-MM-DD`), `slot` (`HH:MM–HH:MM` in the branch's timezone), `currency`
    /// (`requested_currency`), `amount` (`requested_quantity`, as stored); `status` = the booking's
    /// status; `branch_name` = `branches.name`.
    fn load_for<'a>(
        &'a self,
        member: &'a str,
        id: &'a str,
        qr_hash: &'a [u8; 32],
    ) -> BoxFuture<'a, AppResult<Option<CopyDetails>>> {
        Box::pin(async move { self.load(Some(member), id, qr_hash).await })
    }

    /// The same row by `id` and `qr_token_hash` only.
    fn load_by_link<'a>(
        &'a self,
        id: &'a str,
        qr_hash: &'a [u8; 32],
    ) -> BoxFuture<'a, AppResult<Option<CopyDetails>>> {
        Box::pin(async move { self.load(None, id, qr_hash).await })
    }
}
