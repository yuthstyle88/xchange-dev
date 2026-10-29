//! The save check (pricing-formula.md §4, §6a): before a save that can change a price is written, price every
//! affected (branch, currency, tier) from today's reference through the formula that would apply, and refuse
//! buy > sell, a price ≤ 0, an evaluation error, or a price outside the currency guard.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::Value;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate, RatePercent, Side};

use crate::application::draft::reference_base::ReferenceBase;
use crate::application::formula::formula_resolver::FormulaResolver;
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::formula::error::FormulaError;
use crate::domain::formula::input::FormulaInputs;
use crate::domain::formula::price_formula::PriceFormula;
use crate::domain::policy::currency_tier_override::CurrencyTierOverride;
use crate::domain::policy::policy_tiers::PolicyTiers;
use crate::domain::policy::rate_policy::RatePolicy;
use crate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use crate::domain::pricing::pricing_mode::PricingModeStore;
use crate::infrastructure::policy::currency_tier_override_store::CurrencyTierOverrideStore;
use crate::infrastructure::policy::policy_tiers_store::PolicyTiersStore;
use crate::infrastructure::pricing::pricing_mode_store::PgPricingModeStore;
use crate::infrastructure::pricing::suggestion_settings_store::PricingSuggestionSettingsStore;

/// What would be written.
#[derive(Debug, Clone)]
pub enum SpreadCandidate {
    /// A new organisation formula: checked at every branch of the organisation whose current policy version carries
    /// no formula of its own (B7, B8).
    Organisation {
        organization_id: Uuid,
        formula: PriceFormula,
    },
    /// A branch's next policy version, as `PolicyVersionWriter` would write it.
    Branch {
        branch: BranchId,
        version: BranchCandidate,
    },
}

#[derive(Debug, Clone)]
pub struct BranchCandidate {
    /// `Some` = the version's own formula; `None` = it follows the organisation (resolved as §3).
    pub formula: Option<PriceFormula>,
    /// The percentages and guards (B18).
    pub policy: RatePolicy,
    /// The value tiers when the switch is on, else `None` (single-price line only).
    pub tiers: Option<PolicyTiers>,
    /// Every per-currency override of the version after the change.
    pub overrides: Vec<CurrencyTierOverride>,
}

/// AUTO = the draft path (§2 bindings); FOLLOW = the §3a binding on the market anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpreadMode {
    Auto,
    Follow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpreadReason {
    BuyAboveSell,
    Formula(FormulaError),
    /// The price's distance from the reference, `(price ÷ ref − 1) × 100`, is outside the currency guard (B18).
    OutsideGuard {
        side: Side,
        pct: Decimal,
        min: Decimal,
        max: Decimal,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpreadFailure {
    pub branch: BranchId,
    pub currency: CurrencyCode,
    /// `None` = the single-price line; `Some(1..=4)` = that tier.
    pub tier: Option<u8>,
    pub mode: SpreadMode,
    pub buy: Option<Decimal>,
    pub sell: Option<Decimal>,
    pub reason: SpreadReason,
}

impl SpreadFailure {
    /// One line of the save refusal's message (§4 step 3), e.g. `MAIN USD buy 33.8837374 > sell 33.7293303`,
    /// `MAIN USD tier 3 FOLLOW buy 1.1 > sell 1.0`, `B2 JPY tier 1 divideByZero`,
    /// `MAIN USD buy 34.1 outside guard [-1, 1] (+1.2 %)`. Prices are printed normalised (no trailing zeros).
    pub fn describe(&self) -> String {
        let mut head = format!("{} {}", self.branch.as_str(), self.currency.as_str());
        if let Some(n) = self.tier {
            head.push_str(&format!(" tier {n}"));
        }
        if self.mode == SpreadMode::Follow {
            head.push_str(" FOLLOW");
        }
        let show = |v: Option<Decimal>| v.map_or("-".to_owned(), |d| d.normalize().to_string());
        match &self.reason {
            SpreadReason::BuyAboveSell => {
                format!("{head} buy {} > sell {}", show(self.buy), show(self.sell))
            }
            SpreadReason::Formula(e) => format!("{head} {}", e.code()),
            SpreadReason::OutsideGuard {
                side,
                pct,
                min,
                max,
            } => {
                let (name, price) = match side {
                    Side::Buy => ("buy", self.buy),
                    Side::Sell => ("sell", self.sell),
                };
                format!(
                    "{head} {name} {} outside guard [{}, {}] ({:+} %)",
                    show(price),
                    min.normalize(),
                    max.normalize(),
                    pct.normalize()
                )
            }
        }
    }
}

/// One priced row, failing or not: the preview's table (§4, B17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpreadRow {
    pub branch: BranchId,
    pub currency: CurrencyCode,
    pub tier: Option<u8>,
    pub mode: SpreadMode,
    pub buy: Option<Decimal>,
    pub sell: Option<Decimal>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpreadReport {
    /// Every row priced, in branch, currency, tier order.
    pub priced: Vec<SpreadRow>,
    pub failures: Vec<SpreadFailure>,
    /// Currencies with no current reference line from their anchor: not checked, never silently passed (§4 step 4).
    pub not_checked: Vec<(BranchId, CurrencyCode)>,
}

impl SpreadReport {
    pub fn is_ok(&self) -> bool {
        self.failures.is_empty()
    }
}

pub struct SpreadCheck;

impl SpreadCheck {
    /// §4 steps 1–4, in the caller's transaction and before it writes. First takes
    /// `SELECT id FROM organizations WHERE id = $1 FOR UPDATE` on the organisation concerned (§6a "Two saves at
    /// once", B19), then prices every affected branch and currency. Failures are listed in branch, currency, tier
    /// order; an `Err` is only a database failure.
    pub async fn run(
        tx: &mut Transaction<'_, Postgres>,
        candidate: &SpreadCandidate,
    ) -> Result<SpreadReport, RateEngineError> {
        let org_id = match candidate {
            SpreadCandidate::Organisation {
                organization_id, ..
            } => *organization_id,
            SpreadCandidate::Branch { branch, .. } => {
                let Some(id): Option<Uuid> =
                    sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                        .bind(branch.as_str())
                        .fetch_optional(&mut **tx)
                        .await
                        .map_err(|e| RateEngineError::Database(e.to_string()))?
                else {
                    return Ok(SpreadReport::default());
                };
                id
            }
        };

        sqlx::query("SELECT id FROM organizations WHERE id = $1 FOR UPDATE")
            .bind(org_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let branches: Vec<(
            BranchId,
            PriceFormula,
            RatePolicy,
            Option<PolicyTiers>,
            Vec<CurrencyTierOverride>,
        )> = match candidate {
            SpreadCandidate::Organisation {
                organization_id,
                formula,
            } => {
                let rows: Vec<(Uuid, String, Decimal, Decimal, Value)> = sqlx::query_as(
                    "SELECT v.id, v.branch_id, v.buy_adjust_pct, v.sell_adjust_pct, v.guards \
                     FROM rate_policy_versions v \
                     JOIN branches b ON b.id = v.branch_id \
                     WHERE b.organization_id = $1 \
                       AND v.valid_to IS NULL \
                       AND v.buy_formula IS NULL \
                       AND v.sell_formula IS NULL \
                     ORDER BY v.branch_id ASC",
                )
                .bind(organization_id)
                .fetch_all(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;

                let mut list = Vec::with_capacity(rows.len());
                for (ver_id, branch_id, buy_dec, sell_dec, guards) in rows {
                    let buy_pct = RatePercent::parse(&format!("{buy_dec:.4}")).map_err(|e| {
                        RateEngineError::Internal(format!("invalid buy_adjust_pct: {e}"))
                    })?;
                    let sell_pct = RatePercent::parse(&format!("{sell_dec:.4}")).map_err(|e| {
                        RateEngineError::Internal(format!("invalid sell_adjust_pct: {e}"))
                    })?;
                    let policy = RatePolicy::new(ver_id.to_string(), buy_pct, sell_pct, &guards)?;
                    let tiers = PolicyTiersStore::for_version(tx, &ver_id).await?;
                    let overrides = CurrencyTierOverrideStore::for_version(tx, &ver_id).await?;
                    list.push((
                        BranchId::new(branch_id),
                        formula.clone(),
                        policy,
                        tiers,
                        overrides,
                    ));
                }
                list
            }
            SpreadCandidate::Branch { branch, version } => {
                let formula = match &version.formula {
                    Some(f) => f.clone(),
                    None => FormulaResolver::organisation_current(tx, org_id)
                        .await?
                        .map(|o| o.formula)
                        .unwrap_or_default(),
                };
                vec![(
                    branch.clone(),
                    formula,
                    version.policy.clone(),
                    version.tiers.clone(),
                    version.overrides.clone(),
                )]
            }
        };

        if branches.is_empty() {
            return Ok(SpreadReport::default());
        }

        let currency_codes: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT trim(currency_code) AS code FROM rate_reference_lines ORDER BY code ASC",
        )
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let mut currencies = Vec::with_capacity(currency_codes.len());
        for code_str in &currency_codes {
            let code = CurrencyCode::parse(code_str).map_err(|e| {
                RateEngineError::Internal(format!("invalid currency_code {code_str}: {e}"))
            })?;
            currencies.push(code);
        }

        let unit_rows: Vec<(String, Decimal)> =
            sqlx::query_as("SELECT trim(code), unit_size FROM currencies WHERE code = ANY($1)")
                .bind(&currency_codes)
                .fetch_all(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;
        let unit_sizes: HashMap<String, Decimal> = unit_rows.into_iter().collect();

        let following = PgPricingModeStore.following_in(tx).await?;
        let main_settings = PricingSuggestionSettingsStore::for_branch(tx, "MAIN").await?;

        let mut report = SpreadReport::default();

        for (branch, formula, policy, tiers, overrides) in &branches {
            for &currency in &currencies {
                let unit_size = unit_sizes
                    .get(currency.as_str())
                    .copied()
                    .unwrap_or(Decimal::ONE);

                if branch.as_str() == "MAIN" && following.contains(&currency) {
                    let market_rows = sqlx::query(
                        "SELECT DISTINCT ON (l.reference_type) \
                             l.reference_type, l.unit_size, l.buy, l.sell, b.fetched_at \
                         FROM rate_reference_lines l \
                         JOIN rate_reference_batches b ON b.id = l.batch_id \
                         WHERE trim(l.currency_code) = $1 \
                           AND l.reference_type IN ('MARKET_TT', 'MARKET_SR') \
                         ORDER BY l.reference_type, b.fetched_at DESC, b.id DESC",
                    )
                    .bind(currency.as_str())
                    .fetch_all(&mut **tx)
                    .await
                    .map_err(|e| RateEngineError::Database(e.to_string()))?;

                    let mut quotes = Vec::with_capacity(market_rows.len());
                    for r in market_rows {
                        let ref_type: String = r.get("reference_type");
                        let source = match ref_type.as_str() {
                            "MARKET_TT" => MarketSource::Tt,
                            "MARKET_SR" => MarketSource::SuperRich,
                            _ => continue,
                        };
                        let q_unit: Decimal = r.get("unit_size");
                        let buy: Option<Decimal> = r.get("buy");
                        let sell: Option<Decimal> = r.get("sell");
                        let fetched_at: DateTime<Utc> = r.get("fetched_at");
                        quotes.push(MarketQuote {
                            source,
                            buy: buy.and_then(|d| Rate::parse(&d.to_string()).ok()),
                            sell: sell.and_then(|d| Rate::parse(&d.to_string()).ok()),
                            unit_size: q_unit,
                            fetched_at,
                            stale: false,
                        });
                    }

                    let (Some(anchor), _) = MarketQuote::anchor_for(&quotes) else {
                        report.not_checked.push((branch.clone(), currency));
                        continue;
                    };

                    let ref_buy = anchor.buy.map(|b| b.value() * unit_size / anchor.unit_size);
                    let ref_sell = anchor
                        .sell
                        .map(|s| s.value() * unit_size / anchor.unit_size);
                    let (better, _) = main_settings.better_for(currency.as_str());
                    for (idx, &b) in better.iter().enumerate() {
                        let inputs = FormulaInputs {
                            ref_buy,
                            ref_sell,
                            buy_pct: Decimal::ZERO,
                            sell_pct: Decimal::ZERO,
                            tier_buy_pct: b,
                            tier_sell_pct: -b,
                        };
                        check_row(
                            &mut report,
                            branch,
                            currency,
                            Some((idx + 1) as u8),
                            SpreadMode::Follow,
                            formula,
                            &inputs,
                            policy,
                        );
                    }
                    continue;
                }

                let Some(ref_id) = ReferenceBase::newest_in(tx, branch, &currency).await? else {
                    report.not_checked.push((branch.clone(), currency));
                    continue;
                };

                let (line_unit, raw_buy, raw_sell): (Decimal, Option<Decimal>, Option<Decimal>) =
                    sqlx::query_as(
                        "SELECT unit_size, buy, sell FROM rate_reference_lines WHERE id = $1",
                    )
                    .bind(ref_id)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(|e| RateEngineError::Database(e.to_string()))?;

                let ref_buy = raw_buy.map(|b| b * unit_size / line_unit);
                let ref_sell = raw_sell.map(|s| s * unit_size / line_unit);
                let base_inputs = FormulaInputs {
                    ref_buy,
                    ref_sell,
                    buy_pct: policy.buy_adjust_pct().value(),
                    sell_pct: policy.sell_adjust_pct().value(),
                    tier_buy_pct: Decimal::ZERO,
                    tier_sell_pct: Decimal::ZERO,
                };
                check_row(
                    &mut report,
                    branch,
                    currency,
                    None,
                    SpreadMode::Auto,
                    formula,
                    &base_inputs,
                    policy,
                );

                if let Some(branch_tiers) = tiers {
                    let ov = overrides.iter().find(|o| o.currency == currency);
                    let eff_tiers = branch_tiers.for_currency(ov);
                    for tier_no in 1..=4 {
                        let tier_inputs = FormulaInputs {
                            tier_buy_pct: eff_tiers.margin(tier_no, Side::Buy).value(),
                            tier_sell_pct: eff_tiers.margin(tier_no, Side::Sell).value(),
                            ..base_inputs
                        };
                        check_row(
                            &mut report,
                            branch,
                            currency,
                            Some(tier_no),
                            SpreadMode::Auto,
                            formula,
                            &tier_inputs,
                            policy,
                        );
                    }
                }
            }
        }

        Ok(report)
    }
}

#[allow(clippy::too_many_arguments)]
fn check_row(
    report: &mut SpreadReport,
    branch: &BranchId,
    currency: CurrencyCode,
    tier: Option<u8>,
    mode: SpreadMode,
    formula: &PriceFormula,
    inputs: &FormulaInputs,
    policy: &RatePolicy,
) {
    match formula.price(inputs) {
        Err(e) => {
            report.priced.push(SpreadRow {
                branch: branch.clone(),
                currency,
                tier,
                mode,
                buy: None,
                sell: None,
            });
            report.failures.push(SpreadFailure {
                branch: branch.clone(),
                currency,
                tier,
                mode,
                buy: None,
                sell: None,
                reason: SpreadReason::Formula(e),
            });
        }
        Ok((buy_rate, sell_rate)) => {
            let buy = buy_rate.value();
            let sell = sell_rate.value();
            report.priced.push(SpreadRow {
                branch: branch.clone(),
                currency,
                tier,
                mode,
                buy: Some(buy),
                sell: Some(sell),
            });
            if buy > sell {
                report.failures.push(SpreadFailure {
                    branch: branch.clone(),
                    currency,
                    tier,
                    mode,
                    buy: Some(buy),
                    sell: Some(sell),
                    reason: SpreadReason::BuyAboveSell,
                });
            } else if mode == SpreadMode::Auto {
                let hundred = Decimal::from(100);
                for (side, price, ref_val) in [
                    (Side::Buy, buy, inputs.ref_buy),
                    (Side::Sell, sell, inputs.ref_sell),
                ] {
                    if let (Some(guard), Some(r)) =
                        (policy.guard_for(currency.as_str(), side), ref_val)
                        && r > Decimal::ZERO
                    {
                        let pct = ((price / r - Decimal::ONE) * hundred)
                            .round_dp_with_strategy(4, RoundingStrategy::MidpointAwayFromZero);
                        if pct < guard.min_pct || pct > guard.max_pct {
                            report.failures.push(SpreadFailure {
                                branch: branch.clone(),
                                currency,
                                tier,
                                mode,
                                buy: Some(buy),
                                sell: Some(sell),
                                reason: SpreadReason::OutsideGuard {
                                    side,
                                    pct,
                                    min: guard.min_pct,
                                    max: guard.max_pct,
                                },
                            });
                            break;
                        }
                    }
                }
            }
        }
    }
}
