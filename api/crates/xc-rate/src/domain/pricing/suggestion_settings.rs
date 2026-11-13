//! A branch's `better` % settings for the suggestion (pricing-tiers.md §5.2; V.9 `src/pricing/store.mjs:10-11`,
//! `src/pricing.mjs:51,204-206`). Not part of the policy version: it shapes the suggestion only, never a price.
//!
//! SKELETON (XC-PRICING PX-0): `better_for`, `from` and `validate` are PX-10's. Types and signatures stay.

use std::collections::HashMap;

use rust_decimal::Decimal;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::policy::pricing_rules::{PctRange, require};
use crate::domain::pricing::pricing_suggestion::SuggestionConfig;

/// Where a currency's effective `better` comes from (the board's `betterPctFrom`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BetterFrom {
    Currency,
    Branch,
    Default,
}

impl BetterFrom {
    /// `"currency"` · `"branch"` · `"default"`.
    pub fn as_str(self) -> &'static str {
        match self {
            BetterFrom::Currency => "currency",
            BetterFrom::Branch => "branch",
            BetterFrom::Default => "default",
        }
    }
}

/// One branch's rows: the `'*'` row and one per currency.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SuggestionSettings {
    pub branch_default: Option<[Decimal; 4]>,
    pub by_currency: HashMap<String, [Decimal; 4]>,
}

impl SuggestionSettings {
    /// The currency's row (`Currency`), else the branch `'*'` row (`Branch`), else `None` ("not set",
    /// owner-set-values.md §3 row 6).
    pub fn better_for_with(&self, currency: &str) -> Option<([Decimal; 4], BetterFrom)> {
        if let Some(&better) = self.by_currency.get(currency) {
            Some((better, BetterFrom::Currency))
        } else {
            self.branch_default
                .map(|better| (better, BetterFrom::Branch))
        }
    }

    /// GA-3 deletes this.
    pub fn better_for(&self, currency: &str) -> ([Decimal; 4], BetterFrom) {
        self.better_for_with(currency)
            .unwrap_or((SuggestionConfig::v9_default().better, BetterFrom::Default))
    }

    /// The currency override (`rate_policy_currency_tiers.min_spread_pct`) when set, else the branch `'*'` row's
    /// `pricing_suggestion_settings.min_spread_pct`, else `None` ("not set", owner-set-values.md §3 row 6).
    pub fn min_spread_for_with(
        currency_override: Option<Decimal>,
        branch_default: Option<Decimal>,
    ) -> Option<Decimal> {
        currency_override.or(branch_default)
    }

    /// GA-3 deletes this.
    pub fn min_spread_for(
        currency_override: Option<Decimal>,
        branch_default: Option<Decimal>,
    ) -> Decimal {
        Self::min_spread_for_with(currency_override, branch_default)
            .unwrap_or(SuggestionConfig::v9_default().min_spread_pct)
    }

    /// Checks `values` against the Owner's `better_pct_range` (`None` ⇒ `pricingRuleNotSet`, field `"betterPctRange"`):
    /// exactly 4 values, each in `[range.min, range.max]`, each ≥ the previous, rescaled to 4 places.
    pub fn validate_with(
        values: &[Decimal],
        range: Option<PctRange>,
    ) -> Result<[Decimal; 4], RateEngineError> {
        let r = require(&range, "betterPctRange").map_err(|e| RateEngineError::Refused {
            code: "pricingRuleNotSet",
            message: e.field.to_owned(),
        })?;
        let [a, b, c, d] = <[Decimal; 4]>::try_from(values).map_err(|_| {
            RateEngineError::ValidationFailed(format!(
                "ส่วนต่างแต่ละระดับต้องอยู่ระหว่าง {}% ถึง {}%",
                r.min, r.max
            ))
        })?;
        if [a, b, c, d].into_iter().any(|v| v < r.min || v > r.max) {
            return Err(RateEngineError::ValidationFailed(format!(
                "ส่วนต่างแต่ละระดับต้องอยู่ระหว่าง {}% ถึง {}%",
                r.min, r.max
            )));
        }
        if b < a || c < b || d < c {
            return Err(RateEngineError::ValidationFailed(
                "ระดับที่สูงกว่าต้องให้ราคาดีกว่าหรือเท่ากับระดับก่อนหน้า".to_string(),
            ));
        }
        Ok([a, b, c, d].map(|v| {
            let mut rescaled = v.round_dp(4);
            rescaled.rescale(4);
            rescaled
        }))
    }

    /// Checks a branch `min_spread_pct` save against `min_spread_pct_range`: clearing (`None`) is allowed even when
    /// `range` is `None`; setting `Some(v)` requires `range` (`None` ⇒ `pricingRuleNotSet`, field `"minSpreadPctRange"`).
    pub fn validate_min_spread_with(
        value: Option<Decimal>,
        range: Option<PctRange>,
    ) -> Result<Option<Decimal>, RateEngineError> {
        let Some(d) = value else {
            return Ok(None);
        };
        let r = require(&range, "minSpreadPctRange").map_err(|e| RateEngineError::Refused {
            code: "pricingRuleNotSet",
            message: e.field.to_owned(),
        })?;
        if d < r.min || d > r.max {
            return Err(RateEngineError::ValidationFailed(format!(
                "minSpreadPct must be between {} and {}",
                r.min, r.max
            )));
        }
        Ok(Some(d))
    }

    /// GA-3 deletes this.
    pub fn validate(values: &[Decimal]) -> Result<[Decimal; 4], String> {
        Self::validate_with(
            values,
            SuggestionConfig::v9_default().rules.better_pct_range,
        )
        .map_err(|e| match e {
            RateEngineError::ValidationFailed(msg) => msg,
            other => other.to_string(),
        })
    }
}
