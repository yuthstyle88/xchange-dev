//! Per-branch, per-currency drawer low-cash warning levels (design `docs/design/blueprint/ledger-inventory.md`
//! §5.1.3 "2. Low-cash warning"). The one reader and writer of `cash_drawer_low_levels`.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

pub struct DrawerLowLevels;

impl DrawerLowLevels {
    pub async fn for_branch_in(
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
    ) -> AppResult<Vec<(CurrencyCode, Decimal)>> {
        let rows: Vec<(String, Decimal)> = sqlx::query_as(
            "SELECT l.currency, l.low_level \
               FROM cash_drawer_low_levels l \
               JOIN branches b ON b.organization_id = l.organization_id AND b.id = l.branch_id \
              WHERE b.id = $1 \
              ORDER BY l.currency",
        )
        .bind(branch.as_str())
        .fetch_all(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter()
            .map(|(c, level)| {
                let code =
                    CurrencyCode::parse(c.trim()).map_err(|e| AppError::Internal(e.to_string()))?;
                Ok((code, level.normalize()))
            })
            .collect()
    }

    pub async fn replace_in(
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        levels: &[(CurrencyCode, Decimal)],
        by: Uuid,
        at: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM cash_drawer_low_levels l \
              USING branches b \
              WHERE b.organization_id = l.organization_id AND b.id = l.branch_id AND b.id = $1",
        )
        .bind(branch.as_str())
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        for &(currency, low_level) in levels {
            sqlx::query(
                "INSERT INTO cash_drawer_low_levels \
                     (organization_id, branch_id, currency, low_level, updated_by, updated_at) \
                 SELECT b.organization_id, b.id, $2, $3, $4, $5 \
                   FROM branches b WHERE b.id = $1",
            )
            .bind(branch.as_str())
            .bind(currency.as_str())
            .bind(low_level)
            .bind(by)
            .bind(at)
            .execute(&mut **tx)
            .await
            .map_err(AppError::Database)?;
        }
        Ok(())
    }

    pub async fn set_meta_in(
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
    ) -> AppResult<Option<(String, DateTime<Utc>)>> {
        sqlx::query_as(
            "SELECT l.updated_by::text, l.updated_at \
               FROM cash_drawer_low_levels l \
               JOIN branches b ON b.organization_id = l.organization_id AND b.id = l.branch_id \
              WHERE b.id = $1 \
              LIMIT 1",
        )
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)
    }
}
