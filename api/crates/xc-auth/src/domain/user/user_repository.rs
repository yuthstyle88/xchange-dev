//! User repository trait.

use std::future::Future;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;

use super::user::User;
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
}
