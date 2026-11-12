//! `PricingRulesStore`: `pricing_rule_versions` (owner-set-values.md §2, §5; xchange#6555). One row per save, never
//! updated; the organisation's current rules are its newest row.

use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::policy::pricing_rules::{
    AdviceThresholds, PctRange, PricingRules, RateDecimals,
};

/// One saved version, or the "not set" answer when the organisation has none (every `Option` is then `None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PricingRulesVersion {
    pub version: Option<String>,
    pub rules: PricingRules,
    pub set_by: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

impl PricingRulesVersion {
    pub fn not_set() -> Self {
        Self {
            version: None,
            rules: PricingRules::not_set(),
            set_by: None,
            created_at: None,
        }
    }
}

const COLUMNS: &str = "version, far_from_anchor_limit_pct, large_move_pct, rate_decimals, thb_cash_step, \
     branch_bp_cap, better_pct_min, better_pct_max, min_spread_pct_min, min_spread_pct_max, \
     market_stale_after_minutes, advice_far_pct, advice_spread_ratio, advice_spread_gap_pt, \
     advice_market_moved_pct, set_by, created_at";

fn db(e: sqlx::Error) -> RateEngineError {
    RateEngineError::Database(e.to_string())
}

fn pair(min: Option<Decimal>, max: Option<Decimal>) -> Option<PctRange> {
    Some(PctRange {
        min: min?,
        max: max?,
    })
}

fn decimals_from_json(v: Value) -> Result<RateDecimals, RateEngineError> {
    let bad = || {
        RateEngineError::Internal(format!(
            "pricing_rule_versions.rate_decimals is malformed: {v}"
        ))
    };
    let bands = v
        .as_array()
        .ok_or_else(bad)?
        .iter()
        .map(|band| {
            let from = band["fromPrice"].as_str()?.parse::<Decimal>().ok()?;
            let decimals = u32::try_from(band["decimals"].as_u64()?).ok()?;
            Some((from, decimals))
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(bad)?;
    Ok(RateDecimals { bands })
}

fn decimals_to_json(d: &RateDecimals) -> Value {
    Value::Array(
        d.bands
            .iter()
            .map(|(from, decimals)| json!({ "fromPrice": from.to_string(), "decimals": decimals }))
            .collect(),
    )
}

fn version_of(row: &PgRow) -> Result<PricingRulesVersion, RateEngineError> {
    let advice = (|| {
        Some(AdviceThresholds {
            far_pct: row.get::<Option<Decimal>, _>("advice_far_pct")?,
            spread_ratio: row.get::<Option<Decimal>, _>("advice_spread_ratio")?,
            spread_gap_pt: row.get::<Option<Decimal>, _>("advice_spread_gap_pt")?,
            market_moved_pct: row.get::<Option<Decimal>, _>("advice_market_moved_pct")?,
        })
    })();
    let rules = PricingRules {
        far_from_anchor_limit_pct: row.get("far_from_anchor_limit_pct"),
        large_move_pct: row.get("large_move_pct"),
        rate_decimals: row
            .get::<Option<Value>, _>("rate_decimals")
            .map(decimals_from_json)
            .transpose()?,
        thb_cash_step: row.get("thb_cash_step"),
        branch_bp_cap: row.get("branch_bp_cap"),
        better_pct_range: pair(row.get("better_pct_min"), row.get("better_pct_max")),
        min_spread_pct_range: pair(row.get("min_spread_pct_min"), row.get("min_spread_pct_max")),
        market_stale_after: row
            .get::<Option<i32>, _>("market_stale_after_minutes")
            .map(|m| Duration::minutes(m.into())),
        advice,
    };
    Ok(PricingRulesVersion {
        version: Some(row.get("version")),
        rules,
        set_by: Some(row.get("set_by")),
        created_at: Some(row.get("created_at")),
    })
}

pub struct PricingRulesStore;

impl PricingRulesStore {
    /// Serialises the organisation's saves until the transaction ends, so a save reads the version it builds on and no
    /// concurrent save is lost.
    pub async fn lock_in(conn: &mut PgConnection, org: Uuid) -> Result<(), RateEngineError> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('pricing_rule_versions:' || $1::text, 0))")
            .bind(org)
            .execute(&mut *conn)
            .await
            .map_err(db)?;
        Ok(())
    }

    /// The organisation's newest version, or [`PricingRulesVersion::not_set`].
    pub async fn current_in(
        conn: &mut PgConnection,
        org: Uuid,
    ) -> Result<PricingRulesVersion, RateEngineError> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM pricing_rule_versions WHERE organization_id = $1 \
             ORDER BY created_at DESC LIMIT 1"
        )))
        .bind(org)
        .fetch_optional(&mut *conn)
        .await
        .map_err(db)?;
        row.as_ref()
            .map_or(Ok(PricingRulesVersion::not_set()), version_of)
    }

    /// Writes `rules` as the organisation's new version. Its `created_at` is `at`, moved just after the newest one if
    /// the clock is behind it, so the newest row is always the last save. Call [`Self::lock_in`] first.
    pub async fn save_in(
        conn: &mut PgConnection,
        org: Uuid,
        rules: &PricingRules,
        by: &str,
        at: DateTime<Utc>,
    ) -> Result<PricingRulesVersion, RateEngineError> {
        let advice = rules.advice.as_ref();
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO pricing_rule_versions ({COLUMNS}, organization_id) \
             SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
                    GREATEST($17, (SELECT max(created_at) + interval '1 microsecond' \
                                   FROM pricing_rule_versions WHERE organization_id = $18)), $18 \
             RETURNING {COLUMNS}"
        )))
        .bind(Uuid::new_v4().to_string())
        .bind(rules.far_from_anchor_limit_pct)
        .bind(rules.large_move_pct)
        .bind(rules.rate_decimals.as_ref().map(decimals_to_json))
        .bind(rules.thb_cash_step)
        .bind(rules.branch_bp_cap)
        .bind(rules.better_pct_range.map(|r| r.min))
        .bind(rules.better_pct_range.map(|r| r.max))
        .bind(rules.min_spread_pct_range.map(|r| r.min))
        .bind(rules.min_spread_pct_range.map(|r| r.max))
        .bind(rules.market_stale_after.map(|d| d.num_minutes() as i32))
        .bind(advice.map(|a| a.far_pct))
        .bind(advice.map(|a| a.spread_ratio))
        .bind(advice.map(|a| a.spread_gap_pt))
        .bind(advice.map(|a| a.market_moved_pct))
        .bind(by)
        .bind(at)
        .bind(org)
        .fetch_one(&mut *conn)
        .await
        .map_err(db)?;
        version_of(&row)
    }

    /// Every version of the organisation, newest first.
    pub async fn history_in(
        conn: &mut PgConnection,
        org: Uuid,
    ) -> Result<Vec<PricingRulesVersion>, RateEngineError> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM pricing_rule_versions WHERE organization_id = $1 ORDER BY created_at DESC"
        )))
        .bind(org)
        .fetch_all(&mut *conn)
        .await
        .map_err(db)?;
        rows.iter().map(version_of).collect()
    }
}
