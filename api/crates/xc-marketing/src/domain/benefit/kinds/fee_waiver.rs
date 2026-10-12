use std::str::FromStr;

use rust_decimal::Decimal;
use xc_kernel::money::Money;

use crate::domain::benefit::calculator::{BenefitCalculator, BenefitEffect};
use crate::domain::benefit::rule::BenefitClass;
use crate::domain::engine::engine::QuoteLineInput;

#[derive(Debug, Clone, Copy, Default)]
pub struct FeeWaiver;

impl BenefitCalculator for FeeWaiver {
    fn kind(&self) -> &str {
        "FEE_WAIVER"
    }

    fn class(&self) -> BenefitClass {
        BenefitClass::Price
    }

    fn apply(&self, line: &QuoteLineInput, params: &serde_json::Value) -> BenefitEffect {
        let waived_minor = if let Some(pct_val) = params.get("percent") {
            let pct = if let Some(s) = pct_val.as_str() {
                Decimal::from_str(s).unwrap_or(Decimal::ZERO)
            } else if let Some(n) = pct_val.as_i64() {
                Decimal::from(n)
            } else {
                Decimal::from_str(&pct_val.to_string()).unwrap_or(Decimal::ZERO)
            };
            if pct >= Decimal::ONE {
                line.fee_thb.minor()
            } else if pct <= Decimal::ZERO {
                0
            } else {
                let waived = Decimal::from(line.fee_thb.minor()) * pct;
                waived.round().mantissa() as i64
            }
        } else if let Some(fixed_val) = params.get("amount").or_else(|| params.get("fixed")) {
            let fixed = if let Some(s) = fixed_val.as_str() {
                Decimal::from_str(s).unwrap_or(Decimal::ZERO)
            } else if let Some(n) = fixed_val.as_i64() {
                Decimal::from(n)
            } else {
                Decimal::from_str(&fixed_val.to_string()).unwrap_or(Decimal::ZERO)
            };
            (fixed * Decimal::from(100)).round().mantissa() as i64
        } else {
            0
        };

        let waived_minor = waived_minor.clamp(0, line.fee_thb.minor());
        let new_fee_minor = line.fee_thb.minor() - waived_minor;
        let new_fee_thb = Some(Money::from_minor(new_fee_minor).unwrap_or(Money::ZERO));
        let amount_thb = Money::from_minor(waived_minor).unwrap_or(Money::ZERO);

        BenefitEffect {
            new_final_rate: None,
            new_fee_thb,
            amount_thb,
        }
    }
}
