//! One formula per side, and the built-in default that reproduces today's AUTO price (pricing-formula.md §2, §6).

use rust_decimal::{Decimal, RoundingStrategy};
use xc_kernel::money::{Rate, Side};

use crate::domain::formula::error::FormulaError;
use crate::domain::formula::input::FormulaInputs;
use crate::domain::formula::text::FormulaText;

/// `rate_proposal.rs:44-48` / `tier_proposal.rs:63-66` as text.
pub const DEFAULT_BUY: &str = "ref_buy * (1 + (buy_pct + tier_buy_pct) / 100)";
pub const DEFAULT_SELL: &str = "ref_sell * (1 + (sell_pct + tier_sell_pct) / 100)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceFormula {
    pub buy: FormulaText,
    pub sell: FormulaText,
}

impl PriceFormula {
    pub fn new(buy: FormulaText, sell: FormulaText) -> PriceFormula {
        PriceFormula { buy, sell }
    }

    /// `DEFAULT_BUY` / `DEFAULT_SELL`, parsed.
    pub fn default_formula() -> PriceFormula {
        PriceFormula::new(
            FormulaText::parse(DEFAULT_BUY).expect("DEFAULT_BUY parses"),
            FormulaText::parse(DEFAULT_SELL).expect("DEFAULT_SELL parses"),
        )
    }

    pub fn is_default(&self) -> bool {
        *self == PriceFormula::default_formula()
    }

    /// One side's price: evaluate, round to 8 places half away from zero (`rate_proposal.rs:49-52`), then a result
    /// ≤ 0 ⇒ `NotPositive`. Evaluation errors pass through unchanged.
    pub fn side(&self, side: Side, inputs: &FormulaInputs) -> Result<Rate, FormulaError> {
        let text = match side {
            Side::Buy => &self.buy,
            Side::Sell => &self.sell,
        };
        let rounded = text
            .expr()
            .eval(inputs)?
            .round_dp_with_strategy(8, RoundingStrategy::MidpointAwayFromZero);
        if rounded <= Decimal::ZERO {
            return Err(FormulaError::NotPositive);
        }
        Rate::parse(&rounded.to_string()).map_err(|_| FormulaError::Overflow)
    }

    /// Both sides (`side(Buy)`, then `side(Sell)`); the first error wins. No buy ≤ sell check here.
    pub fn price(&self, inputs: &FormulaInputs) -> Result<(Rate, Rate), FormulaError> {
        Ok((
            self.side(Side::Buy, inputs)?,
            self.side(Side::Sell, inputs)?,
        ))
    }
}

impl Default for PriceFormula {
    fn default() -> Self {
        PriceFormula::default_formula()
    }
}
