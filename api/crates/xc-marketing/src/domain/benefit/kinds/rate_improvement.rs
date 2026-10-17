use rust_decimal::Decimal;
use xc_kernel::money::{ExchangeArithmetic, Money, Rate, Side};

use crate::domain::benefit::calculator::{
    BenefitCalculator, BenefitEffect, ParamError, decimal_param,
};
use crate::domain::benefit::rule::BenefitClass;
use crate::domain::engine::engine::QuoteLineInput;

#[derive(Debug, Clone, Copy, Default)]
pub struct RateImprovement;

enum RateImprovementParams {
    Delta(Decimal),
    Percent(Decimal),
}

fn parse_params(params: &serde_json::Value) -> Result<RateImprovementParams, ParamError> {
    if let Some(delta) = decimal_param(params, "delta", "delta")? {
        return Ok(RateImprovementParams::Delta(delta));
    }
    if let Some(pct) = decimal_param(params, "percent", "percent")? {
        return Ok(RateImprovementParams::Percent(pct));
    }
    Err(ParamError {
        field: "calculation",
        reason: "RATE_IMPROVEMENT requires delta or percent".into(),
    })
}

impl BenefitCalculator for RateImprovement {
    fn kind(&self) -> &str {
        "RATE_IMPROVEMENT"
    }

    fn class(&self) -> BenefitClass {
        BenefitClass::Price
    }

    fn validate_params(&self, params: &serde_json::Value) -> Result<(), ParamError> {
        parse_params(params).map(|_| ())
    }

    fn apply(
        &self,
        line: &QuoteLineInput,
        params: &serde_json::Value,
    ) -> Result<BenefitEffect, ParamError> {
        let delta = match parse_params(params)? {
            RateImprovementParams::Delta(delta) => delta,
            RateImprovementParams::Percent(pct) => line.base_rate.value() * pct,
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

        Ok(BenefitEffect {
            new_final_rate,
            new_fee_thb: None,
            amount_thb,
        })
    }
}
