//! #5557: the SQL form of `BranchScope::covers_user`, for the paged reads that filter in the database
//! (`admin_users`, `admin_employee_register`, `UserRepository::list_audit` / `list_logins`).

use chrono::{DateTime, Utc};
use sqlx::{Postgres, QueryBuilder};

use crate::domain::access::BranchScope;

/// Appends ` AND <the user is in scope>` for the user whose `users.id` (a uuid) is the SQL expression `user_id`.
/// `All` appends nothing. `Branches` keeps a user with at least one live `user_roles` row at `now`, every one of
/// them on a listed branch; a NULL `user_id` (no such user) is never kept. The same rule as
/// `BranchScope::covers_user`.
pub fn push_user_in_scope(
    builder: &mut QueryBuilder<Postgres>,
    scope: &BranchScope,
    user_id: &str,
    now: DateTime<Utc>,
) {
    // Skeleton (#5557): T2 writes the body. Today's answer: no filter.
    let _ = (builder, scope, user_id, now);
}
