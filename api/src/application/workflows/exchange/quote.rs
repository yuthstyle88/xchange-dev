//! QuoteService workflow (§4.3, §7).

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel, QuoteStatus};
use xc_exchange::domain::quote::quote_line::QuoteLine;
use xc_exchange::domain::quote::quote_type::QuoteType;
use xc_exchange::domain::quote::repository::QuoteRepository;
use xc_exchange::domain::settings::repository::SettingsRepository;
use xc_exchange::infrastructure::quote::repository::PgQuoteRepository;
use xc_exchange::infrastructure::settings::repository::PgSettingsRepository;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{
    CurrencyCode, ExchangeArithmetic, ForeignAmount, Money, Rate, Settlement, SettlementError,
    Side, UnitSize,
};
use xc_marketing::domain::benefit::registry::CalculatorRegistry;
use xc_marketing::domain::coupon::benefit_error::BenefitError;
use xc_marketing::domain::coupon::code_hasher::CouponCodeHasher;
use xc_marketing::domain::engine::active_rules::ActiveRule;
use xc_marketing::domain::engine::engine::BenefitEngine;
use xc_marketing::domain::engine::snapshot::{AppliedBenefit, SkippedRule};
use xc_marketing::infrastructure::pg_budget_ledger::{
    BudgetLedger, ReserveOutcome, ReserveRequest,
};
use xc_marketing::infrastructure::pg_campaign_repository::PgCampaignRepository;
use xc_rate::domain::currency::branch_currencies::{BranchCurrencies, Display};
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::domain::publication::published_rates::{LineSelector, PublishedRates};
use xc_rate::infrastructure::currency::branch_currencies::PgBranchCurrencies;
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;

use super::benefit_pricing::{
    BenefitPricing, BenefitPricingInput, CouponMatch, MemoryActiveRules, PricingLine,
};
use crate::application::workflows::save_step::SaveActor;

fn thb_or_refuse(q: ForeignAmount, r: Rate, u: UnitSize) -> AppResult<Money> {
    ExchangeArithmetic::try_thb(q, r, u).map_err(|_| {
        AppError::Domain("validationFailed: quantity overflows thb amount".to_string())
    })
}

fn reserved_amount(applied: &AppliedBenefit) -> AppResult<Decimal> {
    Decimal::from_str(&applied.amount_thb).map_err(|_| {
        AppError::Internal(format!(
            "applied benefit {} has an unreadable amountThb {:?}",
            applied.rule_id, applied.amount_thb
        ))
    })
}

#[derive(Debug, Clone)]
pub struct QuoteRequestLine {
    pub currency: CurrencyCode,
    pub shop_side: Side,
    pub quantity: ForeignAmount,
    pub fee_thb: Option<Money>,
    pub denomination_group: GroupCode,
}

#[derive(Debug, Clone)]
pub struct QuoteRequest {
    pub branch_id: BranchId,
    pub customer_id: Option<String>,
    pub channel: QuoteChannel,
    pub lines: Vec<QuoteRequestLine>,
    pub payment_method: Option<PaymentMethod>,
}

/// What `QuoteService::price_in` writes (`binding-quote.md` D-1, D-10): the quote's type and, for a binding
/// quote, its lifetime (the organisation's `binding_quote_ttl_seconds`) in place of the indicative TTL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteTerms {
    /// Today's quote: `INDICATIVE`, `expires_at = now + exchange_settings.quote_ttl_seconds`.
    Indicative,
    /// `BINDING`, `expires_at = now + ttl`.
    Binding { ttl: Duration },
}

impl QuoteTerms {
    pub fn quote_type(&self) -> QuoteType {
        match self {
            Self::Indicative => QuoteType::Indicative,
            Self::Binding { .. } => QuoteType::Binding,
        }
    }
}

/// `marketing-campaigns.md` O-3 A: the channel a quote's benefits are judged eligible on. A `Booking` quote is
/// issued by staff holding `BookingsDesk` (no customer reaches `QuoteService`), so it is judged as `DESK`;
/// every other channel is its own. (`CustomerSelf` ⇒ `APP` has no caller yet: the customer-side estimate needs a
/// design of its own.)
pub fn benefit_channel(channel: QuoteChannel) -> QuoteChannel {
    match channel {
        QuoteChannel::Booking => QuoteChannel::Desk,
        other => other,
    }
}

/// `marketing-campaigns.md` O-3 A: a `Booking` quote shows its benefit with no reservation (no budget, no coupon
/// lock); every other channel reserves as today. The completion's DESK quote is the one that reserves.
pub fn reserves_benefits(channel: QuoteChannel) -> bool {
    channel != QuoteChannel::Booking
}

pub struct QuoteService {
    published_rates: PgPublishedRates,
    quote_repo: PgQuoteRepository,
    settings_repo: PgSettingsRepository,
    benefit_pricing: BenefitPricing,
    budget_ledger: Arc<BudgetLedger>,
}

impl QuoteService {
    pub fn new(pool: PgPool) -> Self {
        let campaign_repo = Arc::new(PgCampaignRepository::new());
        Self {
            published_rates: PgPublishedRates::new(pool.clone()),
            quote_repo: PgQuoteRepository::new(pool.clone()),
            settings_repo: PgSettingsRepository::new(pool.clone()),
            benefit_pricing: BenefitPricing::new(campaign_repo),
            budget_ledger: Arc::new(BudgetLedger::new(pool)),
        }
    }

    pub async fn quote_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        req: QuoteRequest,
        who: &SaveActor,
        now: DateTime<Utc>,
    ) -> AppResult<Quote> {
        self.quote_with_coupon_in(tx, req, who, now, None).await
    }

    /// `binding-quote.md` D-1: the one priced path for every quote, writing `terms`' type and lifetime
    /// (BQ-2). `quote_with_coupon_in` is this with `QuoteTerms::Indicative`.
    pub async fn price_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        req: QuoteRequest,
        who: &SaveActor,
        now: DateTime<Utc>,
        coupon_code: Option<&str>,
        terms: QuoteTerms,
    ) -> AppResult<Quote> {
        // Step 1: Exactly 1 line
        if req.lines.len() != 1 {
            return Err(AppError::Domain(
                "validationFailed: exactly one line required".to_string(),
            ));
        }
        let line_req = &req.lines[0];

        // Step 2: Validate quantity
        let qty_dec = line_req.quantity.value();
        if qty_dec <= Decimal::ZERO {
            return Err(AppError::Domain(
                "validationFailed: quantity must be greater than zero".to_string(),
            ));
        }

        let minor_unit: Option<i16> =
            sqlx::query_scalar("SELECT minor_unit FROM currencies WHERE code = $1")
                .bind(line_req.currency.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(AppError::Database)?;

        let minor_unit = match minor_unit {
            Some(mu) => mu as u32,
            None => {
                return Err(AppError::Domain(
                    "rateUnavailable: currency not found".to_string(),
                ));
            }
        };

        if qty_dec.normalize().scale() > minor_unit {
            return Err(AppError::Domain(
                "validationFailed: quantity does not fit minor unit".to_string(),
            ));
        }

        // Step 3: Fetch published rate line
        let published_side = self
            .published_rates
            .line_for(
                tx,
                &req.branch_id,
                LineSelector {
                    code: line_req.currency,
                    side: line_req.shop_side,
                    group: line_req.denomination_group.clone(),
                    quantity: Some(line_req.quantity),
                },
            )
            .await?;

        let published_side = match published_side {
            Some(s) => s,
            None => {
                return Err(AppError::Domain(format!(
                    "rateUnavailable: no published line for {}",
                    line_req.currency
                )));
            }
        };

        if PgBranchCurrencies
            .display_in(
                tx,
                &req.branch_id,
                line_req.currency,
                &line_req.denomination_group,
            )
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
            == Some(Display::Ask)
            && req.channel != QuoteChannel::Desk
        {
            return Err(AppError::Domain(format!(
                "rateUnavailable: {} is ASK",
                line_req.currency
            )));
        }

        // Step 4: Arithmetic
        let thb_amount = thb_or_refuse(
            line_req.quantity,
            published_side.rate,
            published_side.unit_size,
        )?;

        let fee_thb = line_req.fee_thb.unwrap_or(Money::ZERO);
        if fee_thb > thb_amount {
            return Err(AppError::Domain(
                "validationFailed: fee exceeds thb amount".to_string(),
            ));
        }

        // Step 5: Staleness check under policy version the line was published under
        if published_side.stale {
            let stale_policy = self
                .published_rates
                .stale_policy_in(tx, &published_side)
                .await?;
            stale_policy
                .admits(&published_side, thb_amount)
                .map_err(|_| AppError::Domain("rateUnavailable: stale line refused".to_string()))?;
        }

        // Step 6: Organization & TTL
        let org_id: Option<Uuid> =
            sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                .bind(req.branch_id.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(AppError::Database)?;

        let org_id = match org_id {
            Some(id) => id,
            None => {
                return Err(AppError::Domain(
                    "validationFailed: branch not found".to_string(),
                ));
            }
        };

        if let Some(cid) = req.customer_id.as_deref() {
            let customer_org_id: Option<Uuid> =
                sqlx::query_scalar("SELECT organization_id FROM customers WHERE id = $1")
                    .bind(cid)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;
            match customer_org_id {
                None => {
                    return Err(AppError::Domain(format!(
                        "validationFailed: customerId {cid} not found"
                    )));
                }
                Some(c_org) if c_org != org_id => {
                    return Err(AppError::Domain(format!(
                        "outOfScope: customer {cid} is not this branch's organisation's"
                    )));
                }
                Some(_) => {}
            }
        }

        let expires_at = match terms {
            QuoteTerms::Indicative => {
                let ttl_seconds = self
                    .settings_repo
                    .get_ttl_in(tx, org_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::Domain(
                            "settingNotSet: quoteTtlSeconds: the Owner has not set the quote lifetime"
                                .to_string(),
                        )
                    })?;
                now + Duration::seconds(ttl_seconds as i64)
            }
            QuoteTerms::Binding { ttl } => now + ttl,
        };

        let quote_id = Uuid::new_v4();
        let line_id = Uuid::new_v4();

        // Step 7: Benefit evaluation and reservation (MK-5)
        let coupon_info: Option<(Uuid, Uuid)> = if let Some(code) = coupon_code {
            let keys =
                xc_customer::get_pii_keys().map_err(|e| AppError::Internal(e.to_string()))?;
            let hasher = CouponCodeHasher::new(&keys);
            let code_hash = hasher.hash(code);
            type CouponRow = (Uuid, Uuid, String, Option<String>, DateTime<Utc>);
            let row: Option<CouponRow> = sqlx::query_as(
                "SELECT id, rule_id, status, customer_id, expires_at \
                 FROM coupons \
                 WHERE organization_id = $1 AND code_hash = $2",
            )
            .bind(org_id)
            .bind(&code_hash[..])
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            let (c_id, c_rule_id, c_status, c_customer, c_expires) = match row {
                Some(r) => r,
                None => {
                    return Err(AppError::Domain(
                        "couponInvalid: unknown coupon code".to_string(),
                    ));
                }
            };

            if c_status != "ISSUED" || c_expires <= now {
                return Err(AppError::Domain(
                    "couponInvalid: coupon expired or not issued".to_string(),
                ));
            }

            if c_customer.is_some() && c_customer.as_deref() != req.customer_id.as_deref() {
                return Err(AppError::Domain(
                    "couponInvalid: customer mismatch".to_string(),
                ));
            }

            Some((c_id, c_rule_id))
        } else {
            None
        };

        let mut priced = self
            .benefit_pricing
            .evaluate_in(
                tx,
                BenefitPricingInput {
                    organization_id: org_id,
                    branch: &req.branch_id,
                    eligibility_channel: benefit_channel(req.channel),
                    also_admitted_on: None,
                    customer_id: req.customer_id.as_deref(),
                    coupon: coupon_info
                        .map(|(coupon_id, rule_id)| CouponMatch { coupon_id, rule_id }),
                    coupon_code,
                    line: PricingLine {
                        currency: line_req.currency,
                        shop_side: line_req.shop_side,
                        quantity: line_req.quantity,
                        fee_thb,
                        published: published_side.clone(),
                    },
                    now,
                },
            )
            .await?;

        let cash_settlement = req.payment_method.is_some_and(|m| m.is_cash());

        let mut final_rate = priced.evaluation.lines[0].final_rate;
        let mut final_fee_thb = priced.evaluation.lines[0].fee_thb;
        let mut final_thb_amount =
            thb_or_refuse(line_req.quantity, final_rate, published_side.unit_size)?;
        let mut final_settle_thb = {
            let thb_minor = final_thb_amount
                .try_minor()
                .map_err(|_| AppError::Domain("invalid settle amount".to_string()))?;
            let fee_minor = final_fee_thb
                .try_minor()
                .map_err(|_| AppError::Domain("invalid settle amount".to_string()))?;
            match line_req.shop_side {
                Side::Buy => thb_minor
                    .checked_sub(fee_minor)
                    .ok_or_else(|| AppError::Domain("invalid settle amount".to_string()))?,
                Side::Sell => thb_minor
                    .checked_add(fee_minor)
                    .ok_or_else(|| AppError::Domain("invalid settle amount".to_string()))?,
            };
            let settlement = Settlement::of(
                line_req.shop_side,
                final_thb_amount,
                final_fee_thb,
                cash_settlement,
            )
            .map_err(|e| match e {
                SettlementError::BelowSmallestCoin => AppError::Domain(
                    "validationFailed:quantity: amount is below the smallest coin".to_string(),
                ),
                SettlementError::FeeAbovePrincipal | SettlementError::Overflow => {
                    AppError::Domain("invalid settle amount".to_string())
                }
            })?;
            let settle = settlement.settle();
            settle
                .try_minor()
                .map_err(|_| AppError::Domain("invalid settle amount".to_string()))?;
            settle
        };

        let quote_line = QuoteLine {
            id: line_id,
            organization_id: org_id,
            quote_id,
            published_rate_line_id: published_side.line_id.as_uuid(),
            shop_side: line_req.shop_side,
            currency: line_req.currency,
            denomination_group: line_req.denomination_group.to_string(),
            quantity: line_req.quantity,
            unit_size: published_side.unit_size,
            base_rate: published_side.rate,
            final_rate,
            thb_amount: final_thb_amount,
            fee_thb: final_fee_thb,
            settle_thb: final_settle_thb,
            line_no: 1,
            created_at: now,
            updated_at: now,
        };

        let initial_snapshot = serde_json::to_value(&priced.evaluation.snapshot)
            .map_err(|e| AppError::Domain(e.to_string()))?;

        let mut quote = Quote {
            id: quote_id,
            organization_id: org_id,
            branch_id: req.branch_id,
            customer_id: req.customer_id.clone(),
            quote_type: terms.quote_type(),
            channel: req.channel,
            status: QuoteStatus::Active,
            publication_id: published_side.publication.id.as_uuid(),
            expires_at,
            benefit_snapshot: initial_snapshot,
            created_by: who.id.clone(),
            accepted_by_transaction_id: None,
            created_at: now,
            updated_at: now,
            lines: vec![quote_line],
            stale: published_side.stale,
            cash_settlement,
        };

        // Write quote to DB first so benefit_reservations FK (quote_id -> quotes.id) is satisfied
        self.quote_repo.insert_in(tx, &quote).await?;

        if reserves_benefits(req.channel) && !priced.evaluation.snapshot.applied.is_empty() {
            let mut reserved_applied = Vec::new();
            let mut recompute_needed = false;

            for mut applied in priced.evaluation.snapshot.applied {
                let amount_thb = reserved_amount(&applied)?;
                let reserve_coupon_id = if let Some((cid, crule_id)) = coupon_info {
                    if crule_id == applied.rule_id {
                        Some(cid)
                    } else {
                        None
                    }
                } else {
                    None
                };

                let reserve_req = ReserveRequest {
                    organization_id: org_id,
                    quote_id,
                    rule_id: applied.rule_id,
                    campaign_version_id: applied.version_id,
                    coupon_id: reserve_coupon_id,
                    customer_id: req.customer_id.clone(),
                    amount_thb,
                    expires_at,
                };

                match self.budget_ledger.reserve_in(tx, &reserve_req).await {
                    Ok(ReserveOutcome::Reserved(res_id)) => {
                        applied.reservation_id = Some(res_id);
                        applied.coupon_id = reserve_coupon_id;
                        reserved_applied.push(applied);
                    }
                    Ok(ReserveOutcome::Skipped(reason)) => {
                        priced.evaluation.snapshot.skipped.push(SkippedRule {
                            rule_id: applied.rule_id,
                            reason: reason.as_str().to_string(),
                        });
                        recompute_needed = true;
                    }
                    Err(BenefitError::Store(e)) => return Err(e),
                    Err(e) => return Err(AppError::Domain(e.to_string())),
                }
            }

            if recompute_needed {
                if reserved_applied.is_empty() {
                    final_rate = published_side.rate;
                    final_fee_thb = fee_thb;
                } else {
                    let reserved_rule_ids: HashSet<Uuid> =
                        reserved_applied.iter().map(|a| a.rule_id).collect();
                    let remaining_rules: Vec<ActiveRule> = priced
                        .active_rules
                        .into_iter()
                        .filter(|r| reserved_rule_ids.contains(&r.rule.id))
                        .collect();
                    let re_engine = BenefitEngine::new(
                        Arc::new(MemoryActiveRules(remaining_rules)),
                        Arc::new(CalculatorRegistry::new()),
                        Arc::new(priced.sources.tier),
                        Arc::new(priced.sources.consent),
                        Arc::new(priced.sources.referral),
                    );
                    let re_eval = re_engine
                        .evaluate(tx, &priced.context, &[priced.line_input])
                        .await?;
                    final_rate = re_eval.lines[0].final_rate;
                    final_fee_thb = re_eval.lines[0].fee_thb;
                }

                final_thb_amount =
                    thb_or_refuse(line_req.quantity, final_rate, published_side.unit_size)?;
                final_settle_thb = {
                    let thb_minor = final_thb_amount
                        .try_minor()
                        .map_err(|_| AppError::Domain("invalid settle amount".to_string()))?;
                    let fee_minor = final_fee_thb
                        .try_minor()
                        .map_err(|_| AppError::Domain("invalid settle amount".to_string()))?;
                    match line_req.shop_side {
                        Side::Buy => thb_minor
                            .checked_sub(fee_minor)
                            .ok_or_else(|| AppError::Domain("invalid settle amount".to_string()))?,
                        Side::Sell => thb_minor
                            .checked_add(fee_minor)
                            .ok_or_else(|| AppError::Domain("invalid settle amount".to_string()))?,
                    };
                    let settlement = Settlement::of(
                        line_req.shop_side,
                        final_thb_amount,
                        final_fee_thb,
                        cash_settlement,
                    )
                    .map_err(|e| match e {
                        SettlementError::BelowSmallestCoin => AppError::Domain(
                            "validationFailed:quantity: amount is below the smallest coin"
                                .to_string(),
                        ),
                        SettlementError::FeeAbovePrincipal | SettlementError::Overflow => {
                            AppError::Domain("invalid settle amount".to_string())
                        }
                    })?;
                    settlement.settle()
                };

                let bound_fee_minor = final_fee_thb
                    .try_minor()
                    .map_err(|_| AppError::Domain("invalid settle amount".to_string()))?;
                let bound_thb_minor = final_thb_amount
                    .try_minor()
                    .map_err(|_| AppError::Domain("invalid settle amount".to_string()))?;
                let bound_settle_minor = final_settle_thb
                    .try_minor()
                    .map_err(|_| AppError::Domain("invalid settle amount".to_string()))?;

                sqlx::query(
                    "UPDATE quote_lines SET final_rate = $1, fee_thb = $2, thb_amount = $3, settle_thb = $4 WHERE id = $5",
                )
                .bind(final_rate.value())
                .bind(Decimal::new(bound_fee_minor, 2))
                .bind(Decimal::new(bound_thb_minor, 2))
                .bind(Decimal::new(bound_settle_minor, 2))
                .bind(line_id)
                .execute(&mut **tx)
                .await
                .map_err(AppError::Database)?;

                quote.lines[0].final_rate = final_rate;
                quote.lines[0].fee_thb = final_fee_thb;
                quote.lines[0].thb_amount = final_thb_amount;
                quote.lines[0].settle_thb = final_settle_thb;
            }

            priced.evaluation.snapshot.applied = reserved_applied;
            let snapshot_json = serde_json::to_value(&priced.evaluation.snapshot)
                .map_err(|e| AppError::Domain(e.to_string()))?;

            sqlx::query("UPDATE quotes SET benefit_snapshot = $1 WHERE id = $2")
                .bind(&snapshot_json)
                .bind(quote_id)
                .execute(&mut **tx)
                .await
                .map_err(AppError::Database)?;

            quote.benefit_snapshot = snapshot_json;
        }

        Ok(quote)
    }

    pub async fn quote_with_coupon_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        req: QuoteRequest,
        who: &SaveActor,
        now: DateTime<Utc>,
        coupon_code: Option<&str>,
    ) -> AppResult<Quote> {
        self.price_in(tx, req, who, now, coupon_code, QuoteTerms::Indicative)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xc_marketing::domain::benefit::rule::BenefitClass;

    fn sample_applied(rule_id: Uuid, amount_thb: &str) -> AppliedBenefit {
        AppliedBenefit {
            rule_id,
            version_id: Uuid::new_v4(),
            benefit_type: "RATE_IMPROVEMENT".to_string(),
            benefit_class: BenefitClass::Price,
            coupon_id: None,
            reservation_id: None,
            amount_thb: amount_thb.to_string(),
            rate_before: "35.00".to_string(),
            rate_after: "35.50".to_string(),
            fee_before: "0.00".to_string(),
            fee_after: "0.00".to_string(),
            floor_capped: false,
        }
    }

    #[test]
    fn r1_unreadable_alphabetic_amount_returns_internal_error_naming_rule_id() {
        let rule_id = Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
        let applied = sample_applied(rule_id, "abc");
        let err = reserved_amount(&applied).expect_err("non-decimal amountThb must be refused");
        match err {
            AppError::Internal(msg) => {
                assert!(
                    msg.contains(&rule_id.to_string()),
                    "expected error message to name rule_id {rule_id}, got {msg:?}"
                );
                assert!(
                    msg.contains("abc"),
                    "expected error message to name unreadable text, got {msg:?}"
                );
            }
            other => panic!("expected AppError::Internal, got {other:?}"),
        }
    }

    #[test]
    fn r2_empty_amount_returns_internal_error_naming_rule_id() {
        let rule_id = Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap();
        let applied = sample_applied(rule_id, "");
        let err = reserved_amount(&applied).expect_err("empty amountThb must be refused");
        match err {
            AppError::Internal(msg) => {
                assert!(
                    msg.contains(&rule_id.to_string()),
                    "expected error message to name rule_id {rule_id}, got {msg:?}"
                );
            }
            other => panic!("expected AppError::Internal, got {other:?}"),
        }
    }

    #[test]
    fn k1_valid_decimal_amount_returns_parsed_decimal() {
        let rule_id = Uuid::new_v4();
        let applied = sample_applied(rule_id, "12.50");
        let amount = reserved_amount(&applied).expect("valid decimal amountThb");
        assert_eq!(amount, Decimal::new(1250, 2));
    }

    #[test]
    fn k2_zero_amount_returns_real_zero() {
        let rule_id = Uuid::new_v4();
        let applied = sample_applied(rule_id, "0.00");
        let amount = reserved_amount(&applied).expect("0.00 is a valid zero amountThb");
        assert_eq!(amount, Decimal::ZERO);
    }
}
