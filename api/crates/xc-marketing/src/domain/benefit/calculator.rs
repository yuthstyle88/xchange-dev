//! `BenefitCalculator` trait and `BenefitEffect` (design §6, §7, D-6, D-6a).

use std::fmt;
use std::str::FromStr;

use rust_decimal::Decimal;
use xc_kernel::money::{Money, Rate};

use super::rule::BenefitClass;
use crate::domain::engine::engine::QuoteLineInput;

/// Why a stored or submitted benefit calculation parameter could not be read (D-6a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamError {
    pub field: &'static str,
    pub reason: String,
}

impl fmt::Display for ParamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.reason)
    }
}

impl std::error::Error for ParamError {}

/// Reads an optional decimal-string parameter from `params[key]` (D-6a).
///
/// Absent ⇒ `Ok(None)`; a JSON string `Decimal` reads ⇒ `Ok(Some)`; anything else (including a
/// JSON number) ⇒ `Err`.
pub fn decimal_param(
    params: &serde_json::Value,
    key: &str,
    field: &'static str,
) -> Result<Option<Decimal>, ParamError> {
    let Some(val) = params.get(key) else {
        return Ok(None);
    };
    let Some(s) = val.as_str() else {
        return Err(ParamError {
            field,
            reason: format!("parameter {key:?} must be a decimal string"),
        });
    };
    Decimal::from_str(s).map(Some).map_err(|e| ParamError {
        field,
        reason: format!("parameter {key:?} is not a valid decimal: {e}"),
    })
}

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
    fn validate_params(&self, params: &serde_json::Value) -> Result<(), ParamError>;
    fn apply(
        &self,
        line: &QuoteLineInput,
        params: &serde_json::Value,
    ) -> Result<BenefitEffect, ParamError>;
}
