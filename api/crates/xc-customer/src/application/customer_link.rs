//! The link from a v3 user to its customer (#654 CI-7; design `customer-identity-consent.md` K7,
//! `customers.user_id UNIQUE`). Read by the session helper `resolve_session_actor` in the `api` crate.

use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;

/// The id of the customer whose `user_id` is `user_id`, or `None` when that user has no customer.
pub async fn customer_of_user(pool: &PgPool, user_id: &str) -> AppResult<Option<String>> {
    // A legacy (v2) user id is no UUID and has no customer.
    let Ok(user) = Uuid::parse_str(user_id) else {
        return Ok(None);
    };
    sqlx::query_scalar("SELECT id FROM customers WHERE user_id = $1")
        .bind(user)
        .fetch_optional(pool)
        .await
        .map_err(AppError::Database)
}
