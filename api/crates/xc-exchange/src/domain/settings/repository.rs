//! Settings repository trait.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Money;

pub trait SettingsRepository: Send + Sync {
    fn get_ttl_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        organization_id: Uuid,
    ) -> impl std::future::Future<Output = AppResult<i32>> + Send + 'a;

    fn approval_limit_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        organization_id: Uuid,
        branch: &'a BranchId,
    ) -> impl std::future::Future<Output = AppResult<Option<Money>>> + Send + 'a;

    fn set_ttl_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        organization_id: Uuid,
        seconds: i32,
        by: &'a str,
        now: DateTime<Utc>,
    ) -> impl std::future::Future<Output = AppResult<()>> + Send + 'a;

    #[allow(clippy::too_many_arguments)]
    fn set_approval_limit_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        organization_id: Uuid,
        branch: &'a BranchId,
        limit: Option<Money>,
        by: &'a str,
        now: DateTime<Utc>,
    ) -> impl std::future::Future<Output = AppResult<()>> + Send + 'a;
}
