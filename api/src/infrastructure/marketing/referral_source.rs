//! Marketing referral source adapter (MK-5, design §6, §6.1, D-12).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::{PgPool, Postgres, Transaction};
use xc_marketing::domain::engine::referral_source::ReferralSource;

#[derive(Debug, Clone, Default)]
pub struct MarketingReferralSource {
    referred: HashMap<String, bool>,
}

impl MarketingReferralSource {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_referred(customer: impl Into<String>, referred: bool) -> Self {
        let mut map = HashMap::new();
        map.insert(customer.into(), referred);
        Self { referred: map }
    }

    /// Prefetches whether `customer` has an ACTIVE referral attribution captured at or before `at` (D-12).
    pub async fn prefetch(pool: &PgPool, customer: &str, at: DateTime<Utc>) -> AppResult<Self> {
        let referred: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 FROM referral_attributions \
                 WHERE customer_id = $1 AND status = 'ACTIVE' AND captured_at <= $2 \
             )",
        )
        .bind(customer)
        .bind(at)
        .fetch_one(pool)
        .await
        .map_err(pos_error::AppError::Database)?;

        Ok(Self::with_referred(customer, referred))
    }

    pub async fn prefetch_in(
        tx: &mut Transaction<'_, Postgres>,
        customer: &str,
        at: DateTime<Utc>,
    ) -> AppResult<Self> {
        let referred: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 FROM referral_attributions \
                 WHERE customer_id = $1 AND status = 'ACTIVE' AND captured_at <= $2 \
             )",
        )
        .bind(customer)
        .bind(at)
        .fetch_one(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        Ok(Self::with_referred(customer, referred))
    }
}

impl ReferralSource for MarketingReferralSource {
    fn referred(&self, customer: &str, _at: DateTime<Utc>) -> bool {
        self.referred.get(customer).copied().unwrap_or(false)
    }
}
