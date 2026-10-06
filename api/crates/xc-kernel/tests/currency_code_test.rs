//! `CurrencyCode`: the stock register of v2 (`XCStock.currencies`, `server.mjs:6512`).
//!
//! Provided with ticket T2c. These tests are the specification: make them pass, do not edit them.

use std::collections::HashMap;

use xc_kernel::money::{CurrencyCode, UnknownCurrency};

const V2_CURRENCIES: &str = include_str!("fixtures/v2_currencies.json");

#[test]
fn the_register_is_v2s_list_in_v2s_order() {
    let expected: Vec<String> =
        serde_json::from_str(V2_CURRENCIES).expect("the fixture is a JSON array");
    assert_eq!(expected.len(), 153, "the fixture itself changed");
    let actual: Vec<&str> = CurrencyCode::all().iter().map(|c| c.as_str()).collect();
    assert_eq!(actual, expected);
}

#[test]
fn baht_comes_first_and_is_named() {
    assert_eq!(CurrencyCode::all()[0], CurrencyCode::THB);
    assert_eq!(CurrencyCode::THB.as_str(), "THB");
    assert!(CurrencyCode::THB.is_thb());
}

#[test]
fn a_registered_code_parses_to_itself() {
    for text in ["THB", "USD", "JPY", "ZWG"] {
        let code = CurrencyCode::parse(text).expect("registered");
        assert_eq!(code.as_str(), text);
        assert_eq!(code.to_string(), text);
    }
    assert_eq!(CurrencyCode::parse("THB"), Ok(CurrencyCode::THB));
    assert!(!CurrencyCode::parse("USD").unwrap().is_thb());
}

#[test]
fn every_code_of_the_register_parses() {
    for code in CurrencyCode::all() {
        assert_eq!(CurrencyCode::parse(code.as_str()), Ok(*code));
    }
}

#[test]
fn anything_else_is_refused() {
    // v2 compares with `currencies.includes(code)`: exact, case-sensitive, no trimming.
    for text in [
        "",
        "US",
        "USDX",
        "usd",
        "Usd",
        " USD",
        "USD ",
        "XXX",
        "BTC",
        "U$D",
        "ไทย",
    ] {
        assert_eq!(
            CurrencyCode::parse(text),
            Err(UnknownCurrency),
            "{text:?} must be refused"
        );
    }
}

#[test]
fn a_code_is_a_cheap_key() {
    let usd = CurrencyCode::parse("USD").unwrap();
    let copy = usd; // Copy
    let mut stock: HashMap<CurrencyCode, i64> = HashMap::new();
    stock.insert(usd, 1);
    *stock.entry(copy).or_insert(0) += 1;
    assert_eq!(stock[&usd], 2);
    assert_eq!(stock.len(), 1);
}
