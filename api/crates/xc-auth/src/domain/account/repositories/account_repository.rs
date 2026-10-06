//! Account repository trait.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use std::future::Future;

use crate::domain::account::entities::employee::Employee;
use crate::domain::account::entities::owner::Owner;

/// How many rows a purge removed, by kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Purged {
    pub owner_sessions: u64,
    pub employee_sessions: u64,
    /// Rows of `auth_attempts`: one per key and 15-minute window.
    pub attempt_windows: u64,
}

pub trait AccountRepository: Send + Sync + 'static {
    fn find_owner_by_username(
        &self,
        username: &str,
    ) -> impl Future<Output = AppResult<Option<(Owner, String)>>> + Send;

    fn find_owner_session(
        &self,
        token_hash: &str,
    ) -> impl Future<Output = AppResult<Option<(Owner, i32, DateTime<Utc>)>>> + Send;

    fn create_owner_session(
        &self,
        token_hash: &str,
        owner_id: &str,
        version: i32,
        expires: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn delete_owner_session(&self, token_hash: &str) -> impl Future<Output = AppResult<()>> + Send;

    fn find_employee_by_username(
        &self,
        username: &str,
    ) -> impl Future<Output = AppResult<Option<(Employee, String)>>> + Send;

    fn find_employee_by_id(
        &self,
        id: &str,
    ) -> impl Future<Output = AppResult<Option<Employee>>> + Send;

    fn find_employee_session(
        &self,
        token_hash: &str,
    ) -> impl Future<Output = AppResult<Option<(Employee, i32, DateTime<Utc>)>>> + Send;

    fn create_employee_session(
        &self,
        token_hash: &str,
        employee_id: &str,
        version: i32,
        expires: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn delete_employee_session(
        &self,
        token_hash: &str,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn list_employees(&self) -> impl Future<Output = AppResult<Vec<Employee>>> + Send;

    fn create_employee(
        &self,
        employee: &Employee,
        password_hash: &str,
        actor: &str,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn reset_employee_password(
        &self,
        id: &str,
        password_hash: &str,
        now: DateTime<Utc>,
        actor: &str,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn disable_employee(
        &self,
        id: &str,
        now: DateTime<Utc>,
        actor: &str,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn change_employee_password(
        &self,
        id: &str,
        expected_version: i32,
        password_hash: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    fn active_employees(
        &self,
        branch_id: &str,
    ) -> impl Future<Output = AppResult<Vec<Employee>>> + Send;

    fn create_owner(
        &self,
        id: &str,
        username: &str,
        name: &str,
        password_hash: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Owner>> + Send;

    fn record_employee_login_audit(
        &self,
        employee_id: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Deletes, in one transaction, what can no longer matter: the sessions of
    /// the owner and of employees whose `expires` is before `now`, and the
    /// attempt rows whose window started before `current_window` (the start
    /// of the window `now` falls in). Nothing else is deleted.
    fn purge_expired(
        &self,
        now: DateTime<Utc>,
        current_window: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Purged>> + Send;

    fn increment_attempt(
        &self,
        key: &str,
        window_start: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<i32>> + Send;
}
