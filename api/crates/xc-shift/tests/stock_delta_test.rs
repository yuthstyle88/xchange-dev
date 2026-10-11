//! What one completed exchange, or one manual movement, does to the till.
//!
//! Provided with ticket T3. These tests are the specification: make them pass,
//! do not edit, delete or weaken them. `fixtures/v2_stock_cases.json` is what the
//! v2 engine computed (`api/tools/v2-fixtures/gen_stock.mjs`); `exact` is the same
//! figure in exact decimal arithmetic and is what the code is held to.

mod common;

use common::{code, fixture, money, shown_delta, signed};
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta, Quote, Rate, Side};
use xc_shift::domain::shift::value_objects::stock_delta::{Direction, StockDelta, StockError};

/// RE-7 #604 retires `Quote::compute`; a quote per 1 unit is `Quote::per_unit` at `UnitSize::ONE`
/// (re-expected by the ticket author, claude/07d3765f; the figures this test pins are unchanged).
fn per_one(
    side: xc_kernel::money::Side,
    amount: xc_kernel::money::Money,
    rate: xc_kernel::money::Rate,
    fee: xc_kernel::money::Money,
) -> Result<xc_kernel::money::Quote, xc_kernel::money::QuoteError> {
    let amount: rust_decimal::Decimal = amount.to_string().parse().expect("a decimal amount");
    xc_kernel::money::Quote::per_unit(
        side,
        xc_kernel::money::ForeignAmount::from_decimal(amount),
        rate,
        xc_kernel::money::UnitSize::ONE,
        fee,
    )
}

fn quote(side: Side, amount: &str, rate: &str, fee: &str) -> Quote {
    per_one(
        side,
        Money::parse(amount).unwrap(),
        Rate::parse(rate).unwrap(),
        Money::parse(fee).unwrap(),
    )
    .expect("a valid exchange")
}

#[test]
fn buying_for_cash_takes_foreign_in_and_pays_baht_out() {
    let q = quote(Side::Buy, "100", "32.1", "10");
    let d = StockDelta::for_tx(Side::Buy, code("USD"), money("100"), &q, true).unwrap();
    assert_eq!(d.get(code("USD")), signed("100.00"));
    assert_eq!(d.get(CurrencyCode::THB), signed("-3200.00")); // principal − fee
    assert_eq!(d.get(code("EUR")), MoneyDelta::ZERO);
}

#[test]
fn selling_for_cash_gives_foreign_out_and_takes_baht_in() {
    let q = quote(Side::Sell, "100", "32.1", "10");
    let d = StockDelta::for_tx(Side::Sell, code("USD"), money("100"), &q, true).unwrap();
    assert_eq!(d.get(code("USD")), signed("-100.00"));
    assert_eq!(d.get(CurrencyCode::THB), signed("3220.00")); // principal + fee
}

#[test]
fn when_the_customer_does_not_pay_in_cash_the_baht_in_the_till_does_not_move() {
    let q = quote(Side::Sell, "100", "32.1", "10");
    let d = StockDelta::for_tx(Side::Sell, code("USD"), money("100"), &q, false).unwrap();
    assert_eq!(d.get(code("USD")), signed("-100.00"));
    assert_eq!(d.get(CurrencyCode::THB), MoneyDelta::ZERO);
    assert_eq!(d.non_zero().len(), 1);
}

#[test]
fn baht_is_not_something_the_shop_exchanges() {
    let q = quote(Side::Buy, "100", "1", "0");
    assert_eq!(
        StockDelta::for_tx(Side::Buy, CurrencyCode::THB, money("100"), &q, true),
        Err(StockError::NotAStockCurrency)
    );
}

#[test]
fn a_movement_adds_or_removes_one_currency_and_baht_is_allowed() {
    let d = StockDelta::for_movement(CurrencyCode::THB, Direction::In, money("500"));
    assert_eq!(d.get(CurrencyCode::THB), signed("500.00"));
    let d = StockDelta::for_movement(code("JPY"), Direction::Out, money("0.01"));
    assert_eq!(d.get(code("JPY")), signed("-0.01"));
    assert_eq!(d.non_zero(), vec![(code("JPY"), signed("-0.01"))]);
}

#[test]
fn nothing_is_a_delta_too() {
    assert!(StockDelta::none().non_zero().is_empty());
    assert_eq!(StockDelta::none().get(code("USD")), MoneyDelta::ZERO);
}

#[test]
fn pairs_for_the_same_currency_add_up_and_zeros_are_not_listed() {
    let d = StockDelta::of(&[
        (code("USD"), signed("10.00")),
        (code("USD"), signed("-10.00")),
        (code("EUR"), signed("1.50")),
        (code("EUR"), signed("1.50")),
    ]);
    assert_eq!(d.get(code("USD")), MoneyDelta::ZERO);
    assert_eq!(d.non_zero(), vec![(code("EUR"), signed("3.00"))]);
    assert_eq!(
        StockDelta::of(&[(code("USD"), MoneyDelta::ZERO)]),
        StockDelta::none()
    );
    assert_eq!(
        StockDelta::of(&[
            (code("USD"), signed("10.00")),
            (code("USD"), signed("-10.00")),
        ]),
        StockDelta::none()
    );
}

#[test]
fn an_explicit_zero_equals_none() {
    assert_eq!(
        StockDelta::of(&[(code("USD"), MoneyDelta::ZERO)]),
        StockDelta::none()
    );
    assert_eq!(
        StockDelta::of(&[
            (code("USD"), signed("5.00")),
            (code("USD"), signed("-5.00")),
        ]),
        StockDelta::none()
    );
}

#[test]
fn entries_are_listed_in_the_registers_order_baht_first() {
    let d = StockDelta::of(&[
        (code("ZWG"), signed("1.00")),
        (code("USD"), signed("1.00")),
        (CurrencyCode::THB, signed("1.00")),
    ]);
    let order: Vec<&str> = d.non_zero().iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(order, ["THB", "USD", "ZWG"]);
}

#[test]
fn the_fixture_is_the_one_that_was_generated() {
    let f = fixture();
    assert_eq!(f.deltas.len(), 604);
    assert_eq!(f.delta_refusals.len(), 4);
    assert_eq!(f.scenarios.len(), 40);
    assert_eq!(f.guards.len(), 10);
    assert_eq!(f.differences.len(), 6);
    // v2's float arithmetic agrees with exact decimals on every recorded case.
    assert!(
        f.deltas
            .iter()
            .all(|c| !c.v2_differs_from_exact && c.v2 == c.exact)
    );
}

#[test]
fn every_transaction_v2_computed_a_delta_for_gets_the_same_delta() {
    let f = fixture();
    let mut checked = 0;
    for c in &f.deltas {
        let side = if c.r#type == "BUY" {
            Side::Buy
        } else {
            Side::Sell
        };
        let amount = Money::parse(&c.amount).unwrap();
        let q = per_one(
            side,
            amount,
            Rate::parse(&c.rate).unwrap(),
            Money::parse(&c.fee).unwrap(),
        )
        .unwrap_or_else(|e| panic!("{} {} × {} fee {}: {e}", c.r#type, c.amount, c.rate, c.fee));
        let d = StockDelta::for_tx(side, code(&c.currency), amount, &q, c.method == "Cash")
            .unwrap_or_else(|e| panic!("{} {}: {e}", c.r#type, c.currency));
        assert_eq!(
            shown_delta(&d),
            c.exact,
            "{} {} {} × {} fee {} by {}",
            c.r#type,
            c.currency,
            c.amount,
            c.rate,
            c.fee,
            c.method
        );
        checked += 1;
    }
    assert_eq!(checked, 604);
}

#[test]
fn every_currency_v2_refused_is_refused() {
    let f = fixture();
    let q = quote(Side::Buy, "1", "1", "0");
    for r in &f.delta_refusals {
        assert_eq!(r.error, "NOT_A_STOCK_CURRENCY");
        // Either it is not a code at all, or it is baht.
        let refused = match CurrencyCode::parse(&r.currency) {
            Err(_) => true,
            Ok(c) => {
                StockDelta::for_tx(Side::Buy, c, money("1"), &q, true)
                    == Err(StockError::NotAStockCurrency)
            }
        };
        assert!(refused, "{:?} must be refused", r.currency);
    }
}
