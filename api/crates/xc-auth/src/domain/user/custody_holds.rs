//! Port: does a user hold cash or a pending handover (design §3.2 cash guard). C3 writes the real query.

use futures::future::BoxFuture;
use pos_error::AppResult;

pub trait CustodyHolds: Send + Sync {
    fn holds_cash_or_pending_handover<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>>;
}
