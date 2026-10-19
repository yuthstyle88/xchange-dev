//! #2948: the one test writer of `users` + `user_roles` rows.
//!
//! `user_roles.valid_from` defaults to `now()` (`api/migrations/20261011110000_users_roles.sql:69`).
//! A test that leaves it out inherits the wall clock, and a check at a fixed test clock then sees
//! "no valid role". Every row this module writes names `valid_from` and `valid_to` explicitly,
//! from the same values the caller puts in its in-memory roles, so the two cannot disagree.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;

/// One `user_roles` row: the role, its branch (`None` = organisation-wide) and its window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleRow<'a> {
    /// A `roles.code`, e.g. `"TELLER"`.
    pub role_code: &'a str,
    /// A `branches.id` of the same organisation, or `None`.
    pub branch_id: Option<&'a str>,
    /// Written as given. Never left to the column's `DEFAULT now()`.
    pub valid_from: DateTime<Utc>,
    /// Written as given; `None` is SQL `NULL` (no end).
    pub valid_to: Option<DateTime<Utc>>,
}

/// Inserts one `users` row (`organization_id = org`, `name`, `status = 'ACTIVE'`,
/// `created_by = 'test'`), then one `user_roles` row per entry of `roles` through
/// [`seed_roles`]. Returns the new user's id as text.
pub async fn seed_person(
    conn: &mut PgConnection,
    org: &str,
    name: &str,
    roles: &[RoleRow<'_>],
) -> anyhow::Result<String> {
    let id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(org)
    .bind(name)
    .fetch_one(&mut *conn)
    .await?;
    seed_roles(conn, &id, org, roles).await?;
    Ok(id)
}

/// Inserts one `user_roles` row per entry of `roles` for an existing user, each with
/// `valid_from` and `valid_to` written explicitly.
pub async fn seed_roles(
    conn: &mut PgConnection,
    user_id: &str,
    org: &str,
    roles: &[RoleRow<'_>],
) -> anyhow::Result<()> {
    for role in roles {
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from, valid_to) \
             VALUES ($1::uuid, $2, $3::uuid, $4, $5, $6)",
        )
        .bind(user_id)
        .bind(role.role_code)
        .bind(org)
        .bind(role.branch_id)
        .bind(role.valid_from)
        .bind(role.valid_to)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}
