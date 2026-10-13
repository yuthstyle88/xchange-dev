//! `PerformanceReader` (design §6 `PerformanceReader ..|> PromoExpenseFacts`, D-15): M04's figures and executive's
//! REWARD expense, read from `benefit_redemptions`, `transactions` and `transaction_lines`.

use std::sync::Arc;

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::PgPool;
use xc_kernel::money::Money;

use crate::domain::performance::commission::CampaignCommission;
use crate::domain::performance::period::{PerformancePeriod, PerformanceScope};
use crate::domain::performance::promo_expense::PromoExpenseFacts;
use crate::domain::performance::unavailable::Unavailable;
use crate::domain::performance::version_performance::VersionPerformance;

pub struct PerformanceReader {
    pool: PgPool,
    commission: Arc<dyn CampaignCommission>,
}

impl PerformanceReader {
    pub fn new(pool: PgPool, commission: Arc<dyn CampaignCommission>) -> Self {
        Self { pool, commission }
    }

    /// One row per campaign version with at least one counted redemption in `scope` and `period`, ordered by
    /// campaign name, then version number.
    pub async fn by_version(
        &self,
        scope: &PerformanceScope,
        period: &PerformancePeriod,
    ) -> AppResult<Vec<VersionPerformance>> {
        let branch_ids: Vec<&str> = scope.branches.iter().map(|b| b.as_str()).collect();
        if branch_ids.is_empty() {
            return Ok(Vec::new());
        }

        let version_rows = sqlx::query(
            r#"
            SELECT
                c.id AS campaign_id,
                c.name AS campaign_name,
                cv.id AS version_id,
                cv.version AS version,
                COUNT(br.id)::bigint AS redemptions,
                COALESCE(SUM(CASE WHEN br.benefit_class = 'PRICE' THEN br.amount_thb ELSE 0 END), 0)::numeric(20,2) AS price_cost,
                COALESCE(SUM(CASE WHEN br.benefit_class = 'REWARD' THEN br.amount_thb ELSE 0 END), 0)::numeric(20,2) AS reward_expense,
                ARRAY_AGG(DISTINCT br.transaction_id) AS transaction_ids
            FROM benefit_redemptions br
            JOIN campaign_versions cv ON cv.id = br.campaign_version_id
            JOIN campaigns c ON c.id = cv.campaign_id
            JOIN transactions t ON t.id = br.transaction_id
            JOIN branches b ON b.id = t.branch_id
            WHERE br.status = 'REDEEMED'
              AND t.branch_id = ANY($1)
              AND (br.redeemed_at AT TIME ZONE b.timezone)::date >= $2
              AND (br.redeemed_at AT TIME ZONE b.timezone)::date <= $3
            GROUP BY c.id, c.name, cv.id, cv.version
            ORDER BY c.name ASC, cv.version ASC
            "#,
        )
        .bind(&branch_ids[..])
        .bind(period.from)
        .bind(period.to)
        .fetch_all(&self.pool)
        .await?;

        let mut results = Vec::with_capacity(version_rows.len());

        for v_row in version_rows {
            use sqlx::Row;

            let campaign_id: uuid::Uuid = v_row.get("campaign_id");
            let campaign_name: String = v_row.get("campaign_name");
            let version_id: uuid::Uuid = v_row.get("version_id");
            let version: i32 = v_row.get("version");
            let redemptions: i64 = v_row.get("redemptions");
            let mut pc: rust_decimal::Decimal = v_row.get("price_cost");
            pc.rescale(2);
            let price_cost = Money::try_from_minor_i128(pc.mantissa())
                .map_err(|e| pos_error::AppError::Internal(e.to_string()))?;

            let mut re: rust_decimal::Decimal = v_row.get("reward_expense");
            re.rescale(2);
            let reward_expense = Money::try_from_minor_i128(re.mantissa())
                .map_err(|e| pos_error::AppError::Internal(e.to_string()))?;

            let transaction_ids: Vec<uuid::Uuid> = v_row.get("transaction_ids");

            let lines_row = sqlx::query(
                r#"
                SELECT
                    COALESCE(SUM(thb_amount), 0)::numeric(20,2) AS volume_thb,
                    COALESCE(SUM(realized_margin_thb), 0)::numeric(20,2) AS realized_margin_thb,
                    COUNT(CASE WHEN realized_margin_thb IS NULL THEN 1 END)::bigint AS missing_margin_count
                FROM transaction_lines
                WHERE transaction_id = ANY($1)
                "#,
            )
            .bind(&transaction_ids[..])
            .fetch_one(&self.pool)
            .await?;

            let mut vol: rust_decimal::Decimal = lines_row.get("volume_thb");
            vol.rescale(2);
            let volume_thb = Money::try_from_minor_i128(vol.mantissa())
                .map_err(|e| pos_error::AppError::Internal(e.to_string()))?;

            let missing_margin_count: i64 = lines_row.get("missing_margin_count");
            let realized_margin = if missing_margin_count > 0 {
                Err(Unavailable::AwaitsLedger)
            } else {
                let mut rm: rust_decimal::Decimal = lines_row.get("realized_margin_thb");
                rm.rescale(2);
                Ok(xc_kernel::money::MoneyDelta::from_minor(rm.mantissa()))
            };

            let commission = self.commission.commission_of(&transaction_ids).await?;
            let contribution = crate::domain::performance::contribution::attributed_contribution(
                realized_margin,
                reward_expense,
                commission,
            );

            results.push(VersionPerformance {
                campaign_id,
                campaign_name,
                version_id,
                version,
                redemptions,
                price_cost,
                reward_expense,
                volume_thb,
                realized_margin,
                contribution,
            });
        }

        Ok(results)
    }
}

impl PromoExpenseFacts for PerformanceReader {
    fn reward_expense<'a>(
        &'a self,
        scope: &'a PerformanceScope,
        period: &'a PerformancePeriod,
    ) -> BoxFuture<'a, AppResult<Result<Money, Unavailable>>> {
        Box::pin(async move {
            let branch_ids: Vec<&str> = scope.branches.iter().map(|b| b.as_str()).collect();
            if branch_ids.is_empty() {
                return Ok(Ok(Money::ZERO));
            }
            let sum: Option<rust_decimal::Decimal> = sqlx::query_scalar(
                r#"
                SELECT COALESCE(SUM(br.amount_thb), 0)::numeric(20, 2)
                FROM benefit_redemptions br
                JOIN transactions t ON t.id = br.transaction_id
                JOIN branches b ON b.id = t.branch_id
                WHERE br.status = 'REDEEMED'
                  AND br.benefit_class = 'REWARD'
                  AND t.branch_id = ANY($1)
                  AND (br.redeemed_at AT TIME ZONE b.timezone)::date >= $2
                  AND (br.redeemed_at AT TIME ZONE b.timezone)::date <= $3
                "#,
            )
            .bind(&branch_ids[..])
            .bind(period.from)
            .bind(period.to)
            .fetch_one(&self.pool)
            .await?;

            let sum = sum.unwrap_or(rust_decimal::Decimal::ZERO);
            let mut d = sum;
            d.rescale(2);
            let money = Money::try_from_minor_i128(d.mantissa())
                .map_err(|e| pos_error::AppError::Internal(e.to_string()))?;
            Ok(Ok(money))
        })
    }
}
