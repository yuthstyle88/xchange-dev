//! `PgCustodyHolds`: the cash guard's Postgres adapter (design §3.2).

use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use uuid::Uuid;
use xc_auth::domain::user::CustodyHolds;

pub struct PgCustodyHolds;

impl CustodyHolds for PgCustodyHolds {
    fn holds_cash_or_pending_handover<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            let Ok(uid) = Uuid::parse_str(user_id) else {
                return Ok(false);
            };
            let holds: bool = sqlx::query_scalar(
                r#"
                SELECT EXISTS (
                    SELECT 1 FROM shifts
                    WHERE teller_id = $1 AND status = 'OPEN'
                ) OR EXISTS (
                    SELECT 1 FROM handovers h
                    JOIN shifts s ON s.id = h.from_shift_id
                    WHERE h.status = 'PENDING' AND s.teller_id = $1
                ) OR EXISTS (
                    SELECT 1 FROM handovers
                    WHERE status = 'PENDING' AND receiver_id = $1
                ) OR EXISTS (
                    SELECT 1 FROM cash_transfers
                    WHERE status = 'DISPATCHED' AND dispatched_by = $1
                )
                "#,
            )
            .bind(uid)
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            Ok(holds)
        })
    }
}
