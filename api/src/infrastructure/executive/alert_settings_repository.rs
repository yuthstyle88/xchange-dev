//! EX-5 (#1075): repository for branch alert settings and stock floors.

use std::str::FromStr;

use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;

use crate::presentation::dto::v3::branch_alert_settings::{BranchAlertSettingsData, StockFloorDto};

pub struct PgAlertSettingsRepository {
    pool: PgPool,
}

pub struct SaveAlertSettingsParams<'a> {
    pub org_id: Uuid,
    pub branch_id: &'a BranchId,
    pub stale_data_minutes: Option<i32>,
    pub floors: &'a [StockFloorDto],
    pub new_version: i32,
    pub updated_by: Uuid,
}

impl PgAlertSettingsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn read(
        &self,
        org_id: Uuid,
        branch_id: &BranchId,
    ) -> AppResult<BranchAlertSettingsData> {
        let row: Option<(Option<i32>, i32)> = sqlx::query_as(
            "SELECT stale_data_minutes, version FROM branch_alert_settings \
             WHERE organization_id = $1 AND branch_id = $2",
        )
        .bind(org_id)
        .bind(branch_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let Some((stale_data_minutes, version)) = row else {
            return Ok(BranchAlertSettingsData {
                branch: branch_id.as_str().to_string(),
                stale_data_minutes: None,
                floors: vec![],
                version: None,
            });
        };

        let floor_rows: Vec<(String, Decimal)> = sqlx::query_as(
            "SELECT currency, min_quantity FROM branch_stock_floors \
             WHERE organization_id = $1 AND branch_id = $2 \
             ORDER BY currency",
        )
        .bind(org_id)
        .bind(branch_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let floors = floor_rows
            .into_iter()
            .map(|(currency, min_quantity)| StockFloorDto {
                currency: currency.trim().to_string(),
                min_quantity: min_quantity.normalize().to_string(),
            })
            .collect();

        Ok(BranchAlertSettingsData {
            branch: branch_id.as_str().to_string(),
            stale_data_minutes,
            floors,
            version: Some(version),
        })
    }

    pub async fn read_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        org_id: Uuid,
        branch_id: &BranchId,
    ) -> AppResult<BranchAlertSettingsData> {
        let row: Option<(Option<i32>, i32)> = sqlx::query_as(
            "SELECT stale_data_minutes, version FROM branch_alert_settings \
             WHERE organization_id = $1 AND branch_id = $2",
        )
        .bind(org_id)
        .bind(branch_id.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let Some((stale_data_minutes, version)) = row else {
            return Ok(BranchAlertSettingsData {
                branch: branch_id.as_str().to_string(),
                stale_data_minutes: None,
                floors: vec![],
                version: None,
            });
        };

        let floor_rows: Vec<(String, Decimal)> = sqlx::query_as(
            "SELECT currency, min_quantity FROM branch_stock_floors \
             WHERE organization_id = $1 AND branch_id = $2 \
             ORDER BY currency",
        )
        .bind(org_id)
        .bind(branch_id.as_str())
        .fetch_all(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let floors = floor_rows
            .into_iter()
            .map(|(currency, min_quantity)| StockFloorDto {
                currency: currency.trim().to_string(),
                min_quantity: min_quantity.normalize().to_string(),
            })
            .collect();

        Ok(BranchAlertSettingsData {
            branch: branch_id.as_str().to_string(),
            stale_data_minutes,
            floors,
            version: Some(version),
        })
    }

    pub async fn save_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        params: SaveAlertSettingsParams<'_>,
    ) -> AppResult<BranchAlertSettingsData> {
        let SaveAlertSettingsParams {
            org_id,
            branch_id,
            stale_data_minutes,
            floors,
            new_version,
            updated_by,
        } = params;

        sqlx::query(
            "INSERT INTO branch_alert_settings \
                 (organization_id, branch_id, stale_data_minutes, version, updated_by, updated_at) \
             VALUES ($1, $2, $3, $4, $5, now()) \
             ON CONFLICT (organization_id, branch_id) DO UPDATE \
                SET stale_data_minutes = EXCLUDED.stale_data_minutes, \
                    version = EXCLUDED.version, \
                    updated_by = EXCLUDED.updated_by, \
                    updated_at = EXCLUDED.updated_at",
        )
        .bind(org_id)
        .bind(branch_id.as_str())
        .bind(stale_data_minutes)
        .bind(new_version)
        .bind(updated_by)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        sqlx::query(
            "DELETE FROM branch_stock_floors \
             WHERE organization_id = $1 AND branch_id = $2",
        )
        .bind(org_id)
        .bind(branch_id.as_str())
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let mut sorted_floors = floors.to_vec();
        sorted_floors.sort_by(|a, b| a.currency.cmp(&b.currency));

        let mut saved_floors = Vec::with_capacity(sorted_floors.len());
        for floor in sorted_floors {
            let dec = Decimal::from_str(&floor.min_quantity)
                .map_err(|e| AppError::Validation(e.to_string()))?;
            sqlx::query(
                "INSERT INTO branch_stock_floors \
                     (organization_id, branch_id, currency, min_quantity, updated_by, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, now())",
            )
            .bind(org_id)
            .bind(branch_id.as_str())
            .bind(&floor.currency)
            .bind(dec)
            .bind(updated_by)
            .execute(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            saved_floors.push(StockFloorDto {
                currency: floor.currency,
                min_quantity: dec.normalize().to_string(),
            });
        }

        Ok(BranchAlertSettingsData {
            branch: branch_id.as_str().to_string(),
            stale_data_minutes,
            floors: saved_floors,
            version: Some(new_version),
        })
    }
}
