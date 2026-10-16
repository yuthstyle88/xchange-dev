//! QuoteService workflow (§4.3, §7).

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel, QuoteStatus};
use xc_exchange::domain::quote::quote_line::QuoteLine;
use xc_exchange::domain::quote::repository::QuoteRepository;
use xc_exchange::domain::settings::repository::SettingsRepository;
use xc_exchange::infrastructure::quote::repository::PgQuoteRepository;
use xc_exchange::infrastructure::settings::repository::PgSettingsRepository;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ExchangeArithmetic, ForeignAmount, Money, Rate, Side};
use xc_marketing::domain::benefit::registry::CalculatorRegistry;
use xc_marketing::domain::coupon::benefit_error::BenefitError;
use xc_marketing::domain::coupon::code_hasher::CouponCodeHasher;
use xc_marketing::domain::engine::active_rules::{ActiveRule, ActiveRules};
use xc_marketing::domain::engine::engine::{BenefitEngine, EvalContext, QuoteLineInput};
use xc_marketing::domain::engine::snapshot::SkippedRule;
use xc_marketing::infrastructure::pg_budget_ledger::{
    BudgetLedger, ReserveOutcome, ReserveRequest,
};
use xc_marketing::infrastructure::pg_campaign_repository::PgCampaignRepository;
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::domain::publication::published_rates::{LineSelector, PublishedRates};
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;

use crate::application::workflows::save_step::SaveActor;
use crate::infrastructure::marketing::consent_gate::MarketingConsentGate;
use crate::infrastructure::marketing::referral_source::MarketingReferralSource;
use crate::infrastructure::marketing::tier_source::MarketingTierSource;

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
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

struct MemoryActiveRules(Vec<ActiveRule>);

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

struct AvailableActiveRules(Arc<dyn ActiveRules>);

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

pub struct QuoteService {
    published_rates: PgPublishedRates,
    quote_repo: PgQuoteRepository,
    settings_repo: PgSettingsRepository,
    campaign_repo: Arc<PgCampaignRepository>,
    budget_ledger: Arc<BudgetLedger>,
}

impl QuoteService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            published_rates: PgPublishedRates::new(pool.clone()),
            quote_repo: PgQuoteRepository::new(pool.clone()),
            settings_repo: PgSettingsRepository::new(pool.clone()),
            campaign_repo: Arc::new(PgCampaignRepository::new()),
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

    pub async fn quote_with_coupon_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        req: QuoteRequest,
        who: &SaveActor,
        now: DateTime<Utc>,
        coupon_code: Option<&str>,
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
                return Err(AppError::Domain(
                    "rateUnavailable: no published line".to_string(),
                ));
            }
        };

        // Step 4: Arithmetic
        let thb_amount = ExchangeArithmetic::thb(
            line_req.quantity,
            published_side.rate,
            published_side.unit_size,
        );

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

        let ttl_seconds = self.settings_repo.get_ttl_in(tx, org_id).await?;
        let expires_at = now + Duration::seconds(ttl_seconds as i64);

        let quote_id = Uuid::new_v4();
        let line_id = Uuid::new_v4();

        // Step 7: Benefit evaluation and reservation (MK-5)
        let rate_row: Option<(Option<Decimal>, Option<Decimal>)> =
            sqlx::query_as("SELECT buy_rate, sell_rate FROM published_rate_lines WHERE id = $1")
                .bind(published_side.line_id.as_uuid())
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
            _ => (published_side.rate, published_side.rate),
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
            .bind(org_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?
        } else {
            None
        };

        let segment_memberships: Vec<Uuid> = match req.customer_id.as_deref() {
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
        let consent_gate = match req.customer_id.as_deref() {
            Some(cid) => MarketingConsentGate::prefetch_in(tx, cid).await?,
            None => MarketingConsentGate::new(),
        };
        let referral_source = match req.customer_id.as_deref() {
            Some(cid) => MarketingReferralSource::prefetch_in(tx, cid, now).await?,
            None => MarketingReferralSource::new(),
        };

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

        let eval_ctx = EvalContext {
            organization_id: org_id,
            branch: req.branch_id.as_str().to_string(),
            channel: req.channel.as_str().to_string(),
            customer_id: req.customer_id.clone(),
            coupon_code: coupon_code.map(str::to_string),
            now,
            max_spread_share,
            segment_memberships,
        };

        let line_input = QuoteLineInput {
            currency: line_req.currency,
            shop_side: line_req.shop_side,
            quantity: line_req.quantity,
            unit_size: published_side.unit_size,
            base_rate: published_side.rate,
            fee_thb,
            published_buy_rate: pub_buy,
            published_sell_rate: pub_sell,
        };

        let active_rules = AvailableActiveRules(self.campaign_repo.clone())
            .active_for(tx, org_id, req.branch_id.as_str(), now)
            .await?;

        let active_rules: Vec<ActiveRule> = active_rules
            .into_iter()
            .filter(|r| {
                if r.rule.requires_coupon {
                    match coupon_info {
                        Some((_, coupon_rule_id)) => r.rule.id == coupon_rule_id,
                        None => false,
                    }
                } else {
                    true
                }
            })
            .collect();

        let engine = BenefitEngine::new(
            Arc::new(MemoryActiveRules(active_rules.clone())),
            Arc::new(CalculatorRegistry::new()),
            Arc::new(tier_source),
            Arc::new(consent_gate.clone()),
            Arc::new(referral_source.clone()),
        );

        let mut eval = engine
            .evaluate(tx, &eval_ctx, std::slice::from_ref(&line_input))
            .await?;

        let mut final_rate = eval.lines[0].final_rate;
        let mut final_fee_thb = eval.lines[0].fee_thb;
        let mut final_thb_amount =
            ExchangeArithmetic::thb(line_req.quantity, final_rate, published_side.unit_size);
        let mut final_settle_thb = match line_req.shop_side {
            Side::Buy => {
                let settle_minor = final_thb_amount.minor() - final_fee_thb.minor();
                Money::from_minor(settle_minor)
                    .map_err(|_| AppError::Domain("invalid settle amount".to_string()))?
            }
            Side::Sell => {
                let settle_minor = final_thb_amount.minor() + final_fee_thb.minor();
                Money::from_minor(settle_minor)
                    .map_err(|_| AppError::Domain("invalid settle amount".to_string()))?
            }
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

        let initial_snapshot =
            serde_json::to_value(&eval.snapshot).map_err(|e| AppError::Domain(e.to_string()))?;

        let mut quote = Quote {
            id: quote_id,
            organization_id: org_id,
            branch_id: req.branch_id,
            customer_id: req.customer_id.clone(),
            quote_type: "INDICATIVE".to_string(),
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
        };

        // Write quote to DB first so benefit_reservations FK (quote_id -> quotes.id) is satisfied
        self.quote_repo.insert_in(tx, &quote).await?;

        if !eval.snapshot.applied.is_empty() {
            let mut reserved_applied = Vec::new();
            let mut recompute_needed = false;

            for mut applied in eval.snapshot.applied {
                let amount_thb = Decimal::from_str(&applied.amount_thb).unwrap_or(Decimal::ZERO);
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
                        eval.snapshot.skipped.push(SkippedRule {
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
                    let remaining_rules: Vec<ActiveRule> = active_rules
                        .into_iter()
                        .filter(|r| reserved_rule_ids.contains(&r.rule.id))
                        .collect();
                    let re_engine = BenefitEngine::new(
                        Arc::new(MemoryActiveRules(remaining_rules)),
                        Arc::new(CalculatorRegistry::new()),
                        Arc::new(tier_source),
                        Arc::new(consent_gate),
                        Arc::new(referral_source),
                    );
                    let re_eval = re_engine.evaluate(tx, &eval_ctx, &[line_input]).await?;
                    final_rate = re_eval.lines[0].final_rate;
                    final_fee_thb = re_eval.lines[0].fee_thb;
                }

                final_thb_amount = ExchangeArithmetic::thb(
                    line_req.quantity,
                    final_rate,
                    published_side.unit_size,
                );
                final_settle_thb = match line_req.shop_side {
                    Side::Buy => {
                        let settle_minor = final_thb_amount.minor() - final_fee_thb.minor();
                        Money::from_minor(settle_minor)
                            .map_err(|_| AppError::Domain("invalid settle amount".to_string()))?
                    }
                    Side::Sell => {
                        let settle_minor = final_thb_amount.minor() + final_fee_thb.minor();
                        Money::from_minor(settle_minor)
                            .map_err(|_| AppError::Domain("invalid settle amount".to_string()))?
                    }
                };

                sqlx::query(
                    "UPDATE quote_lines SET final_rate = $1, fee_thb = $2, thb_amount = $3, settle_thb = $4 WHERE id = $5",
                )
                .bind(final_rate.value())
                .bind(Decimal::new(final_fee_thb.minor(), 2))
                .bind(Decimal::new(final_thb_amount.minor(), 2))
                .bind(Decimal::new(final_settle_thb.minor(), 2))
                .bind(line_id)
                .execute(&mut **tx)
                .await
                .map_err(AppError::Database)?;

                quote.lines[0].final_rate = final_rate;
                quote.lines[0].fee_thb = final_fee_thb;
                quote.lines[0].thb_amount = final_thb_amount;
                quote.lines[0].settle_thb = final_settle_thb;
            }

            eval.snapshot.applied = reserved_applied;
            let snapshot_json = serde_json::to_value(&eval.snapshot)
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
}
