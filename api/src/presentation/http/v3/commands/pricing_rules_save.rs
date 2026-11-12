//! `pricingRulesSave` (owner-set-values.md §2; xchange#6555): the Owner saves the organisation's pricing rules as a
//! new `pricing_rule_versions` row. Needs `Permission::PricingRulesSave` (EXECUTIVE only by seed).
//!
//! A key left out keeps the current version's value; a key sent as `null` clears it ("not set"). Saves of one
//! organisation are serialised, so two at once both land, in order, each building on the one before.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::{Duration, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_rate::domain::policy::pricing_rules::{AdviceThresholds, PctRange, RateDecimals};
use xc_rate::infrastructure::pricing::pricing_rules_store::PricingRulesStore;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::pricing_rules::{
    AdviceThresholdsDto, PctRangeDto, PricingRulesVersionDto, RateDecimalsBandDto, version_dtos,
};

/// Every key is optional: absent keeps the current value, `null` clears it.
#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_rules_save.ts")]
pub struct PricingRulesSaveInput {
    /// %, > 0.
    #[serde(default, deserialize_with = "some_or_null")]
    #[ts(optional)]
    pub far_from_anchor_limit_pct: Option<Option<String>>,
    /// %, > 0.
    #[serde(default, deserialize_with = "some_or_null")]
    #[ts(optional)]
    pub large_move_pct: Option<Option<String>>,
    /// Rising `fromPrice` starting at `"0"`, `decimals` 0..=4.
    #[serde(default, deserialize_with = "some_or_null")]
    #[ts(optional)]
    pub rate_decimals: Option<Option<Vec<RateDecimalsBandDto>>>,
    /// Baht, > 0, at most 2 places.
    #[serde(default, deserialize_with = "some_or_null")]
    #[ts(optional)]
    pub thb_cash_step: Option<Option<String>>,
    /// Basis points, > 0.
    #[serde(default, deserialize_with = "some_or_null")]
    #[ts(optional)]
    pub branch_bp_cap: Option<Option<i32>>,
    /// %, `min` ≤ `max`.
    #[serde(default, deserialize_with = "some_or_null")]
    #[ts(optional)]
    pub better_pct_range: Option<Option<PctRangeDto>>,
    /// %, 0 ≤ `min` ≤ `max`.
    #[serde(default, deserialize_with = "some_or_null")]
    #[ts(optional)]
    pub min_spread_pct_range: Option<Option<PctRangeDto>>,
    /// Whole minutes, > 0.
    #[serde(default, deserialize_with = "some_or_null")]
    #[ts(optional)]
    pub market_stale_after_minutes: Option<Option<i32>>,
    /// Each > 0.
    #[serde(default, deserialize_with = "some_or_null")]
    #[ts(optional)]
    pub advice: Option<Option<AdviceThresholdsDto>>,
}

/// `Some(None)` for an explicit `null`, `None` (via `default`) when the key is absent.
fn some_or_null<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

fn invalid(field: &'static str, message: impl Into<String>) -> V3Error {
    V3Error::new(
        "validationFailed",
        Some(field),
        message.into(),
        StatusCode::BAD_REQUEST,
    )
}

fn decimal(field: &'static str, s: &str) -> Result<Decimal, V3Error> {
    s.trim()
        .parse::<Decimal>()
        .map_err(|e| invalid(field, format!("ตัวเลขไม่ถูกต้อง: {e}")))
}

fn pct_range(field: &'static str, r: &PctRangeDto) -> Result<PctRange, V3Error> {
    Ok(PctRange {
        min: decimal(field, &r.min)?,
        max: decimal(field, &r.max)?,
    })
}

/// Applies the sent key onto `current`: absent keeps it, `null` clears it, a value is parsed with `parse`.
fn apply<T, U>(
    current: &mut Option<U>,
    sent: Option<Option<T>>,
    parse: impl FnOnce(T) -> Result<U, V3Error>,
) -> Result<(), V3Error> {
    if let Some(value) = sent {
        *current = value.map(parse).transpose()?;
    }
    Ok(())
}

pub struct PricingRulesSaveCommand;

impl CommandAction for PricingRulesSaveCommand {
    const NAME: &'static str = "pricingRulesSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "pricingRulesSave");
    type Input = PricingRulesSaveInput;
    type Output = PricingRulesVersionDto;
    const ROUTE: &'static str = "/api/pricing/rules";
    const METHOD: Method = Method::POST;

    /// `PricingRulesSave` at the seam, else `403`; a value that does not parse or breaks a rule's check ⇒ `400
    /// validationFailed` naming the field, nothing written; else the new version, with the saver's name.
    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let org: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
            .bind(branch.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(|_| server_error())?
            .ok_or_else(server_error)?;

        PricingRulesStore::lock_in(tx, org)
            .await
            .map_err(|_| server_error())?;
        let mut rules = PricingRulesStore::current_in(tx, org)
            .await
            .map_err(|_| server_error())?
            .rules;

        apply(
            &mut rules.far_from_anchor_limit_pct,
            input.far_from_anchor_limit_pct,
            |s| decimal("farFromAnchorLimitPct", &s),
        )?;
        apply(&mut rules.large_move_pct, input.large_move_pct, |s| {
            decimal("largeMovePct", &s)
        })?;
        apply(&mut rules.rate_decimals, input.rate_decimals, |bands| {
            Ok(RateDecimals {
                bands: bands
                    .iter()
                    .map(|b| Ok((decimal("rateDecimals", &b.from_price)?, b.decimals)))
                    .collect::<Result<_, V3Error>>()?,
            })
        })?;
        apply(&mut rules.thb_cash_step, input.thb_cash_step, |s| {
            decimal("thbCashStep", &s)
        })?;
        apply(&mut rules.branch_bp_cap, input.branch_bp_cap, Ok)?;
        apply(&mut rules.better_pct_range, input.better_pct_range, |r| {
            pct_range("betterPctRange", &r)
        })?;
        apply(
            &mut rules.min_spread_pct_range,
            input.min_spread_pct_range,
            |r| pct_range("minSpreadPctRange", &r),
        )?;
        apply(
            &mut rules.market_stale_after,
            input.market_stale_after_minutes,
            |m| {
                if m <= 0 {
                    return Err(invalid(
                        "marketStaleAfterMinutes",
                        "ค่าต้องเป็นจำนวนนาทีเต็มที่มากกว่า 0",
                    ));
                }
                Ok(Duration::minutes(m.into()))
            },
        )?;
        apply(&mut rules.advice, input.advice, |a| {
            Ok(AdviceThresholds {
                far_pct: decimal("advice", &a.far_pct)?,
                spread_ratio: decimal("advice", &a.spread_ratio)?,
                spread_gap_pt: decimal("advice", &a.spread_gap_pt)?,
                market_moved_pct: decimal("advice", &a.market_moved_pct)?,
            })
        })?;

        rules.validate().map_err(|e| invalid(e.field, e.message))?;

        let saved = PricingRulesStore::save_in(tx, org, &rules, &who.user_id, Utc::now())
            .await
            .map_err(|_| server_error())?;
        version_dtos(tx, &[saved])
            .await
            .map_err(|_| server_error())
            .map(|mut v| v.remove(0))
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(PricingRulesSaveCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PricingRulesSaveInput::export_all(&cfg)?;
    Ok(())
}
