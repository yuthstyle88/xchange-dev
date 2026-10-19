//! Who a user id is, for screens and audit text (OR-7.m #1027).
//!
//! Replaces the reads of `employees.username` and `owners.username` that password login left
//! behind: a name now comes from `users.name`, by `users.id`.

use std::collections::HashMap;

use pos_error::AppResult;
use sqlx::PgExecutor;

/// `users.name` of each id in `ids` that is a `users.id`, keyed by that id (as text).
///
/// - An id that is not a `users.id` (an old employee id, `"owner"`, a username, not a UUID) is
///   absent from the answer, never an error.
/// - Repeated ids answer once. An empty `ids` answers an empty map without querying.
/// - A user of any status answers (names on old rows stay readable after a user is disabled).
///
/// `executor` is a pool or a transaction (`&mut *tx`), so a caller inside a transaction reads through it.
pub async fn user_names<'e, E: PgExecutor<'e>>(
    executor: E,
    ids: &[String],
) -> AppResult<HashMap<String, String>> {
    let _ = (executor, ids);
    todo!("OR-7.m T1: SELECT id::text, name FROM users WHERE id::text = ANY($1)")
}
