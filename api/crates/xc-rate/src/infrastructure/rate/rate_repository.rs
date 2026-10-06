//! Rate repository postgres implementation.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate};

use crate::domain::rate::entities::rate_change::RateChange;
use crate::domain::rate::repositories::rate_repository::{
    CurrentRateRow, NewRate, RateRepository, SavedRateRecord,
};
use crate::domain::validation::{decimal_to_rate, rate_to_decimal};

type SavedRateDbRow = (String, Decimal, Decimal, String, Option<Decimal>);

type HistoryDbRow = (
    String,
    Decimal,
    Decimal,
    DateTime<Utc>,
    String,
    String,
    String,
    Option<Decimal>,
    Option<serde_json::Value>,
);

type CurrentRateDbRow = (
    String,
    Decimal,
    Decimal,
    DateTime<Utc>,
    String,
    String,
    Option<Decimal>,
    Option<serde_json::Value>,
);

#[derive(Clone)]
pub struct PgRateRepository {
    pool: PgPool,
}

impl PgRateRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn find_rate_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        code: CurrencyCode,
    ) -> AppResult<Option<(Rate, Rate)>> {
        let row = sqlx::query_as::<_, (Decimal, Decimal)>(
            "SELECT buy, sell FROM rates WHERE branch_id = $1 AND code = $2 ORDER BY id DESC LIMIT 1",
        )
        .bind(branch.as_str())
        .bind(code.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some((buy_dec, sell_dec)) => {
                let buy = decimal_to_rate(&buy_dec)?;
                let sell = decimal_to_rate(&sell_dec)?;
                Ok(Some((buy, sell)))
            }
            None => Ok(None),
        }
    }
}

impl RateRepository for PgRateRepository {
    async fn list_currencies(&self) -> AppResult<Vec<(String, String, Option<String>)>> {
        sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT code, name_th, symbol FROM currencies WHERE code != 'THB'",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)
    }

    async fn current_rates(&self, branch: &BranchId) -> AppResult<Vec<CurrentRateRow>> {
        let rows = sqlx::query_as::<_, CurrentRateDbRow>(
            "SELECT DISTINCT ON (code) code, buy, sell, updated_at, request_id, partner_name, partner_cost, reference FROM rates WHERE branch_id = $1 ORDER BY code, id DESC",
        )
        .bind(branch.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let mut results = Vec::with_capacity(rows.len());
        for (code, buy_dec, sell_dec, updated_at, request_id, partner_name, cost_dec, reference) in
            rows
        {
            let buy = decimal_to_rate(&buy_dec)?;
            let sell = decimal_to_rate(&sell_dec)?;
            let partner_cost = cost_dec.as_ref().map(decimal_to_rate).transpose()?;

            results.push(CurrentRateRow {
                code,
                buy,
                sell,
                updated_at,
                request_id,
                partner_name,
                partner_cost,
                reference,
            });
        }

        Ok(results)
    }

    async fn history(&self, branch: &BranchId) -> AppResult<Vec<RateChange>> {
        let rows = sqlx::query_as::<_, HistoryDbRow>(
            "SELECT code, buy, sell, updated_at, updated_by, request_id, partner_name, partner_cost, reference FROM rates WHERE branch_id = $1 ORDER BY id DESC",
        )
        .bind(branch.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let mut changes = Vec::with_capacity(rows.len());
        for (
            r_code,
            buy_dec,
            sell_dec,
            updated_at,
            updated_by,
            request_id,
            partner_name,
            cost_dec,
            reference,
        ) in rows
        {
            let code = CurrencyCode::parse(r_code.trim())
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let buy = decimal_to_rate(&buy_dec)?;
            let sell = decimal_to_rate(&sell_dec)?;
            let partner_cost = cost_dec.as_ref().map(decimal_to_rate).transpose()?;

            changes.push(RateChange {
                code,
                buy,
                sell,
                updated_at,
                updated_by,
                request_id,
                partner_name,
                partner_cost,
                reference,
            });
        }

        Ok(changes)
    }

    async fn is_abandoned(&self, branch_id: &str, request_id: &str) -> AppResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM abandoned_requests WHERE branch_id = $1 AND request_id = $2)",
        )
        .bind(branch_id)
        .bind(request_id)
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Database)
    }

    async fn is_abandoned_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch_id: &str,
        request_id: &str,
    ) -> AppResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM abandoned_requests WHERE branch_id = $1 AND request_id = $2)",
        )
        .bind(branch_id)
        .bind(request_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)
    }

    async fn find_by_request(
        &self,
        branch_id: &str,
        request_id: &str,
    ) -> AppResult<Option<SavedRateRecord>> {
        let row = sqlx::query_as::<_, SavedRateDbRow>(
            "SELECT code, buy, sell, partner_name, partner_cost FROM rates WHERE branch_id = $1 AND request_id = $2",
        )
        .bind(branch_id)
        .bind(request_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some((code, buy_dec, sell_dec, partner_name, cost_dec)) => {
                let buy = decimal_to_rate(&buy_dec)?;
                let sell = decimal_to_rate(&sell_dec)?;
                let partner_cost = cost_dec.as_ref().map(decimal_to_rate).transpose()?;
                Ok(Some(SavedRateRecord {
                    code,
                    buy,
                    sell,
                    partner_name,
                    partner_cost,
                }))
            }
            None => Ok(None),
        }
    }

    async fn find_by_request_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch_id: &str,
        request_id: &str,
    ) -> AppResult<Option<SavedRateRecord>> {
        let row = sqlx::query_as::<_, SavedRateDbRow>(
            "SELECT code, buy, sell, partner_name, partner_cost FROM rates WHERE branch_id = $1 AND request_id = $2",
        )
        .bind(branch_id)
        .bind(request_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some((code, buy_dec, sell_dec, partner_name, cost_dec)) => {
                let buy = decimal_to_rate(&buy_dec)?;
                let sell = decimal_to_rate(&sell_dec)?;
                let partner_cost = cost_dec.as_ref().map(decimal_to_rate).transpose()?;
                Ok(Some(SavedRateRecord {
                    code,
                    buy,
                    sell,
                    partner_name,
                    partner_cost,
                }))
            }
            None => Ok(None),
        }
    }

    async fn newest_updated_at(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch_id: &str,
        code: &str,
    ) -> AppResult<Option<DateTime<Utc>>> {
        sqlx::query_scalar(
            "SELECT updated_at FROM rates WHERE branch_id = $1 AND code = $2 ORDER BY id DESC LIMIT 1",
        )
        .bind(branch_id)
        .bind(code)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)
    }

    async fn insert(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        rate: NewRate<'_>,
    ) -> AppResult<()> {
        let buy_dec = rate_to_decimal(&rate.buy)?;
        let sell_dec = rate_to_decimal(&rate.sell)?;
        let partner_cost_dec = rate
            .partner_cost
            .as_ref()
            .map(rate_to_decimal)
            .transpose()?;

        sqlx::query(
            "INSERT INTO rates (branch_id, code, buy, sell, partner_name, partner_cost, reference, updated_at, updated_by, request_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(rate.branch_id)
        .bind(rate.code)
        .bind(buy_dec)
        .bind(sell_dec)
        .bind(rate.partner_name)
        .bind(partner_cost_dec)
        .bind(rate.reference)
        .bind(rate.updated_at)
        .bind(rate.updated_by)
        .bind(rate.request_id)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }

    async fn rate_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        code: CurrencyCode,
    ) -> AppResult<Option<(Rate, Rate)>> {
        Self::find_rate_in(tx, branch, code).await
    }
}
