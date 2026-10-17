//! One bounded parse for every numeric write input (design `docs/design/blueprint/bounded-inputs.md` §2 B-1, B-7, §3;
//! tracking #2054). A `Bound` describes the column it protects, spelled as in the migration (`numeric(p, s)`), and
//! its `parse` is the only place that checks syntax, scale, sign and magnitude.

use rust_decimal::Decimal;

/// Which signs a bound accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sign {
    /// `> 0`.
    Positive,
    /// `>= 0`.
    NonNegative,
    /// Any sign; a leading `-` is accepted.
    Signed,
}

/// Why a text was refused by a [`Bound`] or a value by an [`IntBound`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BoundError {
    #[error("not a plain decimal number")]
    Syntax,
    #[error("more than {max} decimal places")]
    TooManyDecimals { max: u8 },
    #[error("out of range")]
    OutOfRange,
    #[error("sign not allowed")]
    Sign,
}

/// The bound of a `numeric(precision, scale)` column, optionally with an inclusive maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bound {
    int_digits: u8,
    scale: u8,
    sign: Sign,
    max_inclusive: Option<i64>,
}

impl Bound {
    /// The bound of a `numeric(precision, scale)` column: |v| < 10^(precision − scale), at most `scale` decimals.
    pub const fn numeric(precision: u8, scale: u8, sign: Sign) -> Self {
        Bound {
            int_digits: precision.saturating_sub(scale),
            scale,
            sign,
            max_inclusive: None,
        }
    }

    /// An inclusive maximum, |v| <= max. It replaces the strict power-of-ten limit of `numeric`.
    pub const fn at_most(self, max: i64) -> Self {
        Bound {
            max_inclusive: Some(max),
            ..self
        }
    }

    /// Parse a plain decimal string against this bound's syntax, scale, sign, and magnitude rules.
    pub fn parse(&self, text: &str) -> Result<Decimal, BoundError> {
        if text.is_empty() {
            return Err(BoundError::Syntax);
        }

        let (negative, unsigned) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };

        if unsigned.is_empty() || unsigned.starts_with('.') || unsigned.ends_with('.') {
            return Err(BoundError::Syntax);
        }

        let mut dots = 0usize;
        for b in unsigned.bytes() {
            match b {
                b'0'..=b'9' => {}
                b'.' => {
                    dots += 1;
                    if dots > 1 {
                        return Err(BoundError::Syntax);
                    }
                }
                _ => return Err(BoundError::Syntax),
            }
        }

        let (int_part, frac_part) = unsigned.split_once('.').unwrap_or((unsigned, ""));
        let max_dp = usize::from(self.scale);
        if frac_part.len() > max_dp && frac_part[max_dp..].bytes().any(|b| b != b'0') {
            return Err(BoundError::TooManyDecimals { max: self.scale });
        }

        let sig_int = int_part.trim_start_matches('0');
        let is_zero = sig_int.is_empty() && frac_part.bytes().all(|b| b == b'0');

        match self.sign {
            Sign::Positive => {
                if negative || is_zero {
                    return Err(BoundError::Sign);
                }
            }
            Sign::NonNegative => {
                if negative {
                    return Err(BoundError::Sign);
                }
            }
            Sign::Signed => {}
        }

        if sig_int.len() > usize::from(self.int_digits) {
            return Err(BoundError::OutOfRange);
        }

        let int_for_parse = if sig_int.is_empty() { "0" } else { sig_int };
        let trunc_frac = &frac_part[..frac_part.len().min(max_dp)];
        let sign_str = if negative && !is_zero { "-" } else { "" };
        let normalized = if trunc_frac.is_empty() {
            format!("{sign_str}{int_for_parse}")
        } else {
            format!("{sign_str}{int_for_parse}.{trunc_frac}")
        };

        let val = Decimal::from_str_exact(&normalized).map_err(|_| BoundError::OutOfRange)?;

        if let Some(max) = self.max_inclusive
            && val.abs() > Decimal::from(max)
        {
            return Err(BoundError::OutOfRange);
        }

        Ok(val)
    }
}

/// An integer CHECK: `min ..= max` (e.g. `BETWEEN 5 AND 240`; `> 0` = `1 ..= i64::MAX`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntBound {
    min: i64,
    max: i64,
}

impl IntBound {
    pub const fn integer(min: i64, max: i64) -> Self {
        IntBound { min, max }
    }

    /// Check that `v` lies within `min ..= max`.
    pub fn check(&self, v: i64) -> Result<i64, BoundError> {
        if v < self.min || v > self.max {
            return Err(BoundError::OutOfRange);
        }
        Ok(v)
    }
}
