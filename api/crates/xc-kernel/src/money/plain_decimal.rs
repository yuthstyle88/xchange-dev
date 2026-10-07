//! The one rule for "a plain decimal" (ticket #147).

use rust_decimal::Decimal;

fn digits(part: &str) -> bool {
    !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())
}

/// `Some` only for ASCII digits with at most one dot that has at least one digit on each side;
/// no trimming, no sign, no underscore, no exponent.
pub fn plain_decimal(text: &str) -> Option<Decimal> {
    let valid = match text.bytes().filter(|&b| b == b'.').count() {
        0 => digits(text),
        1 => match text.split_once('.') {
            Some((whole, fraction)) => digits(whole) && digits(fraction),
            None => false,
        },
        _ => false,
    };

    if !valid {
        return None;
    }

    Decimal::from_str_exact(text).ok()
}
