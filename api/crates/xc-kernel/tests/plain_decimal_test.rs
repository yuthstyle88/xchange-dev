//! Ticket #147 — the one rule for "a plain decimal". Written before the fix. Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! `xc_kernel::money::plain_decimal(text)` answers `Some(decimal)` only for ASCII digits with at
//! most one dot that has at least one digit on each side. No trimming, no sign, no underscore,
//! no exponent. Every place that takes a text from a request or from the bank as a number asks
//! it (see the ticket); the general decimal parser of `rust_decimal` also takes `1_0`, `+1`,
//! `.5`, `5.` and more.
//!
//! No database; these run anywhere.

use std::str::FromStr;

use rust_decimal::Decimal;
use xc_kernel::money::plain_decimal;

#[test]
fn a_plain_decimal_is_read_as_written() {
    for text in [
        "36.4",
        "0",
        "007",
        "10.50",
        "0.5",
        "5",
        "32.1",
        "100000000.00",
    ] {
        let read = plain_decimal(text).unwrap_or_else(|| panic!("{text:?} is a plain decimal"));
        assert_eq!(
            read,
            Decimal::from_str(text).expect("a decimal the library reads"),
            "{text:?} has its own value"
        );
        assert_eq!(
            read.to_string(),
            Decimal::from_str(text).unwrap().to_string(),
            "{text:?} keeps the places it was written with"
        );
    }
}

#[test]
fn a_text_that_is_not_a_plain_decimal_is_nothing() {
    let not_plain = [
        "", ".", ".5", "5.", "1_0", "_1", "1_", "+1", "-1", "-0", "1e2", "1E2", "1e-2", "0.5e1",
        " 1", "1 ", "1\n", "1,000", "0x10", "1.2.3", "1..2", "..", "٠١٢", "١٢", "１２", "1.٢",
        "NaN", "inf",
    ];
    for text in not_plain {
        assert_eq!(plain_decimal(text), None, "{text:?} is not a plain decimal");
    }
}

#[test]
fn a_text_with_more_digits_than_a_decimal_holds_is_nothing_and_never_a_panic() {
    let long = "9".repeat(40);
    assert_eq!(plain_decimal(&long), None);
    let long_fraction = format!("0.{}", "1".repeat(40));
    assert_eq!(plain_decimal(&long_fraction), None);
}

/// The rule written a second, different way: split on the dot, both sides non-empty and all
/// ASCII digits (the one-sided form has no dot at all).
fn by_the_rule(text: &str) -> bool {
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    match text.matches('.').count() {
        0 => digits(text),
        1 => {
            let (whole, fraction) = text.split_once('.').expect("one dot");
            digits(whole) && digits(fraction)
        }
        _ => false,
    }
}

/// Every text of up to four characters over an alphabet that holds every character the general
/// parser is known to be generous with: it is a plain decimal exactly when the rule says so, and
/// when it is, the value is the library's value of the same text.
#[test]
fn every_short_text_is_judged_by_the_rule_and_nothing_else() {
    const ALPHABET: [char; 10] = ['0', '7', '.', '_', '+', '-', 'e', ' ', ',', 'x'];
    let mut texts = vec![String::new()];
    let mut from = 0;
    for _ in 0..4 {
        let to = texts.len();
        for i in from..to {
            for c in ALPHABET {
                let mut longer = texts[i].clone();
                longer.push(c);
                texts.push(longer);
            }
        }
        from = to;
    }
    assert!(
        texts.len() > 10_000,
        "the sweep covers {} texts",
        texts.len()
    );
    for text in &texts {
        let read = plain_decimal(text);
        assert_eq!(
            read.is_some(),
            by_the_rule(text),
            "{text:?}: the rule says {}",
            by_the_rule(text)
        );
        if let Some(value) = read {
            assert_eq!(
                Some(value),
                Decimal::from_str(text).ok(),
                "{text:?}: a plain decimal has the library's value"
            );
        }
    }
}
