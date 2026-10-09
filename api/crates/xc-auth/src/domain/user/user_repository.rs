//! User repository trait.

use std::future::Future;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;

use super::invite::Invite;
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

    fn create_invite(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        invite: &Invite,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn find_invite_by_user_id(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: &str,
    ) -> impl Future<Output = AppResult<Option<Invite>>> + Send;

    fn find_invite_by_code_hash(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        code_hash: &str,
    ) -> impl Future<Output = AppResult<Option<Invite>>> + Send;

    fn mark_invite_used(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn spend_invite(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        code_hash: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Option<String>>> + Send;

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
}
