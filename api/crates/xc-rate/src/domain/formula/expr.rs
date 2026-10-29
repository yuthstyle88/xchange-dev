//! A parsed formula and its evaluation (pricing-formula.md §2).

use rust_decimal::Decimal;

use crate::domain::formula::error::FormulaError;
use crate::domain::formula::input::{FormulaInput, FormulaInputs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormulaExpr {
    Num(Decimal),
    Input(FormulaInput),
    Neg(Box<FormulaExpr>),
    Bin(Op, Box<FormulaExpr>, Box<FormulaExpr>),
    Min(Box<FormulaExpr>, Box<FormulaExpr>),
    Max(Box<FormulaExpr>, Box<FormulaExpr>),
}

impl FormulaExpr {
    /// The exact value in `rust_decimal`, with checked arithmetic: a divisor of zero ⇒ `DivideByZero`, an overflow ⇒
    /// `Overflow`, an input with no value ⇒ `MissingInput`. No rounding and no sign check here: `PriceFormula` rounds
    /// the result and refuses one ≤ 0.
    pub fn eval(&self, inputs: &FormulaInputs) -> Result<Decimal, FormulaError> {
        let _ = inputs;
        todo!("pricing-formula wave 1: FormulaExpr::eval")
    }
}
