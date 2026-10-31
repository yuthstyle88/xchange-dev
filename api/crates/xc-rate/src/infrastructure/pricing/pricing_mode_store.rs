//! `PgPricingModeStore`: `pricing_modes` (pricing-tiers.md §15.2, §15.5).
//!
//! SKELETON (XC-V9-PRICING): every body is PV-f2b's. Signatures stay.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, Row};
use xc_kernel::money::CurrencyCode;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::pricing::pricing_mode::{PricingMode, PricingModeRow, PricingModeStore};
use crate::domain::pricing::tier_set::StoreFuture;

pub struct PgPricingModeStore;

impl PricingModeStore for PgPricingModeStore {
    fn save_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        organization_id: &'a str,
        currency: CurrencyCode,
        mode: PricingMode,
        by: &'a str,
        at: DateTime<Utc>,
    ) -> StoreFuture<'a, i64> {
        Box::pin(async move {
            let row = sqlx::query(
                "INSERT INTO pricing_modes (organization_id, currency_code, mode, revision, updated_by, updated_at) \
                 VALUES ($1::uuid, $2, $3, 1, $4, $5) \
                 ON CONFLICT (organization_id, currency_code) DO UPDATE \
                 SET mode = EXCLUDED.mode, \
                     revision = pricing_modes.revision + 1, \
                     updated_by = EXCLUDED.updated_by, \
                     updated_at = EXCLUDED.updated_at \
                 RETURNING revision",
            )
            .bind(organization_id)
            .bind(currency.as_str())
            .bind(mode.as_str())
            .bind(by)
            .bind(at)
            .fetch_one(&mut *conn)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            Ok(row.get("revision"))
        })
    }

    fn get_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        organization_id: &'a str,
        currency: CurrencyCode,
    ) -> StoreFuture<'a, Option<PricingModeRow>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT mode, revision, updated_by, updated_at \
                 FROM pricing_modes \
                 WHERE organization_id = $1::uuid AND currency_code = $2",
            )
            .bind(organization_id)
            .bind(currency.as_str())
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            let Some(row) = row else {
                return Ok(None);
            };
            let mode_str: String = row.get("mode");
            let mode = PricingMode::parse(&mode_str).ok_or_else(|| {
                RateEngineError::Internal(format!("unknown pricing_modes.mode {mode_str}"))
            })?;
            Ok(Some(PricingModeRow {
                currency,
                mode,
                revision: row.get("revision"),
                updated_by: row.get("updated_by"),
                updated_at: row.get("updated_at"),
            }))
        })
    }

    fn following_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        organization_id: &'a str,
    ) -> StoreFuture<'a, Vec<CurrencyCode>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT currency_code FROM pricing_modes \
                 WHERE organization_id = $1::uuid AND mode = 'FOLLOW' ORDER BY currency_code",
            )
            .bind(organization_id)
            .fetch_all(&mut *conn)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            let mut codes = Vec::with_capacity(rows.len());
            for row in rows {
                let code_str: String = row.get("currency_code");
                let code = CurrencyCode::parse(code_str.trim()).map_err(|e| {
                    RateEngineError::Internal(format!(
                        "invalid pricing_modes.currency_code {code_str}: {e}"
                    ))
                })?;
                codes.push(code);
            }
            Ok(codes)
        })
    }

    fn newest_revision_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        organization_id: &'a str,
    ) -> StoreFuture<'a, Option<i64>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT MAX(revision) AS newest FROM pricing_modes WHERE organization_id = $1::uuid",
            )
            .bind(organization_id)
            .fetch_one(&mut *conn)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            Ok(row.get("newest"))
        })
    }
}
