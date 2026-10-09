//! PgSettingsRepository: PostgreSQL implementation of SettingsRepository.

use pos_error::AppResult;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::settings::repository::SettingsRepository;

pub struct PgSettingsRepository {
    #[allow(dead_code)]
    pool: PgPool,
}

impl PgSettingsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl SettingsRepository for PgSettingsRepository {
    async fn get_ttl_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
    ) -> AppResult<i32> {
        let row = sqlx::query_scalar::<_, i32>(
            "SELECT quote_ttl_seconds FROM exchange_settings WHERE organization_id = $1",
        )
        .bind(organization_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        Ok(row.unwrap_or(300))
    }
}
