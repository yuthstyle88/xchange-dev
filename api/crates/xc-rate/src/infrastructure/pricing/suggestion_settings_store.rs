//! `PricingSuggestionSettingsStore`: `pricing_suggestion_settings` (pricing-tiers.md §5.2, §12.1).
//!
//! SKELETON (XC-PRICING PX-0): every body is PX-10's. Signatures stay.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::{PgConnection, Row};

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::pricing::suggestion_settings::SuggestionSettings;

pub struct PricingSuggestionSettingsStore;

impl PricingSuggestionSettingsStore {
    /// The branch's `'*'` row and its currency rows.
    pub async fn for_branch(
        conn: &mut PgConnection,
        branch: &str,
    ) -> Result<SuggestionSettings, RateEngineError> {
        let rows = sqlx::query(
            "SELECT scope, better_pct FROM pricing_suggestion_settings WHERE branch_id = $1",
        )
        .bind(branch)
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let mut settings = SuggestionSettings::default();
        for row in rows {
            let scope: String = row.get("scope");
            let better_vec: Vec<Decimal> = row.get("better_pct");
            let better: [Decimal; 4] = better_vec.try_into().map_err(|v: Vec<Decimal>| {
                RateEngineError::Internal(format!(
                    "pricing_suggestion_settings({branch}, {scope}) has {} elements, expected 4",
                    v.len()
                ))
            })?;
            if scope == "*" {
                settings.branch_default = Some(better);
            } else {
                settings.by_currency.insert(scope, better);
            }
        }
        Ok(settings)
    }

    /// `scope` is `"*"` or a currency code. `Some(better)` upserts the row; `None` deletes it.
    pub async fn save(
        conn: &mut PgConnection,
        branch: &str,
        scope: &str,
        better: Option<[Decimal; 4]>,
        by: &str,
        at: DateTime<Utc>,
    ) -> Result<(), RateEngineError> {
        match better {
            Some(values) => {
                sqlx::query(
                    "INSERT INTO pricing_suggestion_settings (branch_id, scope, better_pct, updated_by, updated_at) \
                     VALUES ($1, $2, $3, $4, $5) \
                     ON CONFLICT (branch_id, scope) DO UPDATE \
                     SET better_pct = EXCLUDED.better_pct, \
                         updated_by = EXCLUDED.updated_by, \
                         updated_at = EXCLUDED.updated_at",
                )
                .bind(branch)
                .bind(scope)
                .bind(values.as_slice())
                .bind(by)
                .bind(at)
                .execute(&mut *conn)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;
            }
            None => {
                sqlx::query(
                    "DELETE FROM pricing_suggestion_settings WHERE branch_id = $1 AND scope = $2",
                )
                .bind(branch)
                .bind(scope)
                .execute(&mut *conn)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;
            }
        }
        Ok(())
    }
}
