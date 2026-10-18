//! User repository trait.

use std::future::Future;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;

use super::user::User;
use super::user_history::{LoginEntry, UserAuditEntry};
use super::user_profile::UserProfile;
use super::user_status::UserStatus;
use crate::domain::access::{Role, ScopedRole};

pub trait UserRepository: Send + Sync + 'static {
    fn find_by_id(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: &str,
    ) -> impl Future<Output = AppResult<Option<User>>> + Send;

    fn find_by_auth_subject(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        subject: &str,
    ) -> impl Future<Output = AppResult<Option<User>>> + Send;

    fn create_user(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user: &User,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn set_status(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: &str,
        status: UserStatus,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn set_auth_subject(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: &str,
        auth_subject: &str,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn bump_version(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: &str,
    ) -> impl Future<Output = AppResult<i32>> + Send;

    fn add_role(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: &str,
        organization_id: &str,
        role: &ScopedRole,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn revoke_role(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: &str,
        role: Role,
        branch: Option<&BranchId>,
        valid_to: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn find_roles_by_user_id(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: &str,
    ) -> impl Future<Output = AppResult<Vec<ScopedRole>>> + Send;

    /// Appends one `user_audit` row: who did what to whom (#1157: a create, and the Identity id
    /// of a register whose bind failed, so it is never lost).
    fn append_audit(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor_id: &str,
        subject_id: &str,
        action: &str,
        organization_id: &str,
    ) -> impl Future<Output = AppResult<()>> + Send;
    /// ACTIVE users of the organisation holding live organisation-wide EXECUTIVE or ADMIN (design §3.1 rule 6).
    fn count_active_owners(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        organization_id: &str,
    ) -> impl Future<Output = AppResult<usize>> + Send;

    /// The user's four profile fields (C1c); `None` when no such user.
    fn find_profile(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: &str,
    ) -> impl Future<Output = AppResult<Option<UserProfile>>> + Send;

    /// Writes the four profile fields (C1c). Empty strings are stored as NULL. A duplicate
    /// `employee_code` in the organisation (case-insensitive UNIQUE, §4) is
    /// `AppError::Conflict("employeeCodeTaken")`.
    fn set_profile(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: &str,
        profile: &UserProfile,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// `user_audit` rows of the organisation, newest first (`at DESC, id DESC`); `before` is the
    /// last row's `id` of the previous page (C1c, read by F4a).
    fn list_audit(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        organization_id: &str,
        before: Option<&str>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<UserAuditEntry>>> + Send;

    /// `login_history` rows of the organisation, newest first (`at DESC, id DESC`); `before` is
    /// the last row's `id` of the previous page (C1c, read by F4b).
    fn list_logins(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        organization_id: &str,
        before: Option<&str>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<LoginEntry>>> + Send;
}
