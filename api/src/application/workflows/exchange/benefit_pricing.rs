//! `BenefitPricing` (design `marketing-campaigns.md` §6.4.4, E-3): the one engine context of a priced line.
//!
//! `QuoteService::price_in` (every quote) and `BenefitEstimator` (the customer's estimate, §6.4) both call
//! [`BenefitPricing::evaluate_in`], so the margin guard, segment, consent, referral, budget and coupon reads exist
//! once. It reads only and writes nothing: no reservation, no coupon lock (the reserve loop stays in `price_in`).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_exchange::domain::quote::quote::QuoteChannel;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Side};
use xc_marketing::domain::engine::active_rules::ActiveRule;
use xc_marketing::domain::engine::engine::{Evaluation, QuoteLineInput};
use xc_marketing::infrastructure::pg_campaign_repository::PgCampaignRepository;
use xc_rate::domain::publication::published_rates::PublishedSide;

use crate::infrastructure::marketing::consent_gate::MarketingConsentGate;
use crate::infrastructure::marketing::referral_source::MarketingReferralSource;
use crate::infrastructure::marketing::tier_source::MarketingTierSource;

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
        let _ = (&self.campaign_repo, tx, input);
        todo!("BE-1: BenefitPricing::evaluate_in")
    }
}
