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
        match self {
            FormulaExpr::Num(n) => Ok(*n),
            FormulaExpr::Input(i) => inputs.get(*i).ok_or(FormulaError::MissingInput(*i)),
            FormulaExpr::Neg(e) => Ok(-e.eval(inputs)?),
            FormulaExpr::Bin(op, lhs, rhs) => {
                let l = lhs.eval(inputs)?;
                let r = rhs.eval(inputs)?;
                match op {
                    Op::Add => l.checked_add(r).ok_or(FormulaError::Overflow),
                    Op::Sub => l.checked_sub(r).ok_or(FormulaError::Overflow),
                    Op::Mul => l.checked_mul(r).ok_or(FormulaError::Overflow),
                    Op::Div => {
                        if r.is_zero() {
                            Err(FormulaError::DivideByZero)
                        } else {
                            l.checked_div(r).ok_or(FormulaError::Overflow)
                        }
                    }
                }
            }
            FormulaExpr::Min(a, b) => Ok(a.eval(inputs)?.min(b.eval(inputs)?)),
            FormulaExpr::Max(a, b) => Ok(a.eval(inputs)?.max(b.eval(inputs)?)),
        }
    }
}
