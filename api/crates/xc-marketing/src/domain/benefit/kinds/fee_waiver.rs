use rust_decimal::Decimal;
use xc_kernel::money::Money;

use crate::domain::benefit::calculator::{
    BenefitCalculator, BenefitEffect, ParamError, decimal_param,
};
use crate::domain::benefit::rule::BenefitClass;
use crate::domain::engine::engine::QuoteLineInput;

#[derive(Debug, Clone, Copy, Default)]
pub struct FeeWaiver;

enum FeeWaiverParams {
    Percent(Decimal),
    Fixed(Decimal),
}

fn parse_params(params: &serde_json::Value) -> Result<FeeWaiverParams, ParamError> {
    if let Some(pct) = decimal_param(params, "percent", "percent")? {
        if pct <= Decimal::ZERO || pct > Decimal::ONE {
            return Err(ParamError {
                field: "percent",
                reason: "FEE_WAIVER percent must be greater than 0 and at most 1".into(),
            });
        }
        return Ok(FeeWaiverParams::Percent(pct));
    }
    if let Some(amount) = decimal_param(params, "amount", "amount")? {
        if amount <= Decimal::ZERO {
            return Err(ParamError {
                field: "amount",
                reason: "FEE_WAIVER amount must be greater than 0".into(),
            });
        }
        return Ok(FeeWaiverParams::Fixed(amount));
    }
    if let Some(fixed) = decimal_param(params, "fixed", "fixed")? {
        if fixed <= Decimal::ZERO {
            return Err(ParamError {
                field: "fixed",
                reason: "FEE_WAIVER amount must be greater than 0".into(),
            });
        }
        return Ok(FeeWaiverParams::Fixed(fixed));
    }
    Err(ParamError {
        field: "calculation",
        reason: "FEE_WAIVER requires percent, amount, or fixed".into(),
    })
}

impl BenefitCalculator for FeeWaiver {
    fn kind(&self) -> &str {
        "FEE_WAIVER"
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
        let waived_minor = match parse_params(params)? {
            FeeWaiverParams::Percent(pct) => {
                if pct >= Decimal::ONE {
                    line.fee_thb.minor()
                } else if pct <= Decimal::ZERO {
                    0
                } else {
                    let waived = Decimal::from(line.fee_thb.minor()) * pct;
                    waived.round().mantissa() as i64
                }
            }
            FeeWaiverParams::Fixed(fixed) => (fixed * Decimal::from(100)).round().mantissa() as i64,
        };

        let waived_minor = waived_minor.clamp(0, line.fee_thb.minor());
        let new_fee_minor = line.fee_thb.minor() - waived_minor;
        let new_fee_thb = Some(Money::from_minor(new_fee_minor).unwrap_or(Money::ZERO));
        let amount_thb = Money::from_minor(waived_minor).unwrap_or(Money::ZERO);

        Ok(BenefitEffect {
            new_final_rate: None,
            new_fee_thb,
            amount_thb,
        })
    }
}
