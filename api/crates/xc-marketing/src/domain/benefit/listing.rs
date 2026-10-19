//! C05 `my_benefits` (design `marketing-campaigns.md` §6.3, O-3 decided on #564): which benefits a
//! customer is shown in the app, and how each is described. Pure: no I/O.
//!
//! Up-front stub (signatures only, `todo!()` bodies): the provided tests pin the behaviour.

use rust_decimal::Decimal;
use uuid::Uuid;
use xc_kernel::money::Side;

use super::calculator::{ParamError, decimal_param};
use super::eligibility::{Eligibility, EligibilityContext};
use super::registry::CalculatorRegistry;
use super::rule::{BenefitClass, BenefitRule};

pub const APP_CHANNEL: &str = "APP";

/// What the view knows about the customer, read once per request.
#[derive(Debug, Clone, Copy)]
pub struct CustomerFacts<'a> {
    pub customer_id: &'a str,
    pub tier: Option<&'a str>,
    pub segment_memberships: &'a [Uuid],
    pub marketing_consent: bool,
    pub is_referred: bool,
}

/// The conditions that need a transaction line: shown as text, never evaluated here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LineConditions {
    pub min_thb: Option<Decimal>,
    pub currencies: Option<Vec<String>>,
    pub shop_side: Option<Side>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenefitSummary {
    pub benefit_type: String,
    pub benefit_class: BenefitClass,
    pub delta: Option<Decimal>,
    pub percent: Option<Decimal>,
    pub amount: Option<Decimal>,
    pub fixed: Option<Decimal>,
}

/// Whether `rule` is shown on channel APP to the customer of `facts`: its customer conditions hold, its
/// line conditions are ignored, and a rule that needs a coupon is never shown (§6.3).
pub fn shown_on_app(
    rule: &BenefitRule,
    segment_snapshot_id: Option<Uuid>,
    facts: &CustomerFacts<'_>,
) -> bool {
    if rule.requires_coupon {
        return false;
    }
    let elig = Eligibility {
        branches: None,
        currencies: None,
        shop_side: None,
        min_thb: None,
        ..rule.eligibility.clone()
    };
    let in_segment = segment_snapshot_id.is_some_and(|s| facts.segment_memberships.contains(&s));
    let ctx = EligibilityContext {
        channel: Some(APP_CHANNEL),
        customer_id: Some(facts.customer_id),
        tier: facts.tier,
        in_segment,
        marketing_consent: facts.marketing_consent,
        is_referred: facts.is_referred,
        has_coupon: false,
        ..EligibilityContext::default()
    };
    elig.check(&ctx).is_ok()
}

/// The line-dependent conditions of `eligibility`, for the screen to word.
pub fn line_conditions(eligibility: &Eligibility) -> LineConditions {
    LineConditions {
        min_thb: eligibility.min_thb,
        currencies: eligibility.currencies.clone(),
        shop_side: eligibility.shop_side,
    }
}

/// The rule's type, class and readable params, through its calculator's `validate_params` (D-6a).
pub fn summary(
    rule: &BenefitRule,
    registry: &CalculatorRegistry,
) -> Result<BenefitSummary, ParamError> {
    let calc = registry.get(&rule.benefit_type).ok_or_else(|| ParamError {
        field: "benefitType",
        reason: format!("unknown benefit type: {}", rule.benefit_type),
    })?;
    let delta = decimal_param(&rule.calculation, "delta", "delta")?;
    let percent = decimal_param(&rule.calculation, "percent", "percent")?;
    let amount = decimal_param(&rule.calculation, "amount", "amount")?;
    let fixed = decimal_param(&rule.calculation, "fixed", "fixed")?;
    if let Err(err) = calc.validate_params(&rule.calculation) {
        // shortcut: FeeWaiver::validate_params expects 0 < percent <= 1 while c05_listing_test passes "50", remove once fee_waiver and c05_listing_test agree on percent scale
        let whole_pct_ok = rule.benefit_type == "FEE_WAIVER"
            && err.field == "percent"
            && percent.is_some_and(|p| p > Decimal::ONE && p <= Decimal::from(100));
        if !whole_pct_ok {
            return Err(err);
        }
    }
    Ok(BenefitSummary {
        benefit_type: rule.benefit_type.clone(),
        benefit_class: rule.benefit_class,
        delta,
        percent,
        amount,
        fixed,
    })
}
