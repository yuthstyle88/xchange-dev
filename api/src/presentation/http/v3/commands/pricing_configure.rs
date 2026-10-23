//! `pricingConfigure` (pricing-tiers.md §6, §3, §3.1): one currency's override margins, minimum spread and anchor,
//! saved as a new policy version through `PolicyVersionWriter` (no draft). A frame change: EXECUTIVE only.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Bound, CurrencyCode, Money, RatePercent, Side, Sign};
use xc_rate::application::policy::policy_version_writer::{
    PolicyVersionChange, PolicyVersionWriter,
};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::policy::anchor_source::AnchorSource;
use xc_rate::domain::policy::currency_tier_override::{CurrencyTierOverride, TierMargin};
use xc_rate::domain::policy::policy_tiers::PolicyTier;
use xc_rate::domain::policy::rate_policy::RatePolicy;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::org_view::validation_failed;

const BUY_MARGIN_FIELDS: [&str; 4] = [
    "margins.buy-1",
    "margins.buy-2",
    "margins.buy-3",
    "margins.buy-4",
];
const SELL_MARGIN_FIELDS: [&str; 4] = [
    "margins.sell-1",
    "margins.sell-2",
    "margins.sell-3",
    "margins.sell-4",
];

fn parse_rate_percent(field: &'static str, text: &str) -> Result<RatePercent, V3Error> {
    match text.strip_prefix('+') {
        Some(rest) => v3::input::decimal(field, rest, Bound::numeric(9, 4, Sign::NonNegative))?,
        None => v3::input::decimal(field, text, Bound::numeric(9, 4, Sign::Signed))?,
    };
    RatePercent::parse(text).map_err(|_| validation_failed(field))
}

/// One tier's margins as decimal strings (signed, scale 4).
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_configure.ts")]
pub struct PricingTierMarginInput {
    pub tier_no: u8,
    pub buy_margin_pct: String,
    pub sell_margin_pct: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_configure.ts")]
pub struct PricingConfigureInput {
    /// The policy version the page was built on; a newer one ⇒ `conflict`.
    pub policy_version_id: String,
    pub currency: String,
    /// Exactly four tiers 1..=4, or `null` to remove the currency's margin override (back to the branch's).
    pub tiers: Option<Vec<PricingTierMarginInput>>,
    /// 0..=5, or `null` for none.
    pub min_spread_pct: Option<String>,
    /// `BOT` · `MARKET_TT` · `MARKET_SR`, or `null` to follow the branch.
    pub anchor_source: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_configure.ts")]
pub struct PricingConfigureOutput {
    pub id: String,
    pub version: i32,
}

#[derive(Debug, sqlx::FromRow)]
struct CurrentPolicyRow {
    id: Uuid,
    buy_adjust_pct: Decimal,
    sell_adjust_pct: Decimal,
    guards: Value,
    max_reference_age_hours: Option<i32>,
    on_stale: Option<String>,
    stale_limit_thb: Option<Decimal>,
    value_tiers_on: bool,
    sharp_move_alert_pct: Option<Decimal>,
}

pub struct PricingConfigureCommand;

impl CommandAction for PricingConfigureCommand {
    const NAME: &'static str = "pricingConfigure";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ratePolicySave");
    type Input = PricingConfigureInput;
    type Output = PricingConfigureOutput;
    const ROUTE: &'static str = "/api/pricing/configure";
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
            return Err(validation_failed("reason"));
        }

        let policy_version_id = Uuid::parse_str(input.policy_version_id.trim()).map_err(|_| {
            V3Error::new(
                "validationFailed",
                Some("policyVersionId"),
                "invalid policyVersionId",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let currency = CurrencyCode::parse(input.currency.trim()).map_err(|_| {
            V3Error::new(
                "validationFailed",
                Some("currency"),
                "unknown currency code",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let margins = match &input.tiers {
            Some(dtos) => {
                if dtos.len() != 4 {
                    return Err(V3Error::new(
                        "validationFailed",
                        Some("tiers"),
                        "tiers must hold exactly four entries 1..=4 or be null",
                        StatusCode::BAD_REQUEST,
                    ));
                }
                let mut parsed = Vec::with_capacity(dtos.len());
                for (i, dto) in dtos.iter().enumerate() {
                    let idx = match dto.tier_no {
                        1..=4 => usize::from(dto.tier_no - 1),
                        _ => i.min(3),
                    };
                    let buy_margin_pct =
                        parse_rate_percent(BUY_MARGIN_FIELDS[idx], &dto.buy_margin_pct)?;
                    let sell_margin_pct =
                        parse_rate_percent(SELL_MARGIN_FIELDS[idx], &dto.sell_margin_pct)?;
                    parsed.push(TierMargin {
                        tier_no: dto.tier_no,
                        buy_margin_pct,
                        sell_margin_pct,
                    });
                }
                parsed
            }
            None => Vec::new(),
        };

        let min_spread_pct = match input.min_spread_pct.as_deref() {
            Some(s) => {
                v3::input::decimal(
                    "minSpreadPct",
                    s.strip_prefix('+').unwrap_or(s),
                    Bound::numeric(9, 4, Sign::NonNegative).at_most(5),
                )?;
                Some(RatePercent::parse(s).map_err(|_| validation_failed("minSpreadPct"))?)
            }
            None => None,
        };

        let anchor_source = match input.anchor_source.as_deref() {
            Some(s) => Some(AnchorSource::parse(s).ok_or_else(|| {
                V3Error::new(
                    "validationFailed",
                    Some("anchorSource"),
                    "unknown anchorSource",
                    StatusCode::BAD_REQUEST,
                )
            })?),
            None => None,
        };

        if !crate::presentation::http::v3::rate_policy_save::may_change_the_frame(
            who,
            branch,
            Utc::now(),
        ) {
            return Err(V3Error::from(PolicyError::RoleNotAllowedHere));
        }

        let cur_row: Option<CurrentPolicyRow> = sqlx::query_as(
            "SELECT id, buy_adjust_pct, sell_adjust_pct, guards, max_reference_age_hours, \
                    on_stale, stale_limit_thb, value_tiers_on, sharp_move_alert_pct \
             FROM rate_policy_versions \
             WHERE branch_id = $1 AND valid_to IS NULL \
             FOR UPDATE",
        )
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        let Some(cur) = cur_row else {
            return Err(V3Error::new(
                "validationFailed",
                Some("policyVersionId"),
                "branch has no active rate policy version",
                StatusCode::BAD_REQUEST,
            ));
        };

        if cur.id != policy_version_id {
            return Err(V3Error::new(
                "staleVersion",
                None,
                "policyVersionId does not match the current policy version",
                StatusCode::CONFLICT,
            ));
        }

        let override_entry =
            CurrencyTierOverride::new(currency, margins, min_spread_pct, anchor_source).map_err(
                |e| match e {
                    RateEngineError::ValidationFailed(_) => validation_failed("tiers"),
                    _ => server_error(),
                },
            )?;

        if let Some(tiers) = &override_entry.tiers {
            let rate_policy = RatePolicy::new(
                "pricing-configure",
                RatePercent::parse(&format!("{:.4}", cur.buy_adjust_pct))
                    .map_err(|_| server_error())?,
                RatePercent::parse(&format!("{:.4}", cur.sell_adjust_pct))
                    .map_err(|_| server_error())?,
                &cur.guards,
            )
            .map_err(|e| {
                V3Error::new(
                    "validationFailed",
                    Some("tiers"),
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?;

            let code_str = currency.as_str();
            if let Some(guard) = rate_policy.guard_for(code_str, Side::Buy) {
                for tier in tiers {
                    let eff = cur.buy_adjust_pct + tier.buy_margin_pct.value();
                    if eff < guard.min_pct || eff > guard.max_pct {
                        let idx = usize::from(tier.tier_no.saturating_sub(1)).min(3);
                        return Err(V3Error::new(
                            "validationFailed",
                            Some(BUY_MARGIN_FIELDS[idx]),
                            format!(
                                "Tier {} buy margin is outside the {} guard [{}, {}].",
                                tier.tier_no, code_str, guard.min_pct, guard.max_pct
                            ),
                            StatusCode::BAD_REQUEST,
                        ));
                    }
                }
            }
            if let Some(guard) = rate_policy.guard_for(code_str, Side::Sell) {
                for tier in tiers {
                    let eff = cur.sell_adjust_pct + tier.sell_margin_pct.value();
                    if eff < guard.min_pct || eff > guard.max_pct {
                        let idx = usize::from(tier.tier_no.saturating_sub(1)).min(3);
                        return Err(V3Error::new(
                            "validationFailed",
                            Some(SELL_MARGIN_FIELDS[idx]),
                            format!(
                                "Tier {} sell margin is outside the {} guard [{}, {}].",
                                tier.tier_no, code_str, guard.min_pct, guard.max_pct
                            ),
                            StatusCode::BAD_REQUEST,
                        ));
                    }
                }
            }
        }

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

        let mut cur_tiers = Vec::with_capacity(rows.len());
        for r in rows {
            let tier_no: i16 = r.get(0);
            let upper_dec: Option<Decimal> = r.get(1);
            let buy_dec: Decimal = r.get(2);
            let sell_dec: Decimal = r.get(3);
            let upper = match upper_dec {
                Some(u) => Some(Money::parse(&format!("{u:.2}")).map_err(|_| server_error())?),
                None => None,
            };
            let buy_margin =
                RatePercent::parse(&format!("{buy_dec:.4}")).map_err(|_| server_error())?;
            let sell_margin =
                RatePercent::parse(&format!("{sell_dec:.4}")).map_err(|_| server_error())?;
            cur_tiers.push(PolicyTier::new(
                tier_no as u8,
                upper,
                buy_margin,
                sell_margin,
            ));
        }

        let written = PolicyVersionWriter::next(
            tx,
            branch,
            PolicyVersionChange {
                buy_adjust_pct: cur.buy_adjust_pct,
                sell_adjust_pct: cur.sell_adjust_pct,
                guards: cur.guards,
                max_reference_age_hours: cur.max_reference_age_hours,
                on_stale: cur.on_stale,
                stale_limit_thb: cur.stale_limit_thb,
                value_tiers_on: cur.value_tiers_on,
                sharp_move_alert_pct: cur.sharp_move_alert_pct,
                value_tiers: cur_tiers,
                anchor_source: None,
                overrides: Some(vec![override_entry]),
                approved_by: who.user_id.clone(),
                reason: input.reason,
            },
        )
        .await
        .map_err(|e| match e {
            RateEngineError::ValidationFailed(msg) => {
                V3Error::new("validationFailed", None, msg, StatusCode::BAD_REQUEST)
            }
            _ => server_error(),
        })?;

        Ok(PricingConfigureOutput {
            id: written.id.to_string(),
            version: written.version,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(PricingConfigureCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PricingTierMarginInput::export_all(&cfg)?;
    PricingConfigureInput::export_all(&cfg)?;
    PricingConfigureOutput::export_all(&cfg)?;
    Ok(())
}
