//! Marketing consent gate adapter (MK-5, design §6, §6.1, D-13).

use std::collections::HashMap;

use pos_error::AppResult;
use sqlx::{PgPool, Postgres, Transaction};
use xc_marketing::domain::engine::consent_gate::ConsentGate;

#[derive(Debug, Clone, Default)]
pub struct MarketingConsentGate {
    allowed: HashMap<String, bool>,
}

impl MarketingConsentGate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_allowed(customer: impl Into<String>, allowed: bool) -> Self {
        let mut map = HashMap::new();
        map.insert(customer.into(), allowed);
        Self { allowed: map }
    }

    pub async fn prefetch_in(
        _pool: &PgPool,
        tx: &mut Transaction<'_, Postgres>,
        customer: &str,
    ) -> AppResult<Self> {
        let eligible =
            xc_customer::application::consent_service::ConsentService::marketing_eligible_in(
                tx,
                &[customer.to_string()],
            )
            .await?;

        Ok(Self::with_allowed(customer, eligible.contains(customer)))
    }
}

impl ConsentGate for MarketingConsentGate {
    fn marketing_allowed(&self, customer: &str) -> bool {
        self.allowed.get(customer).copied().unwrap_or(false)
    }
}
