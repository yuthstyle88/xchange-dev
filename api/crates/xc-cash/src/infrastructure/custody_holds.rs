//! `PgCustodyHolds`: the cash guard's Postgres adapter (design §3.2).

use futures::future::BoxFuture;
use pos_error::AppResult;
use xc_auth::domain::user::CustodyHolds;

pub struct PgCustodyHolds;

impl CustodyHolds for PgCustodyHolds {
    fn holds_cash_or_pending_handover<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        let _ = (tx, user_id);
        // C3 replaces this body (design §3.2).
        Box::pin(async { Ok(false) })
    }
}
