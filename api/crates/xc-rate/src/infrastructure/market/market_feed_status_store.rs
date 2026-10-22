//! `MarketFeedStatusStore`: each market source's last fetch attempt and error, in `market_feed_status`
//! (pricing-tiers.md §4, §12.1; V.9 keeps failed fetches in `market_snapshots`, `src/pricing/market.mjs:99-113`).
//!
//! SKELETON (XC-PRICING PX-0): `record` and `all` are PX-5's. Types and signatures stay.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::pricing::market_quote::MarketSource;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketAttempt {
    pub at: DateTime<Utc>,
    /// `None` after a success; the error text cut to 200 characters (`market.mjs:110`) after a failure.
    pub error: Option<String>,
}

pub struct MarketFeedStatusStore;

impl MarketFeedStatusStore {
    /// Upserts the source's row (`'TT'` / `'SR'`): `last_attempt_at = at`, `last_error` = the error cut to its first
    /// 200 characters, or NULL.
    pub async fn record(
        pool: &PgPool,
        source: MarketSource,
        at: DateTime<Utc>,
        error: Option<&str>,
    ) -> Result<(), RateEngineError> {
        let code = match source {
            MarketSource::Tt => "TT",
            MarketSource::SuperRich => "SR",
            MarketSource::Bot => {
                return Err(RateEngineError::Internal(
                    "invalid market_feed_status source: BOT".into(),
                ));
            }
        };
        let truncated: Option<String> = error.map(|s| s.chars().take(200).collect());
        sqlx::query(
            "INSERT INTO market_feed_status (source, last_attempt_at, last_error) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (source) DO UPDATE \
             SET last_attempt_at = EXCLUDED.last_attempt_at, last_error = EXCLUDED.last_error",
        )
        .bind(code)
        .bind(at)
        .bind(truncated)
        .execute(pool)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;
        Ok(())
    }

    /// Every stored row, keyed by source.
    pub async fn all(
        conn: &mut PgConnection,
    ) -> Result<HashMap<MarketSource, MarketAttempt>, RateEngineError> {
        let rows: Vec<(String, DateTime<Utc>, Option<String>)> =
            sqlx::query_as("SELECT source, last_attempt_at, last_error FROM market_feed_status")
                .fetch_all(&mut *conn)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let mut out = HashMap::with_capacity(rows.len());
        for (source_str, at, error) in rows {
            let source = match source_str.as_str() {
                "TT" => MarketSource::Tt,
                "SR" => MarketSource::SuperRich,
                other => {
                    return Err(RateEngineError::Internal(format!(
                        "unknown market_feed_status.source: {other}"
                    )));
                }
            };
            out.insert(source, MarketAttempt { at, error });
        }
        Ok(out)
    }
}
