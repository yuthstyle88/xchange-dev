//! `PgCustomerMetrics` adapter (design §4.1, §6, §7).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;
use xc_loyalty::domain::membership_mirror::MembershipMirror;
use xc_marketing::domain::segment::metrics_source::{CustomerMetrics, CustomerMetricsSource};

pub struct PgCustomerMetrics {
    pool: PgPool,
    loyalty_mirror: Option<Arc<dyn MembershipMirror>>,
}

impl PgCustomerMetrics {
    pub fn new(pool: PgPool, loyalty_mirror: Option<Arc<dyn MembershipMirror>>) -> Self {
        Self {
            pool,
            loyalty_mirror,
        }
    }

    pub async fn load_candidate_metrics_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        org: Uuid,
        lookback_months: u32,
        as_of: DateTime<Utc>,
    ) -> AppResult<Vec<CustomerMetrics>> {
        #[derive(sqlx::FromRow)]
        struct CandidateRowIn {
            customer_id: String,
            customer_name: String,
            frequency: i64,
            value_thb: Decimal,
            last_recorded_at: Option<DateTime<Utc>>,
            home_branch: Option<String>,
            tier_code: Option<String>,
        }

        let rows: Vec<CandidateRowIn> = sqlx::query_as(
            "SELECT \
                 c.id AS customer_id, \
                 c.display_name AS customer_name, \
                 COUNT(t.id) AS frequency, \
                 COALESCE(SUM((SELECT SUM(l.thb_amount) FROM transaction_lines l WHERE l.transaction_id = t.id)), 0::numeric) AS value_thb, \
                 MAX(t.recorded_at) AS last_recorded_at, \
                 (ARRAY_AGG(t.branch_id ORDER BY t.recorded_at DESC) FILTER (WHERE t.branch_id IS NOT NULL))[1] AS home_branch, \
                 ( \
                     SELECT m.tier_code \
                     FROM customer_identities i \
                     JOIN customer_tier_mirror m \
                       ON m.organization_id = i.organization_id \
                      AND m.phone_lookup_hash = i.lookup_hash \
                     WHERE i.organization_id = c.organization_id \
                       AND i.customer_id = c.id \
                       AND i.type = 'PHONE' \
                       AND i.status = 'ACTIVE' \
                       AND i.verified_at IS NOT NULL \
                     LIMIT 1 \
                 ) AS tier_code \
             FROM customers c \
             LEFT JOIN transactions t \
                 ON t.customer_id = c.id \
                AND t.status = 'COMPLETED' \
                AND t.mode = 'LIVE' \
                AND t.recorded_at >= ($2 - make_interval(months => $3::int)) \
                AND t.recorded_at <= $2 \
             WHERE c.organization_id = $1 \
               AND c.mode = 'LIVE' \
             GROUP BY c.id, c.display_name \
             ORDER BY c.id",
        )
        .bind(org)
        .bind(as_of)
        .bind(lookback_months as i32)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

        let mut metrics = Vec::with_capacity(rows.len());
        for row in rows {
            let recency_days = row.last_recorded_at.map(|last| {
                (as_of.signed_duration_since(last).num_seconds().max(0) / 86400) as u32
            });

            metrics.push(CustomerMetrics {
                customer_id: row.customer_id,
                customer_name: row.customer_name,
                recency_days,
                frequency: row.frequency.max(0) as u32,
                value_thb: row.value_thb,
                tier_code: row.tier_code,
                home_branch: row.home_branch,
            });
        }

        Ok(metrics)
    }
}

#[derive(sqlx::FromRow)]
struct CandidateRow {
    customer_id: String,
    customer_name: String,
    frequency: i64,
    value_thb: Decimal,
    last_recorded_at: Option<DateTime<Utc>>,
    home_branch: Option<String>,
}

impl CustomerMetricsSource for PgCustomerMetrics {
    fn load_candidate_metrics<'a>(
        &'a self,
        org: Uuid,
        lookback_months: u32,
        as_of: DateTime<Utc>,
    ) -> Pin<Box<dyn Future<Output = AppResult<Vec<CustomerMetrics>>> + Send + 'a>> {
        Box::pin(async move {
            let rows: Vec<CandidateRow> = sqlx::query_as(
                "SELECT \
                     c.id AS customer_id, \
                     c.display_name AS customer_name, \
                     COUNT(t.id) AS frequency, \
                     COALESCE(SUM((SELECT SUM(l.thb_amount) FROM transaction_lines l WHERE l.transaction_id = t.id)), 0::numeric) AS value_thb, \
                     MAX(t.recorded_at) AS last_recorded_at, \
                     (ARRAY_AGG(t.branch_id ORDER BY t.recorded_at DESC) FILTER (WHERE t.branch_id IS NOT NULL))[1] AS home_branch \
                 FROM customers c \
                 LEFT JOIN transactions t \
                     ON t.customer_id = c.id \
                    AND t.status = 'COMPLETED' \
                    AND t.mode = 'LIVE' \
                    AND t.recorded_at >= ($2 - make_interval(months => $3::int)) \
                    AND t.recorded_at <= $2 \
                 WHERE c.organization_id = $1 \
                   AND c.mode = 'LIVE' \
                 GROUP BY c.id, c.display_name \
                 ORDER BY c.id",
            )
            .bind(org)
            .bind(as_of)
            .bind(lookback_months as i32)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;

            let mut metrics = Vec::with_capacity(rows.len());
            for row in rows {
                let recency_days = row.last_recorded_at.map(|last| {
                    (as_of.signed_duration_since(last).num_seconds().max(0) / 86400) as u32
                });

                let tier_code = if let Some(ref mirror) = self.loyalty_mirror {
                    match mirror.tier_of(org, &row.customer_id).await {
                        Ok(Some(tier)) => Some(tier.tier_code),
                        _ => None,
                    }
                } else {
                    None
                };

                metrics.push(CustomerMetrics {
                    customer_id: row.customer_id,
                    customer_name: row.customer_name,
                    recency_days,
                    frequency: row.frequency.max(0) as u32,
                    value_thb: row.value_thb,
                    tier_code,
                    home_branch: row.home_branch,
                });
            }

            Ok(metrics)
        })
    }
}
