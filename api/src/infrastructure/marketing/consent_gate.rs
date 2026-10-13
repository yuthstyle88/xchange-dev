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

    /// Prefetches marketing consent for `customer` using `ConsentService::marketing_allowed`
    /// (async, before the evaluator runs, D-13).
    pub async fn prefetch(pool: &PgPool, customer: &str) -> AppResult<Self> {
        let svc = xc_customer::application::consent_service::ConsentService::pg(pool.clone());
        let allowed = svc
            .marketing_allowed(customer, xc_customer::domain::consent::Channel::Sms)
            .await?
            || svc
                .marketing_allowed(customer, xc_customer::domain::consent::Channel::Email)
                .await?
            || svc
                .marketing_allowed(customer, xc_customer::domain::consent::Channel::Push)
                .await?;

        Ok(Self::with_allowed(customer, allowed))
    }

    pub async fn prefetch_in(
        pool: &PgPool,
        tx: &mut Transaction<'_, Postgres>,
        customer: &str,
    ) -> AppResult<Self> {
        let svc = xc_customer::application::consent_service::ConsentService::pg(pool.clone());
        let allowed = svc
            .marketing_allowed_in(tx, customer, xc_customer::domain::consent::Channel::Sms)
            .await?
            || svc
                .marketing_allowed_in(tx, customer, xc_customer::domain::consent::Channel::Email)
                .await?
            || svc
                .marketing_allowed_in(tx, customer, xc_customer::domain::consent::Channel::Push)
                .await?;

        Ok(Self::with_allowed(customer, allowed))
    }
}

impl ConsentGate for MarketingConsentGate {
    fn marketing_allowed(&self, customer: &str) -> bool {
        self.allowed.get(customer).copied().unwrap_or(false)
    }
}
