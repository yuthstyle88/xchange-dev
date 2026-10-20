//! Rate policy save command (§4.7, §6).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::rate_policy::ValueTierDto;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::input;
use xc_kernel::money::{Bound, Money, RatePercent, Side, Sign};
use xc_rate::application::policy::policy_version_writer::{
    PolicyVersionChange, PolicyVersionWriter,
};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::policy::anchor_source::AnchorSource;
use xc_rate::domain::policy::policy_tiers::{PolicyTier, PolicyTiers};
use xc_rate::domain::policy::rate_policy::RatePolicy;
use xc_rate::infrastructure::policy::currency_tier_override_store::CurrencyTierOverrideStore;

fn deserialize_some<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_policy_save.ts")]
pub struct RatePolicySaveInput {
    pub buy_adjust_pct: String,
    pub sell_adjust_pct: String,
    #[ts(type = "any")]
    pub guards: Value,
    pub max_reference_age_hours: Option<i32>,
    pub on_stale: Option<String>,
    pub stale_limit_thb: Option<String>,
    pub reason: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub value_tiers_on: Option<bool>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub value_tiers: Option<Vec<ValueTierDto>>,
    #[serde(default, deserialize_with = "deserialize_some")]
    #[ts(optional = nullable, type = "string | null")]
    pub sharp_move_alert_pct: Option<Value>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub anchor_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_policy_save.ts")]
pub struct RatePolicySaveOutput {
    pub id: String,
    pub version: i32,
}

#[derive(Debug, sqlx::FromRow)]
struct CurrentPolicyRow {
    id: Uuid,
    guards: Value,
    max_reference_age_hours: Option<i32>,
    on_stale: Option<String>,
    stale_limit_thb: Option<Decimal>,
    value_tiers_on: bool,
    sharp_move_alert_pct: Option<Decimal>,
    anchor_source: String,
}

pub struct RatePolicySaveCommand;

impl CommandAction for RatePolicySaveCommand {
    const NAME: &'static str = "ratePolicySave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ratePolicySave");
    type Input = RatePolicySaveInput;
    type Output = RatePolicySaveOutput;
    const ROUTE: &'static str = "/api/rate-policies";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        if input.reason.trim().is_empty() {
            return Err(V3Error::new(
                "validationFailed",
                Some("reason"),
                "reason cannot be empty",
                StatusCode::BAD_REQUEST,
            ));
        }

        if let Some(on_stale) = input.on_stale.as_deref() {
            input::one_of("onStale", on_stale, &["STOP", "LIMIT"])?;
        }

        let input_anchor = match input.anchor_source.as_deref() {
            Some(s) => Some(AnchorSource::parse(s).ok_or_else(|| {
                V3Error::new(
                    "validationFailed",
                    Some("anchorSource"),
                    format!("invalid anchorSource: {s}"),
                    StatusCode::BAD_REQUEST,
                )
            })?),
            None => None,
        };

        let buy_pct = input::decimal(
            "buyAdjustPct",
            &input.buy_adjust_pct,
            Bound::numeric(9, 4, Sign::Signed),
        )?;
        let sell_pct = input::decimal(
            "sellAdjustPct",
            &input.sell_adjust_pct,
            Bound::numeric(9, 4, Sign::Signed),
        )?;

        let stale_limit = match input.stale_limit_thb.as_deref() {
            Some(s) => Some(input::decimal(
                "staleLimitThb",
                s,
                Bound::numeric(20, 2, Sign::Signed),
            )?),
            None => None,
        };

        let input_sharp_move: Option<Option<Decimal>> = match &input.sharp_move_alert_pct {
            Some(Value::Null) => Some(None),
            Some(Value::String(s)) => Some(Some(input::decimal(
                "sharpMoveAlertPct",
                s,
                Bound::numeric(9, 4, Sign::Positive),
            )?)),
            Some(Value::Number(n)) => Some(Some(input::decimal(
                "sharpMoveAlertPct",
                &n.to_string(),
                Bound::numeric(9, 4, Sign::Positive),
            )?)),
            Some(_) => {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("sharpMoveAlertPct"),
                    "invalid sharpMoveAlertPct format",
                    StatusCode::BAD_REQUEST,
                ));
            }
            None => None,
        };

        let input_tiers: Option<Vec<PolicyTier>> = match &input.value_tiers {
            Some(dtos) => {
                if dtos.is_empty() {
                    Some(Vec::new())
                } else {
                    let mut tiers = Vec::with_capacity(dtos.len());
                    for dto in dtos {
                        let upper = match &dto.upper_inclusive_thb {
                            Some(s) if !s.trim().is_empty() => {
                                Some(Money::parse(s).map_err(|e| {
                                    V3Error::new(
                                        "validationFailed",
                                        Some("valueTiers"),
                                        format!(
                                            "invalid upper bound in tier {}: {}",
                                            dto.tier_no, e
                                        ),
                                        StatusCode::BAD_REQUEST,
                                    )
                                })?)
                            }
                            _ => None,
                        };
                        let buy_margin = RatePercent::parse(&dto.buy_margin_pct).map_err(|e| {
                            V3Error::new(
                                "validationFailed",
                                Some("valueTiers"),
                                format!("invalid buy margin in tier {}: {}", dto.tier_no, e),
                                StatusCode::BAD_REQUEST,
                            )
                        })?;
                        let sell_margin =
                            RatePercent::parse(&dto.sell_margin_pct).map_err(|e| {
                                V3Error::new(
                                    "validationFailed",
                                    Some("valueTiers"),
                                    format!("invalid sell margin in tier {}: {}", dto.tier_no, e),
                                    StatusCode::BAD_REQUEST,
                                )
                            })?;
                        tiers.push(PolicyTier::new(dto.tier_no, upper, buy_margin, sell_margin));
                    }
                    let pt = PolicyTiers::new(tiers).map_err(|e| {
                        V3Error::new(
                            "validationFailed",
                            Some("valueTiers"),
                            e.to_string(),
                            StatusCode::BAD_REQUEST,
                        )
                    })?;
                    Some(pt.tiers().to_vec())
                }
            }
            None => None,
        };

        // Query current version with valid_to IS NULL
        let cur_row: Option<CurrentPolicyRow> = sqlx::query_as(
            "SELECT id, guards, max_reference_age_hours, on_stale, stale_limit_thb, \
                    value_tiers_on, sharp_move_alert_pct, anchor_source \
             FROM rate_policy_versions \
             WHERE branch_id = $1 AND valid_to IS NULL \
             FOR UPDATE",
        )
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        let cur_tiers = match &cur_row {
            Some(cur) => {
                let rows = sqlx::query(
                    "SELECT tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct \
                     FROM rate_policy_value_tiers \
                     WHERE policy_version_id = $1 \
                     ORDER BY tier_no ASC",
                )
                .bind(cur.id)
                .fetch_all(&mut **tx)
                .await
                .map_err(|_| server_error())?;

                let mut tiers = Vec::with_capacity(rows.len());
                for r in rows {
                    let tier_no: i16 = r.get(0);
                    let upper_dec: Option<Decimal> = r.get(1);
                    let buy_dec: Decimal = r.get(2);
                    let sell_dec: Decimal = r.get(3);
                    let upper = match upper_dec {
                        Some(u) => {
                            Some(Money::parse(&format!("{u:.2}")).map_err(|_| server_error())?)
                        }
                        None => None,
                    };
                    let buy_margin =
                        RatePercent::parse(&format!("{buy_dec:.4}")).map_err(|_| server_error())?;
                    let sell_margin = RatePercent::parse(&format!("{sell_dec:.4}"))
                        .map_err(|_| server_error())?;
                    tiers.push(PolicyTier::new(
                        tier_no as u8,
                        upper,
                        buy_margin,
                        sell_margin,
                    ));
                }
                tiers
            }
            None => Vec::new(),
        };

        let resolved_value_tiers_on = input
            .value_tiers_on
            .unwrap_or_else(|| cur_row.as_ref().map(|c| c.value_tiers_on).unwrap_or(false));

        let resolved_sharp_move_alert_pct = match input_sharp_move {
            Some(explicit) => explicit,
            None => cur_row.as_ref().and_then(|c| c.sharp_move_alert_pct),
        };

        let resolved_tiers = match input_tiers {
            Some(tiers) => tiers,
            None => cur_tiers.clone(),
        };

        if resolved_value_tiers_on {
            if resolved_tiers.len() != 4 {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("valueTiers"),
                    "exactly four policy tier rows are required when valueTiersOn is true",
                    StatusCode::BAD_REQUEST,
                ));
            }
            let rate_policy = RatePolicy::new(
                "rate-policy-save",
                RatePercent::parse(&format!("{buy_pct:.4}")).unwrap_or(RatePercent::ZERO),
                RatePercent::parse(&format!("{sell_pct:.4}")).unwrap_or(RatePercent::ZERO),
                &input.guards,
            )
            .map_err(|e| {
                V3Error::new(
                    "validationFailed",
                    Some("valueTiers"),
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?;

            if let Some(guards_obj) = input.guards.as_object() {
                for currency in guards_obj.keys() {
                    if let Some(guard) = rate_policy.guard_for(currency, Side::Buy) {
                        for tier in &resolved_tiers {
                            let eff = buy_pct + tier.buy_margin_pct.value();
                            if eff < guard.min_pct || eff > guard.max_pct {
                                return Err(V3Error::new(
                                    "validationFailed",
                                    Some("valueTiers"),
                                    format!(
                                        "tier {} buy margin outside {} guard [{}, {}]",
                                        tier.tier_no, currency, guard.min_pct, guard.max_pct
                                    ),
                                    StatusCode::BAD_REQUEST,
                                ));
                            }
                        }
                    }
                    if let Some(guard) = rate_policy.guard_for(currency, Side::Sell) {
                        for tier in &resolved_tiers {
                            let eff = sell_pct + tier.sell_margin_pct.value();
                            if eff < guard.min_pct || eff > guard.max_pct {
                                return Err(V3Error::new(
                                    "validationFailed",
                                    Some("valueTiers"),
                                    format!(
                                        "tier {} sell margin outside {} guard [{}, {}]",
                                        tier.tier_no, currency, guard.min_pct, guard.max_pct
                                    ),
                                    StatusCode::BAD_REQUEST,
                                ));
                            }
                        }
                    }
                }
            }
        }

        // Without an EXECUTIVE row here (i.e. a RATE_MANAGER), only the adjustments may change (#1030 §3.9):
        if !may_change_the_frame(who, branch, Utc::now()) {
            let Some(cur) = &cur_row else {
                return Err(V3Error::from(PolicyError::RoleNotAllowedHere));
            };

            if input.guards != cur.guards
                || input.max_reference_age_hours != cur.max_reference_age_hours
                || input.on_stale != cur.on_stale
                || stale_limit != cur.stale_limit_thb
                || resolved_value_tiers_on != cur.value_tiers_on
                || resolved_sharp_move_alert_pct != cur.sharp_move_alert_pct
                || resolved_tiers != cur_tiers
                || input_anchor.is_some_and(|a| a.as_str() != cur.anchor_source)
            {
                return Err(V3Error::from(PolicyError::RoleNotAllowedHere));
            }
        }

        for tier in &resolved_tiers {
            if let Some(m) = tier.upper_inclusive_thb {
                m.try_minor()
                    .map(|cents| Decimal::new(cents, 2))
                    .map_err(|_| {
                        V3Error::new(
                            "validationFailed",
                            Some("valueTiers"),
                            format!(
                                "tier {} upper bound exceeds maximum allowed value",
                                tier.tier_no
                            ),
                            StatusCode::BAD_REQUEST,
                        )
                    })?;
            }
        }

        if let Some(cur) = &cur_row {
            let overrides = CurrencyTierOverrideStore::for_version(tx, &cur.id)
                .await
                .map_err(|_| server_error())?;
            if overrides.iter().any(|ov| ov.tiers.is_some()) {
                let rate_policy = RatePolicy::new(
                    "rate-policy-save",
                    RatePercent::parse(&format!("{buy_pct:.4}")).map_err(|_| server_error())?,
                    RatePercent::parse(&format!("{sell_pct:.4}")).map_err(|_| server_error())?,
                    &input.guards,
                )
                .map_err(|e| {
                    V3Error::new(
                        "validationFailed",
                        Some("valueTiers"),
                        e.to_string(),
                        StatusCode::BAD_REQUEST,
                    )
                })?;

                for ov in &overrides {
                    let Some(tiers) = &ov.tiers else {
                        continue;
                    };
                    let currency = ov.currency.as_str();
                    if let Some(guard) = rate_policy.guard_for(currency, Side::Buy) {
                        for tier in tiers {
                            let eff = buy_pct + tier.buy_margin_pct.value();
                            if eff < guard.min_pct || eff > guard.max_pct {
                                return Err(V3Error::new(
                                    "validationFailed",
                                    Some("valueTiers"),
                                    format!(
                                        "override {} tier {} buy margin outside guard [{}, {}]",
                                        currency, tier.tier_no, guard.min_pct, guard.max_pct
                                    ),
                                    StatusCode::BAD_REQUEST,
                                ));
                            }
                        }
                    }
                    if let Some(guard) = rate_policy.guard_for(currency, Side::Sell) {
                        for tier in tiers {
                            let eff = sell_pct + tier.sell_margin_pct.value();
                            if eff < guard.min_pct || eff > guard.max_pct {
                                return Err(V3Error::new(
                                    "validationFailed",
                                    Some("valueTiers"),
                                    format!(
                                        "override {} tier {} sell margin outside guard [{}, {}]",
                                        currency, tier.tier_no, guard.min_pct, guard.max_pct
                                    ),
                                    StatusCode::BAD_REQUEST,
                                ));
                            }
                        }
                    }
                }
            }
        }

        let change = PolicyVersionChange {
            buy_adjust_pct: buy_pct,
            sell_adjust_pct: sell_pct,
            guards: input.guards,
            max_reference_age_hours: input.max_reference_age_hours,
            on_stale: input.on_stale,
            stale_limit_thb: stale_limit,
            value_tiers_on: resolved_value_tiers_on,
            sharp_move_alert_pct: resolved_sharp_move_alert_pct,
            value_tiers: resolved_tiers,
            anchor_source: input_anchor,
            overrides: None,
            approved_by: who.user_id.clone(),
            reason: input.reason,
        };

        let written = PolicyVersionWriter::next(tx, branch, change)
            .await
            .map_err(|e| match e {
                RateEngineError::ValidationFailed(msg) => V3Error::new(
                    "validationFailed",
                    Some("valueTiers"),
                    msg,
                    StatusCode::BAD_REQUEST,
                ),
                _ => server_error(),
            })?;

        Ok(RatePolicySaveOutput {
            id: written.id.to_string(),
            version: written.version,
        })
    }
}

/// The guards and the stale settings may change iff the caller has a valid-now EXECUTIVE row covering `branch`
/// (#1030 §3.9; design §3.3: the old owner is EXECUTIVE + ADMIN). `pub(crate)` for `pricingConfigure`
/// (pricing-tiers.md §3: override margins and the anchor are frame, under the same rule).
pub(crate) fn may_change_the_frame(who: &Actor, branch: &BranchId, now: DateTime<Utc>) -> bool {
    who.roles.iter().any(|r| {
        r.role == Role::Executive
            && r.valid_from <= now
            && r.valid_to.is_none_or(|to| now < to)
            && r.branch.as_ref().is_none_or(|b| b == branch)
    })
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(RatePolicySaveCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    RatePolicySaveInput::export_all(&cfg)?;
    RatePolicySaveOutput::export_all(&cfg)?;
    Ok(())
}
