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
    let BranchScope::Branches(list) = scope else {
        return;
    };
    let branches: Vec<String> = list.iter().map(|b| b.as_str().to_owned()).collect();
    builder.push(" AND EXISTS (SELECT 1 FROM user_roles cr WHERE cr.user_id = ");
    builder.push(user_id);
    builder.push(" AND cr.valid_from <= ");
    builder.push_bind(now);
    builder.push(" AND (cr.valid_to IS NULL OR cr.valid_to > ");
    builder.push_bind(now);
    builder.push(")) AND NOT EXISTS (SELECT 1 FROM user_roles cr WHERE cr.user_id = ");
    builder.push(user_id);
    builder.push(" AND cr.valid_from <= ");
    builder.push_bind(now);
    builder.push(" AND (cr.valid_to IS NULL OR cr.valid_to > ");
    builder.push_bind(now);
    builder.push(") AND (cr.branch_id IS NULL OR NOT (cr.branch_id = ANY(");
    builder.push_bind(branches);
    builder.push("))))");
}
