//! `rate-alerts.md` retention: a deleted or expired alert and its events use data type `RATE_ALERT`. This is that
//! type's purger, behind `xc-customer`'s `RetentionPurger` port, so `RetentionJob` never writes SQL against
//! `rate_alerts`.

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
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
        _now: DateTime<Utc>,
    ) -> Pin<Box<dyn Future<Output = AppResult<u64>> + Send + 'a>> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await.map_err(AppError::Database)?;

            sqlx::query(
                "DELETE FROM rate_alert_events \
                 WHERE alert_id IN ( \
                     SELECT id FROM rate_alerts \
                     WHERE organization_id = $1 \
                       AND (deleted_at < $2 OR expires_at < $2) \
                 )",
            )
            .bind(org)
            .bind(cutoff)
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;

            let result = sqlx::query(
                "DELETE FROM rate_alerts \
                 WHERE organization_id = $1 \
                   AND (deleted_at < $2 OR expires_at < $2)",
            )
            .bind(org)
            .bind(cutoff)
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;

            tx.commit().await.map_err(AppError::Database)?;

            Ok(result.rows_affected())
        })
    }
}
