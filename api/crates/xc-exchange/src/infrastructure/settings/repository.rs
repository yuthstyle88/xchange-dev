//! PgSettingsRepository: PostgreSQL implementation of SettingsRepository.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Money;

use crate::domain::settings::repository::SettingsRepository;

pub struct PgSettingsRepository {
    #[allow(dead_code)]
    pool: PgPool,
}

impl PgSettingsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// §3.6: the quote lifetime is 30–3600 s (the table's CHECK).
pub const MIN_TTL_SECONDS: i32 = 30;
pub const MAX_TTL_SECONDS: i32 = 3600;

impl SettingsRepository for PgSettingsRepository {
    async fn get_ttl_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
    ) -> AppResult<i32> {
        let row = sqlx::query_scalar::<_, i32>(
            "SELECT quote_ttl_seconds FROM exchange_settings WHERE organization_id = $1",
        )
        .bind(organization_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        Ok(row.unwrap_or(300))
    }

    async fn approval_limit_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        branch: &'a BranchId,
    ) -> AppResult<Option<Money>> {
        let row = sqlx::query_scalar::<_, Option<String>>(
            "SELECT approval_limit_thb::text FROM branch_exchange_limits \
             WHERE organization_id = $1 AND branch_id = $2",
        )
        .bind(organization_id)
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        match row.flatten() {
            None => Ok(None),
            Some(text) => Money::parse(&text)
                .map(Some)
                .map_err(|e| AppError::Internal(format!("approval_limit_thb: {e}"))),
        }
    }

    async fn set_ttl_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        seconds: i32,
        by: &'a str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        if !(MIN_TTL_SECONDS..=MAX_TTL_SECONDS).contains(&seconds) {
            return Err(AppError::Domain(format!(
                "validationFailed: quoteTtlSeconds must be {MIN_TTL_SECONDS}–{MAX_TTL_SECONDS}"
            )));
        }
        sqlx::query(
            "INSERT INTO exchange_settings (organization_id, quote_ttl_seconds, updated_by, updated_at) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (organization_id) DO UPDATE \
             SET quote_ttl_seconds = EXCLUDED.quote_ttl_seconds, \
                 updated_by = EXCLUDED.updated_by, updated_at = EXCLUDED.updated_at",
        )
        .bind(organization_id)
        .bind(seconds)
        .bind(by)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    async fn set_approval_limit_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        branch: &'a BranchId,
        limit: Option<Money>,
        by: &'a str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        if let Some(l) = limit
            && l.try_minor().map_err(|_| {
                AppError::Domain(
                    "validationFailed: approvalLimitThb must be greater than zero".to_owned(),
                )
            })? <= 0
        {
            return Err(AppError::Domain(
                "validationFailed: approvalLimitThb must be greater than zero".to_owned(),
            ));
        }
        let owned: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM branches WHERE organization_id = $1 AND id = $2)",
        )
        .bind(organization_id)
        .bind(branch.as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        if !owned {
            return Err(AppError::Domain(format!(
                "outOfScope: branch {} is not the organisation's",
                branch.as_str()
            )));
        }
        sqlx::query(
            "INSERT INTO branch_exchange_limits \
             (organization_id, branch_id, approval_limit_thb, updated_by, updated_at) \
             VALUES ($1, $2, $3::numeric, $4, $5) \
             ON CONFLICT (organization_id, branch_id) DO UPDATE \
             SET approval_limit_thb = EXCLUDED.approval_limit_thb, \
                 updated_by = EXCLUDED.updated_by, updated_at = EXCLUDED.updated_at",
        )
        .bind(organization_id)
        .bind(branch.as_str())
        .bind(limit.map(|l| l.to_string()))
        .bind(by)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }
}
