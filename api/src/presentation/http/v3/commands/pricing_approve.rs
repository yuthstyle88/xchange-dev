//! `pricingApprove` (pricing-tiers.md §6, P-3 "Use xchange's flow"): for 1..n currencies, one new policy version with
//! their margins (through `PolicyVersionWriter`), then one rate draft with one AUTO line per currency, submitted.
//! In-bound ⇒ published by policy; out-of-bound ⇒ waits in A06 for EXECUTIVE. Idempotent through the command's key.
//!
//! SKELETON (PT-0): `run` (and any service the command needs in `register`) is PT-6's. The name, route, permission
//! (`RatePolicySave`, inherited; `RatesDraft` is checked in `run`) and the input/output types stay.

use std::collections::HashSet;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Bound, CurrencyCode, Money, Rate, RatePercent, Sign};
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::application::draft::reference_base::ReferenceBase;
use xc_rate::application::policy::policy_version_writer::{
    PolicyVersionChange, PolicyVersionWriter,
};
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::domain::draft::rate_draft::{DraftLine, DraftStatus, LineSide};
use xc_rate::domain::policy::currency_tier_override::{CurrencyTierOverride, TierMargin};
use xc_rate::domain::policy::policy_tiers::PolicyTier;
use xc_rate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use xc_rate::domain::pricing::pricing_suggestion::{SuggestionConfig, TierPrice};
use xc_rate::domain::pricing::tier_check::{TierCheck, TierError};
use xc_rate::infrastructure::policy::currency_tier_override_store::CurrencyTierOverrideStore;

use super::commands::{make_approval_service, map_rate_engine_error};
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::org_view::validation_failed;

const TIER_BUY_FIELDS: [&str; 4] = [
    "tiers[0].buy",
    "tiers[1].buy",
    "tiers[2].buy",
    "tiers[3].buy",
];
const TIER_SELL_FIELDS: [&str; 4] = [
    "tiers[0].sell",
    "tiers[1].sell",
    "tiers[2].sell",
    "tiers[3].sell",
];

/// One tier's approved prices (per the currency's unit size), as decimal strings. The server stores the margin that
/// produces each price over the draft path's base (§2, §5.2).
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_approve.ts")]
pub struct PricingTierPriceInput {
    pub tier_no: u8,
    pub buy: String,
    pub sell: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_approve.ts")]
pub struct PricingApproveRowInput {
    pub currency: String,
    /// Exactly four tiers 1..=4.
    pub tiers: Vec<PricingTierPriceInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_approve.ts")]
pub struct PricingApproveInput {
    /// The policy version the board was built on; a newer one ⇒ `conflict`, nothing written.
    pub policy_version_id: String,
    /// One or more currencies (bulk approve).
    pub rows: Vec<PricingApproveRowInput>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_approve.ts")]
pub struct PricingApproveOutput {
    pub policy_version_id: String,
    pub version: i32,
    pub draft_id: String,
    /// The draft's state after submit: `PUBLISHED` when in bound, `PENDING_APPROVAL` when it waits in A06.
    pub draft_state: String,
}

pub struct PricingApproveCommand {
    draft_service: Arc<RateDraftService>,
    publish_service: Arc<PublishService>,
}

impl PricingApproveCommand {
    pub fn new(draft_service: Arc<RateDraftService>, publish_service: Arc<PublishService>) -> Self {
        Self {
            draft_service,
            publish_service,
        }
    }
}

impl CommandAction for PricingApproveCommand {
    const NAME: &'static str = "pricingApprove";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ratePolicySave");
    type Input = PricingApproveInput;
    type Output = PricingApproveOutput;
    const ROUTE: &'static str = "/api/pricing/approve";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        Policy::authorize(
            Some(who),
            EntryPath::Rpc,
            &Action::new("rateDraft"),
            Some(branch),
            chrono::Utc::now(),
        )
        .map_err(V3Error::from)?;

        let reason = input.reason.trim().to_string();
        if reason.is_empty() {
            return Err(validation_failed("reason"));
        }

        if input.rows.is_empty() {
            return Err(V3Error::new(
                "validationFailed",
                Some("rows"),
                "rows must not be empty",
                StatusCode::BAD_REQUEST,
            ));
        }

        let expected_version_id = Uuid::from_str(input.policy_version_id.trim()).map_err(|_| {
            V3Error::new(
                "validationFailed",
                Some("policyVersionId"),
                "invalid policyVersionId",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let mut seen_currencies = HashSet::with_capacity(input.rows.len());
        let mut parsed_rows: Vec<(CurrencyCode, [(Rate, Rate); 4])> =
            Vec::with_capacity(input.rows.len());

        for row in &input.rows {
            let currency = CurrencyCode::parse(row.currency.trim()).map_err(|_| {
                V3Error::new(
                    "validationFailed",
                    Some("currency"),
                    "invalid currency code",
                    StatusCode::BAD_REQUEST,
                )
            })?;
            if !seen_currencies.insert(currency) {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("rows"),
                    "duplicate currency in rows",
                    StatusCode::BAD_REQUEST,
                ));
            }
            if row.tiers.len() != 4 {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("tiers"),
                    "each row must have 4 tiers (1..=4)",
                    StatusCode::BAD_REQUEST,
                ));
            }

            let mut by_tier: [Option<(Rate, Rate)>; 4] = [None, None, None, None];
            for t in &row.tiers {
                if !(1..=4).contains(&t.tier_no) {
                    return Err(V3Error::new(
                        "validationFailed",
                        Some("tiers"),
                        "tierNo must be between 1 and 4",
                        StatusCode::BAD_REQUEST,
                    ));
                }
                let idx = (t.tier_no - 1) as usize;
                if by_tier[idx].is_some() {
                    return Err(V3Error::new(
                        "validationFailed",
                        Some("tiers"),
                        "duplicate tierNo",
                        StatusCode::BAD_REQUEST,
                    ));
                }
                let rate_bound =
                    Bound::numeric(20, 8, Sign::Positive).at_most(Money::MAX_INPUT_UNITS);
                v3::input::decimal(TIER_BUY_FIELDS[idx], t.buy.trim(), rate_bound)?;
                let buy =
                    Rate::parse(t.buy.trim()).map_err(|_| validation_failed(TIER_BUY_FIELDS[idx]))?;
                v3::input::decimal(TIER_SELL_FIELDS[idx], t.sell.trim(), rate_bound)?;
                let sell = Rate::parse(t.sell.trim())
                    .map_err(|_| validation_failed(TIER_SELL_FIELDS[idx]))?;
                by_tier[idx] = Some((buy, sell));
            }

            parsed_rows.push((
                currency,
                [
                    by_tier[0].unwrap(),
                    by_tier[1].unwrap(),
                    by_tier[2].unwrap(),
                    by_tier[3].unwrap(),
                ],
            ));
        }

        let cur_row = sqlx::query(
            "SELECT id, buy_adjust_pct, sell_adjust_pct, guards, \
                    max_reference_age_hours, on_stale, stale_limit_thb, \
                    value_tiers_on, sharp_move_alert_pct \
             FROM rate_policy_versions \
             WHERE branch_id = $1 AND valid_to IS NULL \
             FOR UPDATE",
        )
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| server_error())?
        .ok_or_else(|| {
            V3Error::new(
                "validationFailed",
                Some("policyVersionId"),
                "no active policy version for branch",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let cur_id: Uuid = cur_row.get("id");
        if cur_id != expected_version_id {
            return Err(V3Error::new(
                "staleVersion",
                Some("policyVersionId"),
                "policy version has changed",
                StatusCode::CONFLICT,
            ));
        }

        let buy_adjust_pct: Decimal = cur_row.get("buy_adjust_pct");
        let sell_adjust_pct: Decimal = cur_row.get("sell_adjust_pct");
        let guards: serde_json::Value = cur_row.get("guards");
        let max_reference_age_hours: Option<i32> = cur_row.get("max_reference_age_hours");
        let on_stale: Option<String> = cur_row.get("on_stale");
        let stale_limit_thb: Option<Decimal> = cur_row.get("stale_limit_thb");
        let value_tiers_on: bool = cur_row.get("value_tiers_on");
        let sharp_move_alert_pct: Option<Decimal> = cur_row.get("sharp_move_alert_pct");

        let tier_rows = sqlx::query(
            "SELECT tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct \
             FROM rate_policy_value_tiers \
             WHERE policy_version_id = $1 \
             ORDER BY tier_no ASC",
        )
        .bind(cur_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        let mut value_tiers = Vec::with_capacity(tier_rows.len());
        for tr in tier_rows {
            let tier_no: i16 = tr.get("tier_no");
            let upper_thb: Option<Decimal> = tr.get("upper_inclusive_thb");
            let buy_margin: Decimal = tr.get("buy_margin_pct");
            let sell_margin: Decimal = tr.get("sell_margin_pct");
            let upper_inclusive_thb = match upper_thb {
                Some(d) => Some(Money::parse(&d.to_string()).map_err(|_| server_error())?),
                None => None,
            };
            value_tiers.push(PolicyTier {
                tier_no: tier_no as u8,
                upper_inclusive_thb,
                buy_margin_pct: RatePercent::parse(&buy_margin.to_string())
                    .map_err(|_| server_error())?,
                sell_margin_pct: RatePercent::parse(&sell_margin.to_string())
                    .map_err(|_| server_error())?,
            });
        }

        let existing_overrides = CurrencyTierOverrideStore::for_version(tx, &cur_id)
            .await
            .map_err(map_rate_engine_error)?;

        let hundred = Decimal::from(100);
        let mut overrides = Vec::with_capacity(parsed_rows.len());
        let mut draft_lines = Vec::with_capacity(parsed_rows.len());

        for (currency, tier_prices) in &parsed_rows {
            let cur_unit: Decimal =
                sqlx::query_scalar("SELECT unit_size FROM currencies WHERE code = $1")
                    .bind(currency.as_str())
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|_| server_error())?
                    .unwrap_or(Decimal::ONE);

            let market_rows = sqlx::query(
                "SELECT DISTINCT ON (l.reference_type) \
                     l.reference_type, \
                     l.unit_size, \
                     l.buy, \
                     l.sell, \
                     b.fetched_at \
                 FROM rate_reference_lines l \
                 JOIN rate_reference_batches b ON b.id = l.batch_id \
                 WHERE trim(l.currency_code) = $1 \
                   AND l.reference_type IN ('MARKET_TT', 'MARKET_SR') \
                 ORDER BY l.reference_type, b.fetched_at DESC, b.id DESC",
            )
            .bind(currency.as_str())
            .fetch_all(&mut **tx)
            .await
            .map_err(|_| server_error())?;

            let mut domain_quotes = Vec::with_capacity(market_rows.len());
            for r in market_rows {
                let ref_type: String = r.get("reference_type");
                let unit_size: Decimal = r.get("unit_size");
                let buy: Option<Decimal> = r.get("buy");
                let sell: Option<Decimal> = r.get("sell");
                let fetched_at: chrono::DateTime<chrono::Utc> = r.get("fetched_at");
                let market_source = match ref_type.as_str() {
                    "MARKET_TT" => MarketSource::Tt,
                    "MARKET_SR" => MarketSource::SuperRich,
                    _ => continue,
                };
                domain_quotes.push(MarketQuote {
                    source: market_source,
                    buy: buy.and_then(|d| Rate::parse(&d.to_string()).ok()),
                    sell: sell.and_then(|d| Rate::parse(&d.to_string()).ok()),
                    unit_size,
                    fetched_at,
                    stale: false,
                });
            }

            let (anchor, _) = MarketQuote::anchor_for(&domain_quotes);
            let check_tiers: [TierPrice; 4] = std::array::from_fn(|idx| TierPrice {
                tier_no: (idx + 1) as u8,
                buy: Some(tier_prices[idx].0),
                sell: Some(tier_prices[idx].1),
            });
            let tier_errors = TierCheck::validate(
                &check_tiers,
                cur_unit,
                anchor,
                &SuggestionConfig::v9_default(),
            );
            if let Some(first_err) = tier_errors.first() {
                let (field, msg) = match first_err {
                    TierError::Count => ("tiers", "Must have 4 tiers.".to_string()),
                    TierError::NotPositive { tier } => {
                        let idx = usize::from(tier.saturating_sub(1)).min(3);
                        (
                            TIER_BUY_FIELDS[idx],
                            format!("Tier {tier} buy and sell rates must be positive."),
                        )
                    }
                    TierError::SellNotAboveBuy { tier } => {
                        let idx = usize::from(tier.saturating_sub(1)).min(3);
                        (
                            TIER_SELL_FIELDS[idx],
                            format!("Tier {tier} sell rate must be above the buy rate."),
                        )
                    }
                    TierError::BuyBelowPrevious { tier } => {
                        let idx = usize::from(tier.saturating_sub(1)).min(3);
                        (
                            TIER_BUY_FIELDS[idx],
                            format!("Tier {tier} buy rate must not be below the previous tier."),
                        )
                    }
                    TierError::SellAbovePrevious { tier } => {
                        let idx = usize::from(tier.saturating_sub(1)).min(3);
                        (
                            TIER_SELL_FIELDS[idx],
                            format!("Tier {tier} sell rate must not be above the previous tier."),
                        )
                    }
                    TierError::FarFromAnchor { limit_pct, .. } => (
                        TIER_BUY_FIELDS[0],
                        format!("Tier 1 rate is more than {limit_pct}% from the market anchor."),
                    ),
                };
                return Err(V3Error::new(
                    "validationFailed",
                    Some(field),
                    msg,
                    StatusCode::BAD_REQUEST,
                ));
            }

            let ref_id = ReferenceBase::newest_in(tx, branch, currency)
                .await
                .map_err(map_rate_engine_error)?
                .ok_or_else(|| {
                    V3Error::new(
                        "validationFailed",
                        Some("rows"),
                        "no reference base for currency",
                        StatusCode::BAD_REQUEST,
                    )
                })?;

            let ref_row =
                sqlx::query("SELECT unit_size, buy, sell FROM rate_reference_lines WHERE id = $1")
                    .bind(ref_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|_| server_error())?
                    .ok_or_else(|| {
                        V3Error::new(
                            "validationFailed",
                            Some("rows"),
                            "reference line not found",
                            StatusCode::BAD_REQUEST,
                        )
                    })?;

            let ref_unit: Decimal = ref_row.get("unit_size");
            let ref_buy: Option<Decimal> = ref_row.get("buy");
            let ref_sell: Option<Decimal> = ref_row.get("sell");
            let (Some(ref_buy), Some(ref_sell)) = (ref_buy, ref_sell) else {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("rows"),
                    "reference line missing buy or sell",
                    StatusCode::BAD_REQUEST,
                ));
            };

            if ref_unit <= Decimal::ZERO || cur_unit <= Decimal::ZERO {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("rows"),
                    "invalid unit size",
                    StatusCode::BAD_REQUEST,
                ));
            }

            let base_buy = ref_buy * cur_unit / ref_unit;
            let base_sell = ref_sell * cur_unit / ref_unit;
            if base_buy <= Decimal::ZERO || base_sell <= Decimal::ZERO {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("rows"),
                    "reference base must be positive",
                    StatusCode::BAD_REQUEST,
                ));
            }

            let mut tier_margins = Vec::with_capacity(4);
            for (idx, (price_buy, price_sell)) in tier_prices.iter().enumerate() {
                let tier_no = (idx + 1) as u8;
                let buy_margin_dec = ((price_buy.value() / base_buy - Decimal::ONE) * hundred
                    - buy_adjust_pct)
                    .round_dp_with_strategy(4, RoundingStrategy::MidpointAwayFromZero);
                let sell_margin_dec = ((price_sell.value() / base_sell - Decimal::ONE) * hundred
                    - sell_adjust_pct)
                    .round_dp_with_strategy(4, RoundingStrategy::MidpointAwayFromZero);

                let buy_margin_pct = RatePercent::parse(&buy_margin_dec.to_string())
                    .map_err(|_| validation_failed(TIER_BUY_FIELDS[idx]))?;
                let sell_margin_pct = RatePercent::parse(&sell_margin_dec.to_string())
                    .map_err(|_| validation_failed(TIER_SELL_FIELDS[idx]))?;

                tier_margins.push(TierMargin {
                    tier_no,
                    buy_margin_pct,
                    sell_margin_pct,
                });
            }

            let existing = existing_overrides.iter().find(|o| o.currency == *currency);
            let min_spread_pct = existing.and_then(|o| o.min_spread_pct);
            let anchor_source = existing.and_then(|o| o.anchor_source);

            let ov =
                CurrencyTierOverride::new(*currency, tier_margins, min_spread_pct, anchor_source)
                    .map_err(map_rate_engine_error)?;
            overrides.push(ov);

            draft_lines.push(DraftLine {
                currency: *currency,
                buy: LineSide::Auto,
                sell: LineSide::Auto,
                reason: Some(reason.clone()),
            });
        }

        let written = PolicyVersionWriter::next(
            tx,
            branch,
            PolicyVersionChange {
                buy_adjust_pct,
                sell_adjust_pct,
                guards,
                max_reference_age_hours,
                on_stale,
                stale_limit_thb,
                value_tiers_on,
                sharp_move_alert_pct,
                value_tiers,
                anchor_source: None,
                overrides: Some(overrides),
                approved_by: who.user_id.clone(),
                reason: reason.clone(),
            },
        )
        .await
        .map_err(map_rate_engine_error)?;

        let draft = self
            .draft_service
            .create(tx, branch, who, draft_lines)
            .await
            .map_err(map_rate_engine_error)?;

        let outcome = self
            .draft_service
            .submit(tx, branch, who, &draft.id, key.as_str())
            .await
            .map_err(map_rate_engine_error)?;

        let draft_state = match outcome.status {
            DraftStatus::Approved => {
                let expected_pub_version: Option<i32> = sqlx::query_scalar(
                    "SELECT p.version FROM branches b \
                     JOIN rate_publications p ON p.id = b.current_publication_id \
                     WHERE b.id = $1",
                )
                .bind(branch.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|_| server_error())?;

                self.publish_service
                    .publish(tx, branch, who, &draft.id, expected_pub_version)
                    .await
                    .map_err(map_rate_engine_error)?;
                "PUBLISHED"
            }
            DraftStatus::PendingApproval => "PENDING_APPROVAL",
            _ => return Err(server_error()),
        };

        Ok(PricingApproveOutput {
            policy_version_id: written.id.to_string(),
            version: written.version,
            draft_id: draft.id.to_string(),
            draft_state: draft_state.to_string(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let approval_service = make_approval_service(state.db.clone());
    let draft_service = Arc::new(RateDraftService::new(state.db.clone(), approval_service));
    let publish_service = Arc::new(PublishService::new(state.db.clone()));
    CommandEndpoint::register(
        cfg,
        Arc::new(PricingApproveCommand::new(draft_service, publish_service)),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PricingTierPriceInput::export_all(&cfg)?;
    PricingApproveRowInput::export_all(&cfg)?;
    PricingApproveInput::export_all(&cfg)?;
    PricingApproveOutput::export_all(&cfg)?;
    Ok(())
}
