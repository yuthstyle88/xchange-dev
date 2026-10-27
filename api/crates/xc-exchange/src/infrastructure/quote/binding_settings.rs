//! `PgBindingSettings` (design `docs/design/blueprint/binding-quote.md` §5, D-10).

use pos_error::AppResult;
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
        let _ = (tx, organization_id);
        todo!("BQ-1")
    }
}
