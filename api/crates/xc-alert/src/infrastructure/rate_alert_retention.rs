//! `rate-alerts.md` retention: a deleted or expired alert and its events use data type `RATE_ALERT`. This is that
//! type's purger, behind `xc-customer`'s `RetentionPurger` port, so `RetentionJob` never writes SQL against
//! `rate_alerts`.

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::PgPool;
use uuid::Uuid;
use xc_customer::application::retention_job::RetentionPurger;
use xc_customer::domain::retention::DataType;

pub struct PgRateAlertPurger {
    pool: PgPool,
}

impl PgRateAlertPurger {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl RetentionPurger for PgRateAlertPurger {
    fn data_type(&self) -> DataType {
        DataType::RateAlert
    }

    fn purge<'a>(
        &'a self,
        org: Uuid,
        cutoff: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Pin<Box<dyn Future<Output = AppResult<u64>> + Send + 'a>> {
        let _ = (&self.pool, org, cutoff, now);
        todo!(
            "ticket 5: delete the org's alerts deleted or expired before cutoff, with their events"
        )
    }
}
