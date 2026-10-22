//! A branch's `better` % settings for the suggestion (pricing-tiers.md §5.2; V.9 `src/pricing/store.mjs:10-11`,
//! `src/pricing.mjs:51,204-206`). Not part of the policy version: it shapes the suggestion only, never a price.
//!
//! SKELETON (XC-PRICING PX-0): `better_for`, `from` and `validate` are PX-10's. Types and signatures stay.

use std::collections::HashMap;

use rust_decimal::Decimal;

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
    /// The currency's row, else the branch `'*'` row, else `SuggestionConfig::v9_default().better`, and which.
    pub fn better_for(&self, currency: &str) -> ([Decimal; 4], BetterFrom) {
        if let Some(&better) = self.by_currency.get(currency) {
            (better, BetterFrom::Currency)
        } else if let Some(better) = self.branch_default {
            (better, BetterFrom::Branch)
        } else {
            (SuggestionConfig::v9_default().better, BetterFrom::Default)
        }
    }

    /// V.9 `configure`'s checks (`src/pricing.mjs:204-206`): exactly 4 values, each in [−2, 2], each ≥ the previous.
    /// `Err` carries the reason for `validationFailed`. Returns the 4 values rescaled to 4 places.
    pub fn validate(values: &[Decimal]) -> Result<[Decimal; 4], String> {
        let [a, b, c, d] = <[Decimal; 4]>::try_from(values)
            .map_err(|_| "ส่วนต่างแต่ละระดับต้องอยู่ระหว่าง -2% ถึง 2%".to_string())?;
        let min = Decimal::from(-2);
        let max = Decimal::from(2);
        if [a, b, c, d].into_iter().any(|v| v < min || v > max) {
            return Err("ส่วนต่างแต่ละระดับต้องอยู่ระหว่าง -2% ถึง 2%".to_string());
        }
        if b < a || c < b || d < c {
            return Err("ระดับที่สูงกว่าต้องให้ราคาดีกว่าหรือเท่ากับระดับก่อนหน้า".to_string());
        }
        Ok([a, b, c, d].map(|v| {
            let mut r = v.round_dp(4);
            r.rescale(4);
            r
        }))
    }
}
