//! RBAC §11.4: where groups and their sets are kept (`roles`, `role_permission_grants`, `role_grant_version`).
//!
//! Every write bumps `role_grant_version` once, in the caller's transaction, so `read_session_in` sees it.
//! The store does not authorise and does not run the guardrails; F12's commands do both, and write the audit row.
use std::future::Future;

use sqlx::{PgConnection, Postgres, Transaction};

use super::permission::Permission;
use super::role_grants::{Group, RoleGrantsSnapshot};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RoleGrantStoreError {
    #[error("no group {0}")]
    NotFound(String),
    #[error("a group {0} already exists")]
    CodeTaken(String),
    #[error("group {0} is built in")]
    BuiltIn(String),
    #[error("group {0} is held by a user")]
    InUse(String),
    #[error("database: {0}")]
    Database(String),
}

pub trait RoleGrantStore: Send + Sync + 'static {
    /// Every group and its set, at the current `role_grant_version`.
    fn load(
        &self,
        conn: &mut PgConnection,
    ) -> impl Future<Output = Result<RoleGrantsSnapshot, RoleGrantStoreError>> + Send;

    /// The single `role_grant_version` row.
    fn version(
        &self,
        conn: &mut PgConnection,
    ) -> impl Future<Output = Result<i64, RoleGrantStoreError>> + Send;

    /// §11.5 item 2: for every built-in group with **no** `role_permission_grants` rows, insert
    /// `RoleGrants::seed` of it; bump the version once if anything was inserted. Answers how many groups it seeded.
    fn seed_missing(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> impl Future<Output = Result<u64, RoleGrantStoreError>> + Send;

    /// Replaces `code`'s set. `NotFound` for an unknown code.
    fn save_set(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        code: &str,
        set: &[Permission],
    ) -> impl Future<Output = Result<(), RoleGrantStoreError>> + Send;

    /// Adds a custom group (`built_in` must be false) with `set`. `CodeTaken` when the code exists.
    fn add_group(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        group: &Group,
        set: &[Permission],
    ) -> impl Future<Output = Result<(), RoleGrantStoreError>> + Send;

    /// Deletes a custom group and its set. `BuiltIn` for a built-in group, `InUse` while any `user_roles` row names it,
    /// `NotFound` for an unknown code.
    fn delete_group(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        code: &str,
    ) -> impl Future<Output = Result<(), RoleGrantStoreError>> + Send;
}
