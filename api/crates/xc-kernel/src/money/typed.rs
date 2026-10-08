//! Shared exact-decimal arithmetic for compute actions (§12.20.3, CA7).

use rust_decimal::Decimal;

use super::plain_decimal;

/// What a browser `type=number` input sends, without the exponent form:
/// an optional `-`, then digits with an optional `.digits`, or `.digits`.
/// No space, no `+`, no exponent, no grouping.
/// `None` when it does not fit the 96-bit decimal.
pub fn typed_decimal(text: &str) -> Option<Decimal> {
    if text.is_empty() {
        return None;
    }
    let (is_negative, rest) = match text.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, text),
    };
    if rest.is_empty() {
        return None;
    }
    let normalized;
    let candidate = if rest.starts_with('.') {
        normalized = format!("0{rest}");
        normalized.as_str()
    } else {
        rest
    };
    let mut d = plain_decimal(candidate)?;
    if is_negative {
        if d.is_zero() {
            d = Decimal::ZERO;
        } else {
            d.set_sign_negative(true);
        }
    }
    Some(d)
}

const MAX_96: i128 = 79_228_162_514_264_337_593_543_950_335;
const MIN_96: i128 = -79_228_162_514_264_337_593_543_950_335;

/// Helper to round an exact decimal to `k` decimal places, half toward +∞.
/// Computes `floor(x * 10^k + 0.5)` using exact integer arithmetic.
fn round_to_scale(x: Decimal, k: u32) -> i128 {
    let m = x.mantissa();
    let scale = x.scale();
    if scale <= k {
        let diff = k - scale;
        let factor = 10_i128.pow(diff);
        m * factor
    } else {
        let d = scale - k;
        let ten_d = 10_i128.pow(d);
        let b = 2 * ten_d;
        let a = 2 * m + ten_d;
        if a >= 0 || a % b == 0 {
            a / b
        } else {
            a / b - 1
        }
    }
}

/// v2's `Math.round(x × 100)` on the exact decimal: `floor(x × 100 + ½)`,
/// a half goes toward +∞ (`0.005 → 1`, `−0.005 → 0`, `−0.015 → −1`).
/// `None` on overflow (cents must fit in a 96-bit decimal).
pub fn round_cents(x: Decimal) -> Option<i128> {
    let cents = round_to_scale(x, 2);
    if !(MIN_96..=MAX_96).contains(&cents) {
        None
    } else {
        Some(cents)
    }
}

/// `"-"` only when below zero, integer digits without grouping, `.`, two digits:
/// `"1234.50"`, `"-0.10"`, `"0.00"` (never `"-0.00"`).
pub fn fixed2(cents: i128) -> String {
    if cents == 0 {
        return "0.00".to_string();
    }
    let is_neg = cents < 0;
    let mag = cents.unsigned_abs();
    let whole = mag / 100;
    let frac = mag % 100;
    if is_neg {
        format!("-{whole}.{frac:02}")
    } else {
        format!("{whole}.{frac:02}")
    }
}

/// `x` rounded to six places by the same half-toward-+∞ rule, six digits after the point (`toFixed(6)`).
/// Never minus zero.
pub fn fixed6(x: Decimal) -> String {
    let units = round_to_scale(x, 6);
    if units == 0 {
        return "0.000000".to_string();
    }
    let is_neg = units < 0;
    let mag = units.unsigned_abs();
    let whole = mag / 1_000_000;
    let frac = mag % 1_000_000;
    if is_neg {
        format!("-{whole}.{frac:06}")
    } else {
        format!("{whole}.{frac:06}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed6_negative_zero() {
        for text in [
            "-0",
            "-0.0",
            "-0.0000004",
            "-0.0000004999",
            "-0.0000001",
            "-.00000001",
        ] {
            let x = typed_decimal(text).unwrap();
            assert_eq!(fixed6(x), "0.000000", "fixed6({text})");
        }
    }

    #[test]
    fn typed_decimal_empty_spaces_and_minus() {
        assert_eq!(typed_decimal(""), None);
        assert_eq!(typed_decimal(" "), None);
        assert_eq!(typed_decimal("-"), None);
    }
}
