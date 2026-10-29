//! A formula as the owner wrote it, parsed (pricing-formula.md §2).

use std::str::FromStr;

use rust_decimal::Decimal;

use crate::domain::formula::error::FormulaError;
use crate::domain::formula::expr::{FormulaExpr, Op};
use crate::domain::formula::input::FormulaInput;

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
        let trimmed = text.trim();
        if trimmed.chars().count() > MAX_LEN {
            return Err(FormulaError::TooLong);
        }
        let mut p = Parser {
            chars: trimmed.chars().collect(),
            pos: 0,
            depth: 0,
        };
        let expr = p.parse_expr()?;
        p.skip_ws();
        if p.pos < p.chars.len() {
            return Err(p.syntax("unexpected trailing text"));
        }
        Ok(FormulaText {
            text: trimmed.to_owned(),
            expr,
        })
    }

    pub fn expr(&self) -> &FormulaExpr {
        &self.expr
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn skip_ws(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn syntax(&self, msg: &str) -> FormulaError {
        FormulaError::Syntax {
            pos: self.pos,
            msg: msg.to_owned(),
        }
    }

    fn open_paren(&mut self) -> Result<(), FormulaError> {
        self.skip_ws();
        if self.peek() != Some('(') {
            return Err(self.syntax("expected `(`"));
        }
        self.pos += 1;
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(FormulaError::TooDeep);
        }
        Ok(())
    }

    fn close_paren(&mut self) -> Result<(), FormulaError> {
        self.skip_ws();
        if self.peek() != Some(')') {
            return Err(self.syntax("expected `)`"));
        }
        self.pos += 1;
        self.depth -= 1;
        Ok(())
    }

    fn parse_expr(&mut self) -> Result<FormulaExpr, FormulaError> {
        let mut lhs = self.parse_term()?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some('+') => Op::Add,
                Some('-') => Op::Sub,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_term()?;
            lhs = FormulaExpr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_term(&mut self) -> Result<FormulaExpr, FormulaError> {
        let mut lhs = self.parse_unary()?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some('*') => Op::Mul,
                Some('/') => Op::Div,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_unary()?;
            lhs = FormulaExpr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<FormulaExpr, FormulaError> {
        self.skip_ws();
        if self.peek() == Some('-') {
            self.pos += 1;
            return Ok(FormulaExpr::Neg(Box::new(self.parse_unary()?)));
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<FormulaExpr, FormulaError> {
        self.skip_ws();
        match self.peek() {
            Some('(') => {
                self.open_paren()?;
                let expr = self.parse_expr()?;
                self.close_paren()?;
                Ok(expr)
            }
            Some(c) if c.is_ascii_digit() => self.parse_number(),
            Some(c) if c.is_ascii_alphabetic() || c == '_' => self.parse_ident(),
            _ => Err(self.syntax("expected expression")),
        }
    }

    fn parse_number(&mut self) -> Result<FormulaExpr, FormulaError> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            let frac_start = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
            let frac_len = self.pos - frac_start;
            if frac_len == 0 || frac_len > MAX_PLACES as usize {
                return Err(self.syntax("expected 1 to 8 decimal places"));
            }
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        Decimal::from_str(&s)
            .map(FormulaExpr::Num)
            .map_err(|_| self.syntax("invalid decimal"))
    }

    fn parse_ident(&mut self) -> Result<FormulaExpr, FormulaError> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            self.pos += 1;
        }
        let name: String = self.chars[start..self.pos].iter().collect();
        if name == "min" || name == "max" {
            self.open_paren()?;
            let a = self.parse_expr()?;
            self.skip_ws();
            if self.peek() != Some(',') {
                return Err(self.syntax("expected `,`"));
            }
            self.pos += 1;
            let b = self.parse_expr()?;
            self.close_paren()?;
            return Ok(if name == "min" {
                FormulaExpr::Min(Box::new(a), Box::new(b))
            } else {
                FormulaExpr::Max(Box::new(a), Box::new(b))
            });
        }
        FormulaInput::from_name(&name)
            .map(FormulaExpr::Input)
            .ok_or(FormulaError::UnknownInput(name))
    }
}
