//! `BenefitCalculator` trait and `BenefitEffect` (design §6, §7, D-6).

use xc_kernel::money::{Money, Rate};

use super::rule::BenefitClass;
use crate::domain::engine::engine::QuoteLineInput;

/// The calculated effect of applying a benefit to a quote line.
#[derive(Debug, Clone, PartialEq)]
pub struct BenefitEffect {
    pub new_final_rate: Option<Rate>,
    pub new_fee_thb: Option<Money>,
    pub amount_thb: Money,
}

pub trait BenefitCalculator: Send + Sync {
    fn kind(&self) -> &str;
    fn class(&self) -> BenefitClass;
    fn apply(&self, line: &QuoteLineInput, params: &serde_json::Value) -> BenefitEffect;
}
