//! Outbox handler registration for `loyalty.exchange_reversed`.

use std::sync::Arc;

use xc_loyalty::domain::events::LOYALTY_REVERSED;
use xc_loyalty::infrastructure::exchange_handler::LoyaltyExchangeHandler;
use xc_loyalty::infrastructure::http_client::LoyaltyHttp;
use xc_outbox::domain::handler::OutboxHandler;

use crate::bootstrap::AppState;
use crate::infrastructure::loyalty::pg_facts_source::PgLoyaltyFactsSource;

/// `None` when Loyalty is not configured; then the rows wait and expire after 7 days.
pub fn handler(state: &AppState) -> Option<Arc<dyn OutboxHandler>> {
    let settings = state.settings.loyalty.as_ref()?;
    let config = settings.clone();
    let http = Arc::new(LoyaltyHttp::new(&config));
    let facts = Arc::new(PgLoyaltyFactsSource::new(state.db.clone()));
    Some(Arc::new(LoyaltyExchangeHandler::new(
        &LOYALTY_REVERSED,
        config,
        http,
        facts.clone(),
        facts,
    )))
}
