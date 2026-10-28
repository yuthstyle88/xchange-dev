//! `BindingSettingsRepository` (design `docs/design/blueprint/binding-quote.md` §5): reads an organisation's
//! `BindingQuotePolicy`.

use pos_error::AppResult;
use uuid::Uuid;

use super::binding_policy::BindingQuotePolicy;

pub trait BindingSettingsRepository: Send + Sync {
    /// `exchange_settings.binding_quote_ttl_seconds` of the organisation; no row or `NULL` ⇒ `ttl: None` (D-10).
    fn load_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        organization_id: Uuid,
    ) -> impl std::future::Future<Output = AppResult<BindingQuotePolicy>> + Send + 'a;
}
