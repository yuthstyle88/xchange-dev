//! `PgTierSetStore`: `pricing_tier_sets` (pricing-tiers.md §15.2). `tiers` is jsonb `[{tier,buy,sell}]` with decimal
//! strings; `anchor` is jsonb `{source,buy,sell,fetched_at,batch_id}`.
//!
//! SKELETON (XC-V9-PRICING): every body is PV-a4a's. Signatures stay.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Row, postgres::PgRow};
use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, Rate};

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::pricing::market_quote::MarketSource;
use crate::domain::pricing::tier_set::{
    MarketAnchor, PricingTierSet, StoreFuture, TierRate, TierSetSource, TierSetStore,
};

pub struct PgTierSetStore;

#[derive(Serialize, Deserialize)]
struct TierJson {
    tier: u8,
    buy: String,
    sell: String,
}

#[derive(Serialize, Deserialize)]
struct AnchorJson {
    source: &'static str,
    buy: String,
    sell: String,
    fetched_at: DateTime<Utc>,
    batch_id: Option<Uuid>,
}

#[derive(Deserialize)]
struct AnchorJsonOwned {
    source: String,
    buy: String,
    sell: String,
    fetched_at: DateTime<Utc>,
    batch_id: Option<Uuid>,
}

fn source_str(s: MarketSource) -> &'static str {
    match s {
        MarketSource::Tt => "TT",
        MarketSource::SuperRich => "SR",
        MarketSource::Bot => "BOT",
    }
}

fn parse_source(s: &str) -> Result<MarketSource, RateEngineError> {
    match s {
        "TT" => Ok(MarketSource::Tt),
        "SR" => Ok(MarketSource::SuperRich),
        "BOT" => Ok(MarketSource::Bot),
        other => Err(RateEngineError::Internal(format!(
            "unknown market source: {other}"
        ))),
    }
}

fn parse_rate(s: &str) -> Result<Rate, RateEngineError> {
    Rate::parse(s).map_err(|e| RateEngineError::Internal(format!("invalid rate {s}: {e}")))
}

fn decode_row(row: PgRow) -> Result<PricingTierSet, RateEngineError> {
    let currency_str: String = row.get("currency_code");
    let currency = CurrencyCode::parse(currency_str.trim())
        .map_err(|_| RateEngineError::Internal(format!("unknown currency code: {currency_str}")))?;
    let source_str: String = row.get("source");
    let source = TierSetSource::parse(&source_str).ok_or_else(|| {
        RateEngineError::Internal(format!("unknown tier set source: {source_str}"))
    })?;

    let tiers_val: serde_json::Value = row.get("tiers");
    let mut tiers_vec: Vec<TierJson> = serde_json::from_value(tiers_val)
        .map_err(|e| RateEngineError::Internal(format!("invalid tiers json: {e}")))?;
    tiers_vec.sort_by_key(|t| t.tier);
    if tiers_vec.len() != 4
        || tiers_vec
            .iter()
            .enumerate()
            .any(|(i, t)| t.tier as usize != i + 1)
    {
        return Err(RateEngineError::Internal(
            "tiers json must contain tiers 1..=4".to_string(),
        ));
    }
    let mut tiers_parsed = Vec::with_capacity(4);
    for t in tiers_vec {
        tiers_parsed.push(TierRate {
            buy: parse_rate(&t.buy)?,
            sell: parse_rate(&t.sell)?,
        });
    }
    let tiers: [TierRate; 4] = tiers_parsed
        .try_into()
        .map_err(|_| RateEngineError::Internal("expected 4 tiers".to_string()))?;

    let anchor_val: Option<serde_json::Value> = row.get("anchor");
    let anchor = match anchor_val {
        Some(v) if !v.is_null() => {
            let a: AnchorJsonOwned = serde_json::from_value(v)
                .map_err(|e| RateEngineError::Internal(format!("invalid anchor json: {e}")))?;
            Some(MarketAnchor {
                source: parse_source(&a.source)?,
                buy: parse_rate(&a.buy)?,
                sell: parse_rate(&a.sell)?,
                fetched_at: a.fetched_at,
                batch_id: a.batch_id,
            })
        }
        _ => None,
    };

    Ok(PricingTierSet {
        id: row.get("id"),
        request_id: row.get("request_id"),
        branch: row.get("branch_id"),
        currency,
        source,
        base_tier_set_id: row.get("base_tier_set_id"),
        tiers,
        anchor,
        flags: row.get("flags"),
        draft_id: row.get("draft_id"),
        approved_by: row.get("approved_by"),
        approved_at: row.get("approved_at"),
    })
}

impl TierSetStore for PgTierSetStore {
    fn insert_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        set: &'a PricingTierSet,
    ) -> StoreFuture<'a, bool> {
        Box::pin(async move {
            let tiers_json: Vec<TierJson> = set
                .tiers
                .iter()
                .enumerate()
                .map(|(i, t)| TierJson {
                    tier: (i + 1) as u8,
                    buy: t.buy.value().normalize().to_string(),
                    sell: t.sell.value().normalize().to_string(),
                })
                .collect();
            let tiers_val = serde_json::to_value(tiers_json)
                .map_err(|e| RateEngineError::Internal(e.to_string()))?;

            let anchor_val = match &set.anchor {
                Some(a) => Some(
                    serde_json::to_value(AnchorJson {
                        source: source_str(a.source),
                        buy: a.buy.value().normalize().to_string(),
                        sell: a.sell.value().normalize().to_string(),
                        fetched_at: a.fetched_at,
                        batch_id: a.batch_id,
                    })
                    .map_err(|e| RateEngineError::Internal(e.to_string()))?,
                ),
                None => None,
            };

            let res = sqlx::query(
                "INSERT INTO pricing_tier_sets \
                 (id, request_id, branch_id, currency_code, source, base_tier_set_id, \
                  tiers, anchor, flags, draft_id, approved_by, approved_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
                 ON CONFLICT (request_id) DO NOTHING",
            )
            .bind(set.id)
            .bind(&set.request_id)
            .bind(&set.branch)
            .bind(set.currency.as_str())
            .bind(set.source.as_str())
            .bind(set.base_tier_set_id)
            .bind(tiers_val)
            .bind(anchor_val)
            .bind(&set.flags)
            .bind(set.draft_id)
            .bind(&set.approved_by)
            .bind(set.approved_at)
            .execute(&mut *conn)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            Ok(res.rows_affected() > 0)
        })
    }

    fn latest_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
        currency: CurrencyCode,
    ) -> StoreFuture<'a, Option<PricingTierSet>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, request_id, branch_id, currency_code, source, base_tier_set_id, \
                        tiers, anchor, flags, draft_id, approved_by, approved_at \
                 FROM pricing_tier_sets \
                 WHERE branch_id = $1 AND currency_code = $2 \
                 ORDER BY approved_at DESC LIMIT 1",
            )
            .bind(branch)
            .bind(currency.as_str())
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;
            row.map(decode_row).transpose()
        })
    }

    fn latest_follow_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
        currency: CurrencyCode,
    ) -> StoreFuture<'a, Option<PricingTierSet>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, request_id, branch_id, currency_code, source, base_tier_set_id, \
                        tiers, anchor, flags, draft_id, approved_by, approved_at \
                 FROM pricing_tier_sets \
                 WHERE branch_id = $1 AND currency_code = $2 AND source = 'FOLLOW' \
                 ORDER BY approved_at DESC LIMIT 1",
            )
            .bind(branch)
            .bind(currency.as_str())
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;
            row.map(decode_row).transpose()
        })
    }

    fn get_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        id: Uuid,
    ) -> StoreFuture<'a, Option<PricingTierSet>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, request_id, branch_id, currency_code, source, base_tier_set_id, \
                        tiers, anchor, flags, draft_id, approved_by, approved_at \
                 FROM pricing_tier_sets WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;
            row.map(decode_row).transpose()
        })
    }

    fn set_draft_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        id: Uuid,
        draft_id: Uuid,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query("UPDATE pricing_tier_sets SET draft_id = $1 WHERE id = $2")
                .bind(draft_id)
                .bind(id)
                .execute(&mut *conn)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;
            Ok(())
        })
    }

    fn by_request_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        request_id: &'a str,
    ) -> StoreFuture<'a, Option<PricingTierSet>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, request_id, branch_id, currency_code, source, base_tier_set_id, \
                        tiers, anchor, flags, draft_id, approved_by, approved_at \
                 FROM pricing_tier_sets WHERE request_id = $1",
            )
            .bind(request_id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;
            row.map(decode_row).transpose()
        })
    }
}
