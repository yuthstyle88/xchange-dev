//! `PgFollowResultStore`: `pricing_follow_results` (pricing-tiers.md §15.2, §15.5).
//!
//! SKELETON (XC-V9-PRICING): every body is PV-f2a's. Signatures stay.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, Row};
use xc_kernel::money::CurrencyCode;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::pricing::follow_plan::{
    FollowReason, FollowResult, FollowResultStatus, FollowResultStore,
};
use crate::domain::pricing::tier_set::StoreFuture;

pub struct PgFollowResultStore;

impl FollowResultStore for PgFollowResultStore {
    fn record_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        result: &'a FollowResult,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let reasons: Vec<&str> = result.reasons.iter().map(|r| r.as_str()).collect();
            sqlx::query(
                "INSERT INTO pricing_follow_results \
                 (currency_code, status, reasons, anchor_batch_id, tier_set_id, planned_at) \
                 VALUES ($1, $2, $3, $4, $5, $6) \
                 ON CONFLICT (currency_code) DO UPDATE \
                 SET status = EXCLUDED.status, \
                     reasons = EXCLUDED.reasons, \
                     anchor_batch_id = EXCLUDED.anchor_batch_id, \
                     tier_set_id = EXCLUDED.tier_set_id, \
                     planned_at = EXCLUDED.planned_at",
            )
            .bind(result.currency.as_str())
            .bind(result.status.as_str())
            .bind(reasons)
            .bind(result.anchor_batch_id)
            .bind(result.tier_set_id)
            .bind(result.planned_at)
            .execute(&mut *conn)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;
            Ok(())
        })
    }

    fn latest_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        currency: CurrencyCode,
    ) -> StoreFuture<'a, Option<FollowResult>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT currency_code, status, reasons, anchor_batch_id, tier_set_id, planned_at \
                 FROM pricing_follow_results \
                 WHERE currency_code = $1",
            )
            .bind(currency.as_str())
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            let Some(row) = row else {
                return Ok(None);
            };

            let currency_str: String = row.get("currency_code");
            let currency = CurrencyCode::parse(currency_str.trim()).map_err(|_| {
                RateEngineError::Internal(format!("unknown currency code: {currency_str}"))
            })?;
            let status_str: String = row.get("status");
            let status = FollowResultStatus::parse(&status_str).ok_or_else(|| {
                RateEngineError::Internal(format!("unknown follow result status: {status_str}"))
            })?;
            let reasons_raw: Vec<String> = row.get("reasons");
            let reasons = reasons_raw
                .into_iter()
                .map(|r| {
                    FollowReason::parse(&r).ok_or_else(|| {
                        RateEngineError::Internal(format!("unknown follow reason: {r}"))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;

            Ok(Some(FollowResult {
                currency,
                status,
                reasons,
                anchor_batch_id: row.get("anchor_batch_id"),
                tier_set_id: row.get("tier_set_id"),
                planned_at: row.get("planned_at"),
            }))
        })
    }

    fn newest_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
    ) -> StoreFuture<'a, Option<DateTime<Utc>>> {
        Box::pin(async move {
            let row = sqlx::query("SELECT MAX(planned_at) AS newest FROM pricing_follow_results")
                .fetch_one(&mut *conn)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;
            Ok(row.get("newest"))
        })
    }
}
