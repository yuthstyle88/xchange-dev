//! Shared by the stock tests: the fixture and the few conversions it needs.
//! Provided with ticket T3 — do not edit.
#![allow(dead_code)]

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::Deserialize;
use xc_kernel::money::{CurrencyCode, ForeignAmount, QuantityDelta};
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::domain::shift::value_objects::stock_delta::StockDelta;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeltaCase {
    pub r#type: String,
    pub currency: String,
    pub amount: String,
    pub rate: String,
    pub fee: String,
    pub method: String,
    pub v2: BTreeMap<String, String>,
    pub exact: BTreeMap<String, String>,
    pub v2_differs_from_exact: bool,
}

#[derive(Deserialize)]
pub struct DeltaRefusal {
    pub currency: String,
    pub error: String,
}

#[derive(Deserialize)]
pub struct Tx {
    pub r#type: String,
    pub currency: String,
    pub amount: String,
    pub rate: String,
    pub fee: String,
    pub method: String,
}

#[derive(Deserialize)]
pub struct Move {
    pub currency: String,
    pub direction: String,
    pub amount: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scenario {
    pub opening: BTreeMap<String, String>,
    pub transactions: Vec<Tx>,
    pub movements: Vec<Move>,
    pub exact: BTreeMap<String, String>,
    pub first_negative: Option<String>,
    pub v2_differs_from_exact: bool,
}

#[derive(Deserialize)]
pub struct Guard {
    pub balance: BTreeMap<String, String>,
    pub change: BTreeMap<String, String>,
    pub insufficient: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Difference {
    pub expected: BTreeMap<String, String>,
    pub counted: BTreeMap<String, String>,
    pub exact: BTreeMap<String, String>,
    pub v2_differs_from_exact: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fixture {
    pub deltas: Vec<DeltaCase>,
    pub delta_refusals: Vec<DeltaRefusal>,
    pub scenarios: Vec<Scenario>,
    pub guards: Vec<Guard>,
    pub differences: Vec<Difference>,
}

pub fn fixture() -> Fixture {
    serde_json::from_str(include_str!("../fixtures/v2_stock_cases.json")).expect("fixture parses")
}

pub fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).unwrap_or_else(|_| panic!("{text} is a registered code"))
}

/// `"-12.34"` → −1234. The fixture always writes two decimals.
pub fn minor(text: &str) -> i128 {
    let negative = text.starts_with('-');
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    let value: i128 = digits.parse().expect("digits");
    let scaled = if text.contains('.') {
        let places = text.len() - text.find('.').unwrap() - 1;
        value * 10_i128.pow(2 - places as u32)
    } else {
        value * 100
    };
    if negative { -scaled } else { scaled }
}

pub fn signed(text: &str) -> QuantityDelta {
    QuantityDelta::from_decimal(Decimal::from_i128_with_scale(minor(text), 2)).unwrap()
}

/// A balance may be larger than a person may type, so it does not go through `Money::parse`.
pub fn money(text: &str) -> ForeignAmount {
    signed(text)
        .to_amount()
        .unwrap_or_else(|_| panic!("{text} is not negative"))
}

pub fn balances(map: &BTreeMap<String, String>) -> Balances {
    let pairs: Vec<(CurrencyCode, ForeignAmount)> =
        map.iter().map(|(c, v)| (code(c), money(v))).collect();
    Balances::of(&pairs)
}

pub fn delta(map: &BTreeMap<String, String>) -> StockDelta {
    let pairs: Vec<(CurrencyCode, QuantityDelta)> =
        map.iter().map(|(c, v)| (code(c), signed(v))).collect();
    StockDelta::of(&pairs)
}

/// What a delta holds, as the fixture writes it: non-zero entries, code → `"-12.34"`.
pub fn shown_delta(d: &StockDelta) -> BTreeMap<String, String> {
    d.non_zero()
        .into_iter()
        .map(|(c, v)| (c.as_str().to_string(), v.to_string()))
        .collect()
}

pub fn shown_balances(b: &Balances) -> BTreeMap<String, String> {
    b.non_zero()
        .into_iter()
        .map(|(c, v)| (c.as_str().to_string(), v.to_string()))
        .collect()
}
