//! FOLLOW: automatic market pricing at MAIN (pricing-tiers.md §15.5; `H:` §6). After every `MarketRefresh::tick`
//! (5 min, only when `MARKET_FEED_ENABLED=true`), plan each FOLLOW currency under `RATE_FEED_LOCK_KEY` and publish
//! the unblocked plans. It does not depend on the BoT tick (gap 5).
//!
//! SKELETON (XC-V9-PRICING): `run`, `plan_once` and `plan_currency` are PV-f3's; the spawn in `bootstrap/mod.rs` is
//! PV-f3's too. Types and signatures stay.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate, RatePercent};
use xc_rate::application::draft::reference_base::ReferenceBase;
use xc_rate::application::formula::formula_record::FormulaRecord;
use xc_rate::application::formula::formula_resolver::{FormulaResolver, FormulaSource};
use xc_rate::application::market::market_refresh::MarketRefresh;
use xc_rate::application::pricing::branch_push::{BranchPush, PushKind};
use xc_rate::application::pricing::tier_publish::TierSetPublisher;
use xc_rate::domain::currency::branch_currencies::{BranchCurrencies, Display};
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::formula::price_formula::PriceFormula;
use xc_rate::domain::pricing::follow_plan::{
    FollowInput, FollowOutcome, FollowPlan, FollowResult, FollowResultStatus, FollowResultStore,
    SourceFreshness,
};
use xc_rate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use xc_rate::domain::pricing::pricing_mode::PricingModeStore;
use xc_rate::domain::pricing::pricing_suggestion::{
    PricingSuggestion, SuggestionConfig, SuggestionInput,
};
use xc_rate::domain::pricing::tier_set::{PricingTierSet, TierRate, TierSetSource, TierSetStore};
use xc_rate::infrastructure::currency::branch_currencies::PgBranchCurrencies;
use xc_rate::infrastructure::market::market_feed_status_store::MarketFeedStatusStore;
use xc_rate::infrastructure::policy::currency_tier_override_store::CurrencyTierOverrideStore;
use xc_rate::infrastructure::pricing::follow_result_store::PgFollowResultStore;
use xc_rate::infrastructure::pricing::pricing_mode_store::PgPricingModeStore;
use xc_rate::infrastructure::pricing::suggestion_settings_store::PricingSuggestionSettingsStore;
use xc_rate::infrastructure::pricing::tier_set_store::PgTierSetStore;

use super::feed_loop::RATE_FEED_LOCK_KEY;

pub struct MarketPricingLoop {
    pool: PgPool,
    refresh: Arc<MarketRefresh>,
    publisher: Arc<TierSetPublisher>,
    push: Arc<BranchPush>,
}

impl MarketPricingLoop {
    pub fn new(
        pool: PgPool,
        refresh: Arc<MarketRefresh>,
        publisher: Arc<TierSetPublisher>,
        push: Arc<BranchPush>,
    ) -> Self {
        Self {
            pool,
            refresh,
            publisher,
            push,
        }
    }

    /// `refresh.tick(now)` then `plan_once(now)`, every `interval`, until `stop` turns true.
    pub async fn run(self: Arc<Self>, interval: Duration, mut stop: watch::Receiver<bool>) {
        if *stop.borrow() {
            return;
        }
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() {
                        break;
                    }
                }
                _ = ticker.tick() => {
                    if *stop.borrow() {
                        break;
                    }
                    let now = Utc::now();
                    let report = self.refresh.tick(now).await;
                    for (source, result) in report.results {
                        if let Err(err) = result {
                            tracing::warn!(?source, error = %err, "market refresh source failed");
                        }
                    }
                    if let Err(error) = self.plan_once(now).await {
                        tracing::warn!(%error, "market pricing plan failed");
                    }
                }
            }
        }
    }

    /// Plans every currency whose `pricing_modes.mode` is `FOLLOW`, under `RATE_FEED_LOCK_KEY`, and returns each
    /// result in currency order. See `plan_currency`.
    pub async fn plan_once(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<FollowResult>, RateEngineError> {
        let mut lock_tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(RATE_FEED_LOCK_KEY)
            .execute(&mut *lock_tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let org_id: String =
            sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = 'MAIN'")
                .fetch_one(&mut *lock_tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;
        let currencies = PgPricingModeStore
            .following_in(&mut lock_tx, &org_id)
            .await?;
        let main_branch = BranchId::new("MAIN");
        let all_group = GroupCode::all();
        let mut results = Vec::with_capacity(currencies.len());
        for currency in currencies {
            if PgBranchCurrencies
                .display_in(&mut lock_tx, &main_branch, currency, &all_group)
                .await?
                == Some(Display::Hide)
            {
                continue;
            }
            match self.plan_currency(currency, now).await {
                Ok(res) => results.push(res),
                Err(e) => {
                    let _ = lock_tx.rollback().await;
                    return Err(e);
                }
            }
        }
        let _ = lock_tx.rollback().await;
        Ok(results)
    }

    /// One FOLLOW currency at MAIN: build `FollowInput`, `FollowPlan::plan`; blocked ⇒ record `BLOCKED` with the
    /// reasons, publish nothing; a candidate ⇒ write a `FOLLOW` tier set with
    /// `request_id = follow:MAIN:<code>:<anchor batch id>` (already there ⇒ nothing new, the result stays as it was),
    /// `TierSetPublisher::publish_in` at MAIN, record `PUBLISHED`, then `BranchPush::push(.., PushKind::Follow)`.
    /// A failed publish records `CANDIDATE`.
    ///
    /// [required] FOLLOW publishes as a SYSTEM actor that is **not** EXECUTIVE, the way the BoT feed does
    /// (`branch_refresh.rs:76-85`, `SYSTEM:rate-feed:BOT`), e.g. `SYSTEM:pricing-follow`. `RateDraftService::submit`
    /// self-approves an out-of-bound draft for an EXECUTIVE (`rate_draft_service.rs:645`), so an EXECUTIVE actor would
    /// let another line of the copied publication publish unreviewed; PV-15c pins the refusal.
    pub async fn plan_currency(
        &self,
        currency: CurrencyCode,
        now: DateTime<Utc>,
    ) -> Result<FollowResult, RateEngineError> {
        let (input, formula, formula_source) = self.build_follow_input(currency).await?;
        let outcome = FollowPlan::plan_with(&input, now, &formula);

        match outcome {
            FollowOutcome::Blocked(reasons) => {
                let mut conn = self
                    .pool
                    .acquire()
                    .await
                    .map_err(|e| RateEngineError::Database(e.to_string()))?;
                let result = FollowResult {
                    currency,
                    status: FollowResultStatus::Blocked,
                    reasons,
                    anchor_batch_id: input.anchor_batch_id,
                    tier_set_id: None,
                    planned_at: now,
                };
                PgFollowResultStore.record_in(&mut conn, &result).await?;
                Ok(result)
            }
            FollowOutcome::Candidate(tiers, anchor) => {
                let batch_part = anchor
                    .batch_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| anchor.fetched_at.to_rfc3339());
                let request_id = format!("follow:MAIN:{}:{batch_part}", currency.as_str());

                {
                    let mut conn = self
                        .pool
                        .acquire()
                        .await
                        .map_err(|e| RateEngineError::Database(e.to_string()))?;
                    if let Some(existing) =
                        PgTierSetStore.by_request_in(&mut conn, &request_id).await?
                    {
                        if let Some(prev) =
                            PgFollowResultStore.latest_in(&mut conn, currency).await?
                        {
                            return Ok(prev);
                        }
                        return Ok(FollowResult {
                            currency,
                            status: FollowResultStatus::Published,
                            reasons: Vec::new(),
                            anchor_batch_id: anchor.batch_id,
                            tier_set_id: Some(existing.id),
                            planned_at: now,
                        });
                    }
                }

                let mut tx = self
                    .pool
                    .begin()
                    .await
                    .map_err(|e| RateEngineError::Database(e.to_string()))?;

                let org_id: String = sqlx::query_scalar(
                    "SELECT organization_id::text FROM branches WHERE id = 'MAIN'",
                )
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;

                let system_actor = Actor::new(
                    "SYSTEM:pricing-follow",
                    &org_id,
                    vec![ScopedRole::new(
                        Role::Admin,
                        None,
                        DateTime::<Utc>::UNIX_EPOCH,
                        None,
                    )],
                    None,
                );

                let flags = PricingSuggestion::of_with(&input.suggestion, &formula)
                    .flags
                    .into_iter()
                    .map(|f| format!("{f:?}"))
                    .collect();

                let set = PricingTierSet {
                    id: Uuid::new_v4(),
                    request_id: request_id.clone(),
                    branch: "MAIN".to_owned(),
                    currency,
                    source: TierSetSource::Follow,
                    base_tier_set_id: None,
                    tiers,
                    anchor: Some(anchor.clone()),
                    flags,
                    draft_id: None,
                    approved_by: system_actor.user_id.clone(),
                    approved_at: now,
                };

                PgTierSetStore.insert_in(&mut tx, &set).await?;
                FormulaRecord::tier_set(&mut tx, set.id, formula_source).await?;

                match self
                    .publisher
                    .publish_in(&mut tx, &system_actor, &set, &request_id)
                    .await
                {
                    Ok(_) => {
                        let result = FollowResult {
                            currency,
                            status: FollowResultStatus::Published,
                            reasons: Vec::new(),
                            anchor_batch_id: anchor.batch_id,
                            tier_set_id: Some(set.id),
                            planned_at: now,
                        };
                        PgFollowResultStore.record_in(&mut tx, &result).await?;
                        tx.commit()
                            .await
                            .map_err(|e| RateEngineError::Database(e.to_string()))?;
                        self.push.push(&system_actor, &set, PushKind::Follow).await;
                        Ok(result)
                    }
                    Err(_) => {
                        let _ = tx.rollback().await;
                        let mut conn = self
                            .pool
                            .acquire()
                            .await
                            .map_err(|e| RateEngineError::Database(e.to_string()))?;
                        let result = FollowResult {
                            currency,
                            status: FollowResultStatus::Candidate,
                            reasons: Vec::new(),
                            anchor_batch_id: anchor.batch_id,
                            tier_set_id: None,
                            planned_at: now,
                        };
                        PgFollowResultStore.record_in(&mut conn, &result).await?;
                        Ok(result)
                    }
                }
            }
        }
    }

    async fn build_follow_input(
        &self,
        currency: CurrencyCode,
    ) -> Result<(FollowInput, PriceFormula, FormulaSource), RateEngineError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let unit_size: Decimal =
            sqlx::query_scalar("SELECT unit_size FROM currencies WHERE code = $1")
                .bind(currency.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?
                .unwrap_or(Decimal::ONE);

        let policy_row = sqlx::query(
            "SELECT id, buy_adjust_pct, sell_adjust_pct \
             FROM rate_policy_versions \
             WHERE branch_id = 'MAIN' AND valid_to IS NULL \
             ORDER BY version DESC LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let (policy_version_id, buy_adjust_pct, sell_adjust_pct, ovr_min_spread) = match policy_row
        {
            Some(r) => {
                let id: Uuid = r.get("id");
                let buy_adj: Decimal = r.get("buy_adjust_pct");
                let sell_adj: Decimal = r.get("sell_adjust_pct");
                let buy_adjust_pct = RatePercent::parse(&buy_adj.to_string()).map_err(|_| {
                    RateEngineError::Internal(format!(
                        "rate_policy_versions.buy_adjust_pct {buy_adj} is not a percent"
                    ))
                })?;
                let sell_adjust_pct = RatePercent::parse(&sell_adj.to_string()).map_err(|_| {
                    RateEngineError::Internal(format!(
                        "rate_policy_versions.sell_adjust_pct {sell_adj} is not a percent"
                    ))
                })?;
                let ovrs = CurrencyTierOverrideStore::for_version(&mut tx, &id).await?;
                let ovr_ms = ovrs
                    .into_iter()
                    .find(|o| o.currency == currency)
                    .and_then(|o| o.min_spread_pct)
                    .map(|p| p.value());
                (Some(id), buy_adjust_pct, sell_adjust_pct, ovr_ms)
            }
            None => (None, RatePercent::ZERO, RatePercent::ZERO, None),
        };

        let main_branch = BranchId::new("MAIN");
        let (formula, formula_source) =
            FormulaResolver::resolve(&mut tx, &main_branch, policy_version_id).await?;

        let suggestion_settings =
            PricingSuggestionSettingsStore::for_branch(&mut tx, "MAIN").await?;
        let branch_min_spread =
            PricingSuggestionSettingsStore::branch_min_spread(&mut tx, "MAIN").await?;
        let mut config = SuggestionConfig::v9_default();
        config.better = suggestion_settings.better_for(currency.as_str()).0;
        if let Some(ms) = ovr_min_spread.or(branch_min_spread) {
            config.min_spread_pct = ms;
        }

        let market_rows = sqlx::query(
            "SELECT DISTINCT ON (l.reference_type) \
                 l.reference_type, \
                 l.unit_size, \
                 l.buy, \
                 l.sell, \
                 l.raw->>'decimals' AS decimals_str, \
                 b.id AS batch_id, \
                 b.fetched_at \
             FROM rate_reference_lines l \
             JOIN rate_reference_batches b ON b.id = l.batch_id \
             WHERE trim(l.currency_code) = $1 \
               AND l.reference_type IN ('MARKET_TT', 'MARKET_SR') \
             ORDER BY l.reference_type, b.fetched_at DESC, b.id DESC",
        )
        .bind(currency.as_str())
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let mut quotes = Vec::with_capacity(market_rows.len());
        let mut batch_by_source = HashMap::new();
        let mut tt_decimals = None;
        for r in market_rows {
            let ref_type: String = r.get("reference_type");
            let q_unit: Decimal = r.get("unit_size");
            let buy: Option<Decimal> = r.get("buy");
            let sell: Option<Decimal> = r.get("sell");
            let decimals_str: Option<String> = r.get("decimals_str");
            let batch_id: Uuid = r.get("batch_id");
            let fetched_at: DateTime<Utc> = r.get("fetched_at");
            let market_source = match ref_type.as_str() {
                "MARKET_TT" => {
                    tt_decimals = decimals_str.and_then(|s| s.trim().parse::<u32>().ok());
                    MarketSource::Tt
                }
                "MARKET_SR" => MarketSource::SuperRich,
                _ => continue,
            };
            batch_by_source.insert(market_source, batch_id);
            quotes.push(MarketQuote {
                source: market_source,
                buy: buy.and_then(|d| Rate::parse(&d.to_string()).ok()),
                sell: sell.and_then(|d| Rate::parse(&d.to_string()).ok()),
                unit_size: q_unit,
                fetched_at,
                stale: false,
            });
        }

        let (anchor_quote, _) = MarketQuote::anchor_for(&quotes);
        let anchor_batch_id = anchor_quote.and_then(|q| batch_by_source.get(&q.source).copied());
        let feed_status = MarketFeedStatusStore::all(&mut tx).await?;
        let source = match anchor_quote {
            Some(q) => {
                let attempt = feed_status.get(&q.source);
                SourceFreshness::of_feed(
                    Some(q.fetched_at),
                    attempt.map(|a| a.at),
                    attempt.and_then(|a| a.error.clone()),
                )
            }
            None => SourceFreshness::of_feed(None, None, None),
        };

        let mut base_buy = None;
        let mut base_sell = None;
        if let Some(ref_id) = ReferenceBase::newest_in(&mut tx, &main_branch, &currency).await?
            && let Some(ref_row) =
                sqlx::query("SELECT unit_size, buy, sell FROM rate_reference_lines WHERE id = $1")
                    .bind(ref_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(|e| RateEngineError::Database(e.to_string()))?
        {
            let ref_unit: Decimal = ref_row.get("unit_size");
            let ref_buy: Option<Decimal> = ref_row.get("buy");
            let ref_sell: Option<Decimal> = ref_row.get("sell");
            if ref_unit > Decimal::ZERO && unit_size > Decimal::ZERO {
                base_buy = ref_buy
                    .map(|b| b * unit_size / ref_unit)
                    .and_then(|b| Rate::parse(&b.to_string()).ok());
                base_sell = ref_sell
                    .map(|s| s * unit_size / ref_unit)
                    .and_then(|s| Rate::parse(&s.to_string()).ok());
            }
        }

        let pub_row = sqlx::query(
            "SELECT l.buy_rate, \
                    l.sell_rate, \
                    l.source_snapshot->'partner'->>'cost' AS partner_cost \
             FROM published_rate_lines l \
             JOIN branches b ON b.current_publication_id = l.publication_id \
             WHERE b.id = 'MAIN' \
               AND trim(l.currency_code) = $1 \
               AND l.tier_lower_exclusive_thb IS NULL \
             LIMIT 1",
        )
        .bind(currency.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let (published_tier1_buy, published_tier1_sell, partner_cost) = match pub_row {
            Some(r) => {
                let buy: Option<Decimal> = r.get("buy_rate");
                let sell: Option<Decimal> = r.get("sell_rate");
                let cost_str: Option<String> = r.get("partner_cost");
                (
                    buy.and_then(|d| Rate::parse(&d.to_string()).ok()),
                    sell.and_then(|d| Rate::parse(&d.to_string()).ok()),
                    cost_str.and_then(|s| Rate::parse(s.trim()).ok().map(Rate::value)),
                )
            }
            None => (None, None, None),
        };
        let published_tier1 = published_tier1_buy
            .zip(published_tier1_sell)
            .map(|(buy, sell)| TierRate { buy, sell });

        let last_tier1 = PgTierSetStore
            .latest_in(&mut tx, "MAIN", currency)
            .await?
            .map(|ts| ts.tiers[0]);

        let _ = tx.rollback().await;

        Ok((
            FollowInput {
                suggestion: SuggestionInput {
                    currency,
                    quotes,
                    tt_decimals,
                    unit_size,
                    base_buy,
                    base_sell,
                    buy_adjust_pct,
                    sell_adjust_pct,
                    partner_cost,
                    published_tier1_buy,
                    published_tier1_sell,
                    config,
                },
                source,
                anchor_batch_id,
                last_tier1,
                published_tier1,
            },
            formula,
            formula_source,
        ))
    }
}
