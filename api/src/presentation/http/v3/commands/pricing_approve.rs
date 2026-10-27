//! `pricingApprove` (pricing-tiers.md §6, P-3 "Use xchange's flow"): for 1..n currencies, one new policy version with
//! their margins (through `PolicyVersionWriter`), then one rate draft with one AUTO line per currency, submitted.
//! In-bound ⇒ published by policy; out-of-bound ⇒ waits in A06 for EXECUTIVE. Idempotent through the command's key.
//!
//! SKELETON (PT-0): `run` (and any service the command needs in `register`) is PT-6's. The name, route, permission
//! (`RatePolicySave`, inherited; `RatesDraft` is checked in `run`) and the input/output types stay.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{Acquire, PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Bound, CurrencyCode, Money, Rate, Sign};
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::application::pricing::branch_push::{BranchPush, PushKind};
use xc_rate::application::pricing::tier_publish::TierSetPublisher;
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::pricing::branch_policy::{BranchPolicyPatch, BranchPolicyStore};
use xc_rate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use xc_rate::domain::pricing::market_snapshot::MarketStaleness;
use xc_rate::domain::pricing::pricing_suggestion::SuggestionConfig;
use xc_rate::domain::pricing::tier_check::{TierCheck, TierError};
use xc_rate::domain::pricing::tier_set::{
    MarketAnchor, PricingTierSet, TierRate, TierSetSource, TierSetStore,
};
use xc_rate::infrastructure::pricing::branch_policy_store::PgBranchPolicyStore;
use xc_rate::infrastructure::pricing::tier_set_store::PgTierSetStore;

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
    /// pricing-tiers.md §15.3: the latest `pricing_tier_sets.id` the page saw for this currency (`tierSet.id` on the
    /// board), or omitted when it saw none. Not the latest ⇒ 409 `staleVersion`, nothing written.
    #[serde(default)]
    #[ts(optional)]
    pub tier_set_id: Option<String>,
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
    /// pricing-tiers.md §15.8a: one result per currency.
    pub currencies: Vec<PricingApproveCurrencyResult>,
}

/// One currency's approve result (§15.8a).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_approve.ts")]
pub struct PricingApproveCurrencyResult {
    pub currency: String,
    /// `PUBLISHED` · `REFUSED`. A row that fails parsing or `TierCheck` answers the whole request 400
    /// `validationFailed` and writes nothing (§15.3 step 2); `REFUSED` is a currency that passed the checks but whose
    /// publish was refused (e.g. `approvalRequired` from another line of the copied publication).
    pub state: String,
    pub tier_set_id: Option<String>,
    /// The v3 error code when `REFUSED`.
    pub error: Option<String>,
    /// MAIN only: one per other branch (§15.4). Absent at any other branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub branches: Option<Vec<PricingApproveBranchResult>>,
}

/// One branch's push result (§15.4, §15.8a).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_approve.ts")]
pub struct PricingApproveBranchResult {
    pub branch_id: String,
    /// `PUBLISHED` · `SKIPPED_OWN` · `FAILED` · `ALREADY_DONE`.
    pub state: String,
    pub tier_set_id: Option<String>,
    pub error: Option<String>,
}

pub struct PricingApproveCommand {
    pool: PgPool,
    publisher: Arc<TierSetPublisher>,
    push: Arc<BranchPush>,
}

impl PricingApproveCommand {
    pub fn new(pool: PgPool, publisher: Arc<TierSetPublisher>, push: Arc<BranchPush>) -> Self {
        Self {
            pool,
            publisher,
            push,
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
        let now = chrono::Utc::now();
        Policy::authorize(
            Some(who),
            EntryPath::Rpc,
            &Action::new("rateDraft"),
            Some(branch),
            now,
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
        let mut parsed_rows: Vec<(CurrencyCode, [TierRate; 4], Option<Uuid>)> =
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
            let expected_tier_set_id = match row
                .tier_set_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(s) => Some(Uuid::from_str(s).map_err(|_| {
                    V3Error::new(
                        "validationFailed",
                        Some("tierSetId"),
                        "invalid tierSetId",
                        StatusCode::BAD_REQUEST,
                    )
                })?),
                None => None,
            };
            if row.tiers.len() != 4 {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("tiers"),
                    "each row must have 4 tiers (1..=4)",
                    StatusCode::BAD_REQUEST,
                ));
            }

            let mut by_tier: [Option<TierRate>; 4] = [None, None, None, None];
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
                    Bound::numeric(20, 5, Sign::Positive).at_most(Money::MAX_INPUT_UNITS);
                v3::input::decimal(TIER_BUY_FIELDS[idx], t.buy.trim(), rate_bound)?;
                let buy = Rate::parse(t.buy.trim())
                    .map_err(|_| validation_failed(TIER_BUY_FIELDS[idx]))?;
                v3::input::decimal(TIER_SELL_FIELDS[idx], t.sell.trim(), rate_bound)?;
                let sell = Rate::parse(t.sell.trim())
                    .map_err(|_| validation_failed(TIER_SELL_FIELDS[idx]))?;
                by_tier[idx] = Some(TierRate { buy, sell });
            }

            parsed_rows.push((
                currency,
                [
                    by_tier[0].unwrap(),
                    by_tier[1].unwrap(),
                    by_tier[2].unwrap(),
                    by_tier[3].unwrap(),
                ],
                expected_tier_set_id,
            ));
        }

        let cur_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM rate_policy_versions \
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

        if cur_id != expected_version_id {
            return Err(V3Error::new(
                "staleVersion",
                Some("policyVersionId"),
                "policy version has changed",
                StatusCode::CONFLICT,
            ));
        }

        let mut sets = Vec::with_capacity(parsed_rows.len());

        for (currency, tiers, expected_tier_set_id) in parsed_rows {
            let latest = PgTierSetStore
                .latest_in(tx, branch.as_str(), currency)
                .await
                .map_err(map_rate_engine_error)?;
            if latest.as_ref().map(|s| s.id) != expected_tier_set_id {
                return Err(V3Error::new(
                    "staleVersion",
                    Some("tierSetId"),
                    "tier set has changed",
                    StatusCode::CONFLICT,
                ));
            }

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
                     b.fetched_at, \
                     b.id AS batch_id \
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
            let mut batch_by_source = HashMap::with_capacity(market_rows.len());
            for r in market_rows {
                let ref_type: String = r.get("reference_type");
                let unit_size: Decimal = r.get("unit_size");
                let buy: Option<Decimal> = r.get("buy");
                let sell: Option<Decimal> = r.get("sell");
                let fetched_at: chrono::DateTime<chrono::Utc> = r.get("fetched_at");
                let batch_id: Uuid = r.get("batch_id");
                let market_source = match ref_type.as_str() {
                    "MARKET_TT" => MarketSource::Tt,
                    "MARKET_SR" => MarketSource::SuperRich,
                    _ => continue,
                };
                batch_by_source.insert(market_source, batch_id);
                domain_quotes.push(MarketQuote {
                    source: market_source,
                    buy: buy.and_then(|d| Rate::parse(&d.to_string()).ok()),
                    sell: sell.and_then(|d| Rate::parse(&d.to_string()).ok()),
                    unit_size,
                    fetched_at,
                    stale: MarketStaleness::is_stale(Some(fetched_at), now),
                });
            }

            let (anchor, anchor_flags) = MarketQuote::anchor_for(&domain_quotes);
            let check_tiers = TierRate::to_check(&tiers);
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

            let market_anchor = anchor.and_then(|q| {
                Some(MarketAnchor {
                    source: q.source,
                    buy: q.buy?,
                    sell: q.sell?,
                    fetched_at: q.fetched_at,
                    batch_id: batch_by_source.get(&q.source).copied(),
                })
            });
            let flags = anchor_flags.into_iter().map(|f| format!("{f:?}")).collect();

            sets.push(PricingTierSet {
                id: Uuid::new_v4(),
                request_id: format!("{}:{}", key.as_str(), currency.as_str()),
                branch: branch.as_str().to_owned(),
                currency,
                source: TierSetSource::Manual,
                base_tier_set_id: None,
                tiers,
                anchor: market_anchor,
                flags,
                draft_id: None,
                approved_by: who.user_id.clone(),
                approved_at: now,
            });
        }

        let is_main = branch.as_str() == "MAIN";
        let mut outcomes: Vec<(PricingTierSet, Result<(), String>)> =
            Vec::with_capacity(sets.len());

        for set in sets {
            let mut sp = tx.begin().await.map_err(|_| server_error())?;
            PgTierSetStore
                .insert_in(&mut sp, &set)
                .await
                .map_err(map_rate_engine_error)?;

            match self
                .publisher
                .publish_in(&mut sp, who, &set, &set.request_id)
                .await
            {
                Ok(_) => {
                    if !is_main {
                        PgBranchPolicyStore
                            .save_in(
                                &mut sp,
                                branch.as_str(),
                                set.currency.as_str(),
                                BranchPolicyPatch {
                                    own: Some(true),
                                    ..Default::default()
                                },
                                &who.user_id,
                                now,
                            )
                            .await
                            .map_err(map_rate_engine_error)?;
                    }
                    sp.commit().await.map_err(|_| server_error())?;
                    outcomes.push((set, Ok(())));
                }
                Err(RateEngineError::Database(_) | RateEngineError::Internal(_)) => {
                    return Err(server_error());
                }
                Err(err) => {
                    sp.rollback().await.map_err(|_| server_error())?;
                    outcomes.push((set, Err(map_rate_engine_error(err).code.to_string())));
                }
            }
        }

        if is_main {
            let next_tx = self.pool.begin().await.map_err(|_| server_error())?;
            let main_tx = std::mem::replace(tx, next_tx);
            main_tx.commit().await.map_err(|_| server_error())?;
        }

        let mut currencies = Vec::with_capacity(outcomes.len());
        for (set, outcome) in outcomes {
            match outcome {
                Ok(()) => {
                    let branches = if is_main {
                        Some(
                            self.push
                                .push(who, &set, PushKind::Manual)
                                .await
                                .into_iter()
                                .map(|b| PricingApproveBranchResult {
                                    branch_id: b.branch,
                                    state: b.state.as_str().to_string(),
                                    tier_set_id: b.tier_set_id.map(|id| id.to_string()),
                                    error: b.error,
                                })
                                .collect(),
                        )
                    } else {
                        None
                    };
                    currencies.push(PricingApproveCurrencyResult {
                        currency: set.currency.as_str().to_string(),
                        state: "PUBLISHED".to_string(),
                        tier_set_id: Some(set.id.to_string()),
                        error: None,
                        branches,
                    });
                }
                Err(code) => {
                    currencies.push(PricingApproveCurrencyResult {
                        currency: set.currency.as_str().to_string(),
                        state: "REFUSED".to_string(),
                        tier_set_id: None,
                        error: Some(code),
                        branches: is_main.then(Vec::new),
                    });
                }
            }
        }

        Ok(PricingApproveOutput { currencies })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let approval_service = make_approval_service(state.db.clone());
    let draft_service = Arc::new(RateDraftService::new(state.db.clone(), approval_service));
    let publish_service = Arc::new(PublishService::new(state.db.clone()));
    let publisher = Arc::new(TierSetPublisher::new(draft_service, publish_service));
    let push = Arc::new(BranchPush::new(
        state.db.clone(),
        publisher.clone(),
        Arc::new(PgTierSetStore),
        Arc::new(PgBranchPolicyStore),
    ));
    CommandEndpoint::register(
        cfg,
        Arc::new(PricingApproveCommand::new(
            state.db.clone(),
            publisher,
            push,
        )),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PricingTierPriceInput::export_all(&cfg)?;
    PricingApproveRowInput::export_all(&cfg)?;
    PricingApproveInput::export_all(&cfg)?;
    PricingApproveCurrencyResult::export_all(&cfg)?;
    PricingApproveBranchResult::export_all(&cfg)?;
    PricingApproveOutput::export_all(&cfg)?;
    Ok(())
}
