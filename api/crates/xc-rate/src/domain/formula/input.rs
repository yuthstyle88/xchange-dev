//! The inputs a formula may name (pricing-formula.md §2) and their values for one evaluation.

use rust_decimal::Decimal;

/// One named input. The owner writes it by its English name (`ref_buy`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormulaInput {
    RefBuy,
    RefSell,
    RefMid,
    BuyPct,
    SellPct,
    TierBuyPct,
    TierSellPct,
}

impl FormulaInput {
    pub const ALL: [FormulaInput; 7] = [
        FormulaInput::RefBuy,
        FormulaInput::RefSell,
        FormulaInput::RefMid,
        FormulaInput::BuyPct,
        FormulaInput::SellPct,
        FormulaInput::TierBuyPct,
        FormulaInput::TierSellPct,
    ];

    /// The name the owner writes.
    pub fn name(self) -> &'static str {
        match self {
            FormulaInput::RefBuy => "ref_buy",
            FormulaInput::RefSell => "ref_sell",
            FormulaInput::RefMid => "ref_mid",
            FormulaInput::BuyPct => "buy_pct",
            FormulaInput::SellPct => "sell_pct",
            FormulaInput::TierBuyPct => "tier_buy_pct",
            FormulaInput::TierSellPct => "tier_sell_pct",
        }
    }

    pub fn from_name(name: &str) -> Option<FormulaInput> {
        Self::ALL.into_iter().find(|i| i.name() == name)
    }
}

/// The values of one evaluation. `ref_buy` / `ref_sell` are at the currency's unit size (§6a "Constants are per the
/// line's unit"); `None` when the reference has no such side. The percentages are plain numbers (`0.3` = 0.3 %).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FormulaInputs {
    pub ref_buy: Option<Decimal>,
    pub ref_sell: Option<Decimal>,
    pub buy_pct: Decimal,
    pub sell_pct: Decimal,
    pub tier_buy_pct: Decimal,
    pub tier_sell_pct: Decimal,
}

impl FormulaInputs {
    /// The value of `input`; `ref_mid` = `(ref_buy + ref_sell) / 2` needs both sides. `None` = the input has no value
    /// here (`FormulaError::MissingInput` when a formula names it).
    pub fn get(&self, input: FormulaInput) -> Option<Decimal> {
        match input {
            FormulaInput::RefBuy => self.ref_buy,
            FormulaInput::RefSell => self.ref_sell,
            FormulaInput::RefMid => match (self.ref_buy, self.ref_sell) {
                (Some(b), Some(s)) => b.checked_add(s).map(|sum| sum / Decimal::TWO),
                _ => None,
            },
            FormulaInput::BuyPct => Some(self.buy_pct),
            FormulaInput::SellPct => Some(self.sell_pct),
            FormulaInput::TierBuyPct => Some(self.tier_buy_pct),
            FormulaInput::TierSellPct => Some(self.tier_sell_pct),
        }
    }
}
