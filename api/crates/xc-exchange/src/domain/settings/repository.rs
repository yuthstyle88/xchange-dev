//! Settings repository trait.

use pos_error::AppResult;
use uuid::Uuid;

pub trait SettingsRepository: Send + Sync {
    fn get_ttl_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        organization_id: Uuid,
    ) -> impl std::future::Future<Output = AppResult<i32>> + Send + 'a;
}
