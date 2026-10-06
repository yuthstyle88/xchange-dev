//! Branch repository trait.

use std::future::Future;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use crate::domain::branch::entities::branch::Branch;

pub trait BranchRepository: Send + Sync {
    fn list(&self) -> impl Future<Output = AppResult<Vec<Branch>>> + Send;

    fn find_by_id(&self, id: &BranchId) -> impl Future<Output = AppResult<Option<Branch>>> + Send;

    fn branch_exists(&self, id: &BranchId) -> impl Future<Output = AppResult<bool>> + Send;

    fn find_by_request(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        request_id: &str,
    ) -> impl Future<Output = AppResult<Option<Branch>>> + Send;

    fn is_abandoned(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch_id: &str,
        request_id: &str,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    fn name_taken(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        name: &str,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    fn count(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> impl Future<Output = AppResult<i64>> + Send;

    fn insert(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: &str,
        name: &str,
        created_at: DateTime<Utc>,
        created_by: &str,
        request_id: &str,
    ) -> impl Future<Output = AppResult<()>> + Send;
}
