//! `BookingCopy` (design `kiosk-shared-phone.md` §4.2 D21): the booking subject of the e-mail copy.
//! One read of `bookings` (+ its slot and branch) by member and `qr_token_hash`.

use pos_error::AppResult;
use sqlx::PgPool;
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
        let _ = (&self.pool, member, id, qr_hash);
        Box::pin(async move { todo!("KWQ-D5: the member's booking by QR hash") })
    }

    /// The same row by `id` and `qr_token_hash` only.
    fn load_by_link<'a>(
        &'a self,
        id: &'a str,
        qr_hash: &'a [u8; 32],
    ) -> BoxFuture<'a, AppResult<Option<CopyDetails>>> {
        let _ = (&self.pool, id, qr_hash);
        Box::pin(async move { todo!("KWQ-D5: the booking by QR hash") })
    }
}
