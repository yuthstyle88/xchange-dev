//! Why a formula did not parse or did not give a price (pricing-formula.md §2, §6).

use thiserror::Error;

use crate::domain::formula::input::FormulaInput;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FormulaError {
    /// `pos` is the 0-based character offset where parsing stopped.
    #[error("syntax error at {pos}: {msg}")]
    Syntax { pos: usize, msg: String },
    #[error("unknown input `{0}`")]
    UnknownInput(String),
    /// The formula names an input with no value in this evaluation (e.g. `ref_sell` on a buy-only reference line).
    #[error("input `{}` has no value here", .0.name())]
    MissingInput(FormulaInput),
    #[error("division by zero")]
    DivideByZero,
    /// A result outside `rust_decimal`'s range.
    #[error("the result is too large")]
    Overflow,
    /// The rounded price is zero or below.
    #[error("the price is zero or below")]
    NotPositive,
    #[error("a formula is at most 500 characters")]
    TooLong,
    #[error("a formula nests at most 16 levels")]
    TooDeep,
}

impl FormulaError {
    /// The code the save and preview answers carry (`syntax`, `unknownInput`, …).
    pub fn code(&self) -> &'static str {
        match self {
            FormulaError::Syntax { .. } => "syntax",
            FormulaError::UnknownInput(_) => "unknownInput",
            FormulaError::MissingInput(_) => "missingInput",
            FormulaError::DivideByZero => "divideByZero",
            FormulaError::Overflow => "overflow",
            FormulaError::NotPositive => "notPositive",
            FormulaError::TooLong => "tooLong",
            FormulaError::TooDeep => "tooDeep",
        }
    }
}
