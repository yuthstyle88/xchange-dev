//! What a shift should hold, the guard before a change, and the count at close.
//!
//! Provided with ticket T3. These tests are the specification: make them pass,
//! do not edit, delete or weaken them.

mod common;

use std::collections::BTreeMap;

use common::{balances, code, delta, fixture, money, shown_balances, shown_delta, signed};
use xc_kernel::money::{CurrencyCode, Money, Quote, Rate, Side};
use xc_shift::domain::shift::entities::shift_ledger::ShiftLedger;
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::domain::shift::value_objects::stock_delta::{Direction, StockDelta, StockError};

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn an_empty_till_holds_nothing_of_anything() {
    let b = Balances::empty();
    assert_eq!(b.get(CurrencyCode::THB), Money::ZERO);
    assert_eq!(b.get(code("USD")), Money::ZERO);
    assert!(b.non_zero().is_empty());
}

#[test]
fn a_later_pair_for_the_same_currency_replaces_the_earlier_one() {
    let b = Balances::of(&[(code("USD"), money("5")), (code("USD"), money("7.50"))]);
    assert_eq!(b.get(code("USD")), money("7.50"));
    assert_eq!(b.non_zero().len(), 1);
}

#[test]
fn a_shift_with_no_activity_expects_its_opening() {
    let opening = balances(&map(&[("THB", "500000"), ("USD", "1000")]));
    assert_eq!(ShiftLedger::expected(&opening, &[]), Ok(opening.clone()));
}

#[test]
fn a_balance_may_be_larger_than_a_person_may_type() {
    // 99,999,999.99 USD sold at 32.1 for cash: 3,209,999,999.68 baht comes in.
    let amount = Money::parse("99999999.99").unwrap();
    let q = Quote::compute(
        Side::Sell,
        amount,
        Rate::parse("32.1").unwrap(),
        Money::ZERO,
    )
    .unwrap();
    let sale = StockDelta::for_tx(Side::Sell, code("USD"), amount, &q, true).unwrap();
    let opening = balances(&map(&[("USD", "100000000")]));
    let after = ShiftLedger::expected(&opening, &[sale]).unwrap();
    assert_eq!(after.get(CurrencyCode::THB).to_string(), "3209999999.68");
    assert_eq!(after.get(code("USD")).to_string(), "0.01");
}

#[test]
fn the_order_of_the_changes_does_not_matter_only_where_they_end() {
    // Taken alone the first change would go below zero; the shift as a whole does not.
    let opening = balances(&map(&[("USD", "0")]));
    let out = StockDelta::for_movement(code("USD"), Direction::Out, money("50"));
    let inn = StockDelta::for_movement(code("USD"), Direction::In, money("80"));
    let end = ShiftLedger::expected(&opening, &[out, inn]).unwrap();
    assert_eq!(end.get(code("USD")), money("30"));
}

#[test]
fn every_shift_v2_summed_gets_the_same_balances() {
    let f = fixture();
    let (mut held, mut short) = (0, 0);
    for (n, s) in f.scenarios.iter().enumerate() {
        assert!(!s.v2_differs_from_exact, "scenario {n}");
        let mut deltas = Vec::new();
        for t in &s.transactions {
            let side = if t.r#type == "BUY" {
                Side::Buy
            } else {
                Side::Sell
            };
            let amount = Money::parse(&t.amount).unwrap();
            let q = Quote::compute(
                side,
                amount,
                Rate::parse(&t.rate).unwrap(),
                Money::parse(&t.fee).unwrap(),
            )
            .unwrap_or_else(|e| panic!("scenario {n}: {e}"));
            deltas.push(
                StockDelta::for_tx(side, code(&t.currency), amount, &q, t.method == "Cash")
                    .unwrap(),
            );
        }
        for m in &s.movements {
            let direction = if m.direction == "IN" {
                Direction::In
            } else {
                Direction::Out
            };
            deltas.push(StockDelta::for_movement(
                code(&m.currency),
                direction,
                money(&m.amount),
            ));
        }
        let got = ShiftLedger::expected(&balances(&s.opening), &deltas);
        match &s.first_negative {
            // v2 would print a negative holding; here a shift that ends below zero is refused,
            // naming the first such currency in the register's order.
            Some(c) => {
                assert_eq!(got, Err(StockError::Insufficient(code(c))), "scenario {n}");
                short += 1;
            }
            None => {
                let b = got.unwrap_or_else(|e| panic!("scenario {n}: {e}"));
                assert_eq!(shown_balances(&b), s.exact, "scenario {n}");
                held += 1;
            }
        }
    }
    assert_eq!((held, short), (15, 25));
}

#[test]
fn every_change_v2_let_through_or_refused_is_treated_the_same() {
    let f = fixture();
    let mut refused = 0;
    for (n, g) in f.guards.iter().enumerate() {
        let got = ShiftLedger::ensure_non_negative(&balances(&g.balance), &delta(&g.change));
        match &g.insufficient {
            Some(c) => {
                assert_eq!(got, Err(StockError::Insufficient(code(c))), "guard {n}");
                refused += 1;
            }
            None => assert_eq!(got, Ok(()), "guard {n}"),
        }
    }
    assert_eq!(refused, 5);
}

#[test]
fn spending_exactly_what_is_there_is_allowed_and_one_satang_more_is_not() {
    let till = balances(&map(&[("USD", "100")]));
    let all = StockDelta::for_movement(code("USD"), Direction::Out, money("100"));
    let more = StockDelta::for_movement(code("USD"), Direction::Out, money("100.01"));
    assert_eq!(ShiftLedger::ensure_non_negative(&till, &all), Ok(()));
    assert_eq!(
        ShiftLedger::ensure_non_negative(&till, &more),
        Err(StockError::Insufficient(code("USD")))
    );
}

#[test]
fn when_two_currencies_are_short_the_first_in_the_register_is_named() {
    let change = delta(&map(&[("ZWG", "-1"), ("EUR", "-1")]));
    assert_eq!(
        ShiftLedger::ensure_non_negative(&Balances::empty(), &change),
        Err(StockError::Insufficient(code("EUR")))
    );
    let with_baht = delta(&map(&[("USD", "-1"), ("THB", "-1")]));
    assert_eq!(
        ShiftLedger::ensure_non_negative(&Balances::empty(), &with_baht),
        Err(StockError::Insufficient(CurrencyCode::THB))
    );
}

#[test]
fn every_count_v2_compared_gets_the_same_difference() {
    let f = fixture();
    for (n, d) in f.differences.iter().enumerate() {
        assert!(!d.v2_differs_from_exact, "difference {n}");
        let got = ShiftLedger::difference(&balances(&d.expected), &balances(&d.counted));
        assert_eq!(shown_delta(&got), d.exact, "difference {n}");
    }
}

#[test]
fn the_difference_is_counted_minus_expected_with_its_sign() {
    let expected = balances(&map(&[("THB", "500000"), ("USD", "1000")]));
    let counted = balances(&map(&[("THB", "499999.50"), ("USD", "1000.25")]));
    let d = ShiftLedger::difference(&expected, &counted);
    assert_eq!(d.get(CurrencyCode::THB), signed("-0.50")); // short
    assert_eq!(d.get(code("USD")), signed("0.25")); // over
    assert!(
        ShiftLedger::difference(&expected, &expected)
            .non_zero()
            .is_empty()
    );
}
