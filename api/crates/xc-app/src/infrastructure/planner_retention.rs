//! P13 (`customer-app.md` P13): the `PLANNER_DRAFT` data type's purger, behind `xc-customer`'s `RetentionPurger`
//! port, so `RetentionJob` never writes SQL against `planner_drafts`.

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;
use xc_customer::application::retention_job::RetentionPurger;
use xc_customer::domain::retention::DataType;

pub struct PgPlannerDraftPurger {
    pool: PgPool,
}

impl PgPlannerDraftPurger {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl RetentionPurger for PgPlannerDraftPurger {
    fn data_type(&self) -> DataType {
        DataType::PlannerDraft
    }

    fn purge<'a>(
        &'a self,
        org: Uuid,
        cutoff: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Pin<Box<dyn Future<Output = AppResult<u64>> + Send + 'a>> {
        Box::pin(async move {
            let result = sqlx::query(
                "DELETE FROM planner_drafts \
                 WHERE organization_id = $1 AND (expires_at <= $2 OR created_at < $3)",
            )
            .bind(org)
            .bind(now)
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;

            Ok(result.rows_affected())
        })
    }
}
