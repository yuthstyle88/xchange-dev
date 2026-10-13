//! Retention background job (design `customer-identity-consent.md` §4.1, §4.3).

use chrono::{DateTime, Duration, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::consent::ConsentError;
use crate::domain::retention::DataType;

#[derive(Clone)]
pub struct RetentionJob {
    pool: PgPool,
}

impl RetentionJob {
    pub fn pg(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn set_period(
        &self,
        org: &str,
        set_by: Option<&str>,
        data_type: DataType,
        period_days: Option<i32>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;
        Self::set_period_in(&mut tx, org, set_by, data_type, period_days, now).await?;
        tx.commit().await.map_err(AppError::Database)?;
        Ok(())
    }

    pub async fn set_period_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        org: &str,
        set_by: Option<&str>,
        data_type: DataType,
        period_days: Option<i32>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        if period_days.is_some_and(|p| p <= 0) {
            return Err(ConsentError::InvalidPeriod.into());
        }

        let org_id = Uuid::parse_str(org).map_err(|e| AppError::Domain(e.to_string()))?;
        let set_by_id = set_by
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|e| AppError::Domain(e.to_string()))?;

        sqlx::query(
            "INSERT INTO retention_policies (organization_id, data_type, period_days, set_by, set_at) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (organization_id, data_type) DO UPDATE SET \
                 period_days = EXCLUDED.period_days, \
                 set_by = EXCLUDED.set_by, \
                 set_at = EXCLUDED.set_at",
        )
        .bind(org_id)
        .bind(data_type.as_str())
        .bind(period_days)
        .bind(set_by_id)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }

    pub async fn purge(&self, now: DateTime<Utc>) -> AppResult<u64> {
        let policies: Vec<(Uuid, String, i32)> = sqlx::query_as(
            "SELECT organization_id, data_type, period_days FROM retention_policies \
             WHERE period_days IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let mut total_deleted: u64 = 0;

        for (org_id, data_type, period_days) in policies {
            let cutoff = now - Duration::days(period_days as i64);

            match data_type.as_str() {
                "IDENTITY_VALUE" => {
                    let result = sqlx::query(
                        "DELETE FROM customer_identities \
                         WHERE organization_id = $1 \
                           AND status = 'REVOKED' \
                           AND type IN ('PASSPORT', 'NATIONAL_ID') \
                           AND updated_at < $2",
                    )
                    .bind(org_id)
                    .bind(cutoff)
                    .execute(&self.pool)
                    .await
                    .map_err(AppError::Database)?;

                    total_deleted += result.rows_affected();
                }
                "CONTACT" => {
                    let result = sqlx::query(
                        "DELETE FROM customer_identities \
                         WHERE organization_id = $1 \
                           AND status = 'REVOKED' \
                           AND type IN ('PHONE', 'EMAIL') \
                           AND updated_at < $2",
                    )
                    .bind(org_id)
                    .bind(cutoff)
                    .execute(&self.pool)
                    .await
                    .map_err(AppError::Database)?;

                    total_deleted += result.rows_affected();
                }
                "ASSISTANT_CONVERSATION" => {
                    sqlx::query(
                        "DELETE FROM assistant_messages \
                         WHERE conversation_id IN ( \
                             SELECT id FROM assistant_conversations \
                             WHERE organization_id = $1 \
                               AND (expires_at <= $2 OR (expires_at IS NULL AND created_at < $3)) \
                         )",
                    )
                    .bind(org_id)
                    .bind(now)
                    .bind(cutoff)
                    .execute(&self.pool)
                    .await
                    .map_err(AppError::Database)?;

                    let result = sqlx::query(
                        "DELETE FROM assistant_conversations \
                         WHERE organization_id = $1 \
                           AND (expires_at <= $2 OR (expires_at IS NULL AND created_at < $3))",
                    )
                    .bind(org_id)
                    .bind(now)
                    .bind(cutoff)
                    .execute(&self.pool)
                    .await
                    .map_err(AppError::Database)?;

                    total_deleted += result.rows_affected();
                }
                _ => {
                    // CLOSED_CUSTOMER is ruled out of CI-6
                }
            }
        }

        Ok(total_deleted)
    }
}
