//! A formula as the owner wrote it, parsed (pricing-formula.md §2).

use crate::domain::formula::error::FormulaError;
use crate::domain::formula::expr::FormulaExpr;

/// At most 500 characters (`bounded-inputs.md`).
pub const MAX_LEN: usize = 500;
/// At most 16 nested parentheses, a function call's parentheses included.
pub const MAX_DEPTH: usize = 16;
/// A decimal literal has at most 8 places.
pub const MAX_PLACES: u32 = 8;

/// The text (trimmed) and its expression. Two texts are equal when both their text and their expression are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaText {
    text: String,
    expr: FormulaExpr,
}

impl FormulaText {
    /// Grammar (whitespace between tokens is ignored):
    ///
    /// ```text
    /// expr   := term (('+' | '-') term)*
    /// term   := unary (('*' | '/') unary)*
    /// unary  := '-' unary | atom
    /// atom   := number | input | ('min' | 'max') '(' expr ',' expr ')' | '(' expr ')'
    /// number := digit+ ('.' digit{1,8})?          -- no exponent, no leading '.'
    /// input  := one of the `FormulaInput` names
    /// ```
    ///
    /// Errors, checked in this order: more than `MAX_LEN` characters ⇒ `TooLong`; more than `MAX_DEPTH` nested
    /// parentheses ⇒ `TooDeep`; an identifier that is neither an input nor `min`/`max` ⇒ `UnknownInput(name)`;
    /// anything else that does not match (empty text included) ⇒ `Syntax`.
    pub fn parse(text: &str) -> Result<FormulaText, FormulaError> {
        let _ = text;
        todo!("pricing-formula wave 1: FormulaText::parse")
    }

    pub fn expr(&self) -> &FormulaExpr {
        &self.expr
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }
}
