use std::str::FromStr;

use rust_decimal::Decimal;
use xc_kernel::money::{ExchangeArithmetic, Money, Rate, Side};

use crate::domain::benefit::calculator::{BenefitCalculator, BenefitEffect};
use crate::domain::benefit::rule::BenefitClass;
use crate::domain::engine::engine::QuoteLineInput;

#[derive(Debug, Clone, Copy, Default)]
pub struct RateImprovement;

impl BenefitCalculator for RateImprovement {
    fn kind(&self) -> &str {
        "RATE_IMPROVEMENT"
    }

    fn class(&self) -> BenefitClass {
        BenefitClass::Price
    }

    fn apply(&self, line: &QuoteLineInput, params: &serde_json::Value) -> BenefitEffect {
        let delta = if let Some(delta_val) = params.get("delta") {
            if let Some(s) = delta_val.as_str() {
                Decimal::from_str(s).unwrap_or(Decimal::ZERO)
            } else if let Some(n) = delta_val.as_i64() {
                Decimal::from(n)
            } else {
                Decimal::from_str(&delta_val.to_string()).unwrap_or(Decimal::ZERO)
            }
        } else if let Some(pct_val) = params.get("percent") {
            let pct = if let Some(s) = pct_val.as_str() {
                Decimal::from_str(s).unwrap_or(Decimal::ZERO)
            } else if let Some(n) = pct_val.as_i64() {
                Decimal::from(n)
            } else {
                Decimal::from_str(&pct_val.to_string()).unwrap_or(Decimal::ZERO)
            };
            line.base_rate.value() * pct
        } else {
            Decimal::ZERO
        };

        let new_rate_val = match line.shop_side {
            Side::Buy => line.base_rate.value() + delta,
            Side::Sell => line.base_rate.value() - delta,
        };

        let rounded = new_rate_val
            .round_dp_with_strategy(8, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
        let new_final_rate = Rate::parse(&rounded.to_string()).ok();

        let amount_thb = if let Some(final_rate) = new_final_rate {
            let thb_new = ExchangeArithmetic::thb(line.quantity, final_rate, line.unit_size);
            let thb_base = ExchangeArithmetic::thb(line.quantity, line.base_rate, line.unit_size);
            match line.shop_side {
                Side::Buy => {
                    let diff = thb_new.minor() - thb_base.minor();
                    Money::from_minor(diff.max(0)).unwrap_or(Money::ZERO)
                }
                Side::Sell => {
                    let diff = thb_base.minor() - thb_new.minor();
                    Money::from_minor(diff.max(0)).unwrap_or(Money::ZERO)
                }
            }
        } else {
            Money::ZERO
        };

        BenefitEffect {
            new_final_rate,
            new_fee_thb: None,
            amount_thb,
        }
    }
}
