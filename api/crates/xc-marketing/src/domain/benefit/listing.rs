//! C05 `my_benefits` (design `marketing-campaigns.md` §6.3, O-3 decided on #564): which benefits a
//! customer is shown in the app, and how each is described. Pure: no I/O.
//!
//! Up-front stub (signatures only, `todo!()` bodies): the provided tests pin the behaviour.

use rust_decimal::Decimal;
use uuid::Uuid;
use xc_kernel::money::Side;

use super::calculator::ParamError;
use super::eligibility::Eligibility;
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
    let _ = (rule, segment_snapshot_id, facts);
    todo!("C05: shown_on_app")
}

/// The line-dependent conditions of `eligibility`, for the screen to word.
pub fn line_conditions(eligibility: &Eligibility) -> LineConditions {
    let _ = eligibility;
    todo!("C05: line_conditions")
}

/// The rule's type, class and readable params, through its calculator's `validate_params` (D-6a).
pub fn summary(
    rule: &BenefitRule,
    registry: &CalculatorRegistry,
) -> Result<BenefitSummary, ParamError> {
    let _ = (rule, registry);
    todo!("C05: summary")
}
