//! `BenefitPricing` (design `marketing-campaigns.md` §6.4.4, E-3): the one engine context of a priced line.
//!
//! `QuoteService::price_in` (every quote) and `BenefitEstimator` (the customer's estimate, §6.4) both call
//! [`BenefitPricing::evaluate_in`], so the margin guard, segment, consent, referral, budget and coupon reads exist
//! once. It reads only and writes nothing: no reservation, no coupon lock (the reserve loop stays in `price_in`).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_exchange::domain::quote::quote::QuoteChannel;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side};
use xc_marketing::domain::benefit::registry::CalculatorRegistry;
use xc_marketing::domain::engine::active_rules::{ActiveRule, ActiveRules};
use xc_marketing::domain::engine::engine::{
    BenefitEngine, EvalContext, Evaluation, QuoteLineInput,
};
use xc_marketing::infrastructure::pg_campaign_repository::PgCampaignRepository;
use xc_rate::domain::publication::published_rates::PublishedSide;

use crate::infrastructure::marketing::consent_gate::MarketingConsentGate;
use crate::infrastructure::marketing::referral_source::MarketingReferralSource;
use crate::infrastructure::marketing::tier_source::MarketingTierSource;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub(crate) struct MemoryActiveRules(pub(crate) Vec<ActiveRule>);

impl ActiveRules for MemoryActiveRules {
    fn active_for<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _organization_id: Uuid,
        _branch: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<Vec<ActiveRule>>> {
        Box::pin(async move { Ok(self.0.clone()) })
    }
}

pub(crate) struct AvailableActiveRules(pub(crate) Arc<dyn ActiveRules>);

impl ActiveRules for AvailableActiveRules {
    fn active_for<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        branch: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<Vec<ActiveRule>>> {
        Box::pin(async move {
            let rules = self.0.active_for(tx, organization_id, branch, now).await?;
            if rules.is_empty() {
                return Ok(rules);
            }

            let has_budgets: bool =
                sqlx::query_scalar("SELECT to_regclass('campaign_budgets') IS NOT NULL")
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

            if !has_budgets {
                return Ok(rules);
            }

            let exhausted_versions: Vec<Uuid> = sqlx::query_scalar(
                "SELECT v.id \
                 FROM campaign_versions v \
                 JOIN campaign_budgets b ON b.campaign_version_id = v.id \
                 WHERE v.organization_id = $1 AND b.consumed_thb >= v.budget_thb",
            )
            .bind(organization_id)
            .fetch_all(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            if exhausted_versions.is_empty() {
                Ok(rules)
            } else {
                Ok(rules
                    .into_iter()
                    .filter(|r| !exhausted_versions.contains(&r.version_id))
                    .collect())
            }
        })
    }
}

/// A coupon the caller has already validated (`price_in` keeps its own lookup and checks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CouponMatch {
    pub coupon_id: Uuid,
    pub rule_id: Uuid,
}

/// The one line being priced, at its published side.
#[derive(Debug, Clone)]
pub struct PricingLine {
    pub currency: CurrencyCode,
    pub shop_side: Side,
    pub quantity: ForeignAmount,
    /// The line's fee before benefits; the estimate passes `Money::ZERO` (E-8).
    pub fee_thb: Money,
    pub published: PublishedSide,
}

pub struct BenefitPricingInput<'a> {
    pub organization_id: Uuid,
    pub branch: &'a BranchId,
    /// Already mapped: `benefit_channel(req.channel)` for a quote, `QuoteChannel::App` for the estimate.
    pub eligibility_channel: QuoteChannel,
    /// Keep only rules whose `channels` also admit this channel (absent `channels` = any). OD-1 B: the
    /// estimate passes `Some(QuoteChannel::Desk)`; a quote passes `None`.
    pub also_admitted_on: Option<QuoteChannel>,
    pub customer_id: Option<&'a str>,
    pub coupon: Option<CouponMatch>,
    pub coupon_code: Option<&'a str>,
    pub line: PricingLine,
    pub now: DateTime<Utc>,
}

/// The three customer sources the engine read, kept so `price_in` re-evaluates with the same ones.
#[derive(Debug, Clone)]
pub struct BenefitSources {
    pub tier: MarketingTierSource,
    pub consent: MarketingConsentGate,
    pub referral: MarketingReferralSource,
}

/// What one evaluation produced.
#[derive(Debug, Clone)]
pub struct PricedBenefits {
    pub evaluation: Evaluation,
    /// The context the engine judged with (channel, customer, coupon code, margin share, segments), so `price_in`'s
    /// re-evaluation after a skipped reservation judges with the same one and reads nothing twice.
    pub context: EvalContext,
    pub line_input: QuoteLineInput,
    /// The rules after the budget, coupon and `also_admitted_on` filters: what the engine judged.
    pub active_rules: Vec<ActiveRule>,
    /// How many rules the `also_admitted_on` filter dropped (0 when it is `None`), so the estimate can tell
    /// "nothing runs" (`noActiveRule`) from "runs, but not on this channel" (`notEligible`, row 4).
    pub dropped_by_also_admitted_on: usize,
    pub sources: BenefitSources,
}

pub struct BenefitPricing {
    campaign_repo: Arc<PgCampaignRepository>,
}

impl BenefitPricing {
    pub fn new(campaign_repo: Arc<PgCampaignRepository>) -> Self {
        Self { campaign_repo }
    }

    /// Step 7 of `price_in` up to and including the first `evaluate`: published buy/sell, margin guard,
    /// segment memberships, the three sources, the exhausted-budget filter, the coupon filter, then
    /// `also_admitted_on`, then `BenefitEngine::evaluate`. Reads only; writes nothing.
    pub async fn evaluate_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        input: BenefitPricingInput<'_>,
    ) -> AppResult<PricedBenefits> {
        let rate_row: Option<(Option<Decimal>, Option<Decimal>)> =
            sqlx::query_as("SELECT buy_rate, sell_rate FROM published_rate_lines WHERE id = $1")
                .bind(input.line.published.line_id.as_uuid())
                .fetch_optional(&mut **tx)
                .await
                .map_err(AppError::Database)?;

        let (pub_buy, pub_sell) = match rate_row {
            Some((Some(b), Some(s))) => (
                Rate::parse(&b.to_string())
                    .map_err(|e| AppError::Domain(format!("invalid published buy rate: {e}")))?,
                Rate::parse(&s.to_string())
                    .map_err(|e| AppError::Domain(format!("invalid published sell rate: {e}")))?,
            ),
            _ => (input.line.published.rate, input.line.published.rate),
        };

        let has_margin_guards: bool =
            sqlx::query_scalar("SELECT to_regclass('margin_guards') IS NOT NULL")
                .fetch_one(&mut **tx)
                .await
                .map_err(AppError::Database)?;

        let max_spread_share: Option<Decimal> = if has_margin_guards {
            sqlx::query_scalar(
                "SELECT max_spread_share FROM margin_guards WHERE organization_id = $1",
            )
            .bind(input.organization_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?
            .flatten()
        } else {
            None
        };

        let segment_memberships: Vec<Uuid> = match input.customer_id {
            Some(cid) => {
                // stopgap: MK-6 removes this
                let has_segment_table: bool = sqlx::query_scalar(
                    "SELECT to_regclass('segment_snapshot_members') IS NOT NULL",
                )
                .fetch_one(&mut **tx)
                .await
                .map_err(AppError::Database)?;

                if has_segment_table {
                    sqlx::query_scalar(
                        "SELECT snapshot_id FROM segment_snapshot_members WHERE customer_id = $1",
                    )
                    .bind(cid)
                    .fetch_all(&mut **tx)
                    .await
                    .map_err(AppError::Database)?
                } else {
                    Vec::new()
                }
            }
            None => Vec::new(),
        };

        let tier_source = MarketingTierSource::new();
        let consent_gate = match input.customer_id {
            Some(cid) => MarketingConsentGate::prefetch_in(tx, cid).await?,
            None => MarketingConsentGate::new(),
        };
        let referral_source = match input.customer_id {
            Some(cid) => MarketingReferralSource::prefetch_in(tx, cid, input.now).await?,
            None => MarketingReferralSource::new(),
        };
        let sources = BenefitSources {
            tier: tier_source,
            consent: consent_gate.clone(),
            referral: referral_source.clone(),
        };

        let context = EvalContext {
            organization_id: input.organization_id,
            branch: input.branch.as_str().to_string(),
            channel: input.eligibility_channel.as_str().to_string(),
            customer_id: input.customer_id.map(str::to_string),
            coupon_code: input.coupon_code.map(str::to_string),
            now: input.now,
            max_spread_share,
            segment_memberships,
        };

        let line_input = QuoteLineInput {
            currency: input.line.currency,
            shop_side: input.line.shop_side,
            quantity: input.line.quantity,
            unit_size: input.line.published.unit_size,
            base_rate: input.line.published.rate,
            fee_thb: input.line.fee_thb,
            published_buy_rate: pub_buy,
            published_sell_rate: pub_sell,
        };

        let active_rules = AvailableActiveRules(self.campaign_repo.clone())
            .active_for(tx, input.organization_id, input.branch.as_str(), input.now)
            .await?;

        let mut active_rules: Vec<ActiveRule> = active_rules
            .into_iter()
            .filter(|r| {
                if r.rule.requires_coupon {
                    match input.coupon {
                        Some(coupon) => r.rule.id == coupon.rule_id,
                        None => false,
                    }
                } else {
                    true
                }
            })
            .collect();

        let dropped_by_also_admitted_on = match input.also_admitted_on {
            Some(ch) => {
                let before = active_rules.len();
                let ch_str = ch.as_str();
                active_rules.retain(|r| {
                    r.rule
                        .eligibility
                        .channels
                        .as_ref()
                        .is_none_or(|channels| channels.iter().any(|c| c == ch_str))
                });
                before - active_rules.len()
            }
            None => 0,
        };

        let engine = BenefitEngine::new(
            Arc::new(MemoryActiveRules(active_rules.clone())),
            Arc::new(CalculatorRegistry::new()),
            Arc::new(tier_source),
            Arc::new(consent_gate),
            Arc::new(referral_source),
        );

        let evaluation = engine
            .evaluate(tx, &context, std::slice::from_ref(&line_input))
            .await?;

        Ok(PricedBenefits {
            evaluation,
            context,
            line_input,
            active_rules,
            dropped_by_also_admitted_on,
            sources,
        })
    }
}
