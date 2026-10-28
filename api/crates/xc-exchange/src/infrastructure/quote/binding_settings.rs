//! `PgBindingSettings` (design `docs/design/blueprint/binding-quote.md` §5, D-10).

use chrono::Duration;
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::quote::binding_policy::BindingQuotePolicy;
use crate::domain::quote::binding_settings::BindingSettingsRepository;

#[derive(Debug, Clone, Copy, Default)]
pub struct PgBindingSettings;

impl PgBindingSettings {
    pub fn new() -> Self {
        Self
    }
}

impl BindingSettingsRepository for PgBindingSettings {
    async fn load_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
    ) -> AppResult<BindingQuotePolicy> {
        let ttl_seconds: Option<Option<i32>> = sqlx::query_scalar(
            "SELECT binding_quote_ttl_seconds FROM exchange_settings WHERE organization_id = $1",
        )
        .bind(organization_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(BindingQuotePolicy {
            ttl: ttl_seconds
                .flatten()
                .map(|n| Duration::seconds(i64::from(n))),
        })
    }
}
