//! Ticket #4033 (A3b) — `CounterView` buy/sell rate selection, `offered` filtering/order,
//! and `facts` / `facts_without_rate` without a database.

use xc_kernel::money::{CurrencyCode, MinorUnit, Rate, Side, UnitSize};
use xc_kiosk::domain::kiosk::value_objects::counter_facts::{CounterFacts, CounterLine, LineId};
use xc_kiosk::domain::kiosk::value_objects::counter_view::{CounterRate, CounterView};
use xc_kiosk::domain::kiosk::value_objects::currency_switch::CurrencySwitch;
use xc_kiosk::domain::kiosk::value_objects::kiosk_currencies::KioskCurrencies;

fn code(s: &str) -> CurrencyCode {
    CurrencyCode::parse(s).expect("valid currency code")
}

fn rate(s: &str) -> Rate {
    Rate::parse(s).expect("valid rate")
}

fn counter_rate(id: u128, currency: &str, buy: &str, sell: &str) -> CounterRate {
    CounterRate {
        code: code(currency),
        buy: rate(buy),
        sell: rate(sell),
        unit_size: UnitSize::ONE,
        minor_unit: MinorUnit::TWO,
        line_id: LineId(id),
    }
}

fn view_with(open: bool, rates: Vec<CounterRate>) -> CounterView {
    CounterView {
        branch_name: "MAIN".to_string(),
        open,
        rates,
    }
}

/// Row 1: USD buy 32.1 / sell 32.6, `facts(USD, Buy)` -> rate 32.1.
#[test]
fn row1_facts_buy_selects_buy_rate() {
    let view = view_with(true, vec![counter_rate(1, "USD", "32.1", "32.6")]);
    let currencies = KioskCurrencies::new(&[]);

    let got = view.facts(&currencies, code("USD"), Side::Buy);

    assert_eq!(
        got,
        CounterFacts {
            open: true,
            line: Some(CounterLine {
                line_id: LineId(1),
                rate: rate("32.1"),
                unit_size: UnitSize::ONE,
                minor_unit: MinorUnit::TWO,
            }),
        }
    );
}

/// Row 2: USD buy 32.1 / sell 32.6, `facts(USD, Sell)` -> rate 32.6.
#[test]
fn row2_facts_sell_selects_sell_rate() {
    let view = view_with(true, vec![counter_rate(1, "USD", "32.1", "32.6")]);
    let currencies = KioskCurrencies::new(&[]);

    let got = view.facts(&currencies, code("USD"), Side::Sell);

    assert_eq!(
        got,
        CounterFacts {
            open: true,
            line: Some(CounterLine {
                line_id: LineId(1),
                rate: rate("32.6"),
                unit_size: UnitSize::ONE,
                minor_unit: MinorUnit::TWO,
            }),
        }
    );
}

/// Row 3: USD switched off, `facts(USD, Buy)` -> `line: None`.
#[test]
fn row3_facts_for_switched_off_currency_returns_no_line() {
    let view = view_with(true, vec![counter_rate(1, "USD", "32.1", "32.6")]);
    let currencies = KioskCurrencies::new(&[CurrencySwitch {
        code: code("USD"),
        enabled: false,
    }]);

    let got = view.facts(&currencies, code("USD"), Side::Buy);

    assert_eq!(
        got,
        CounterFacts {
            open: true,
            line: None,
        }
    );
}

/// Row 4: a currency with no rate line -> `line: None`.
#[test]
fn row4_facts_for_currency_without_rate_line_returns_no_line() {
    let view = view_with(true, vec![counter_rate(1, "USD", "32.1", "32.6")]);
    let currencies = KioskCurrencies::new(&[]);

    let got = view.facts(&currencies, code("EUR"), Side::Buy);

    assert_eq!(
        got,
        CounterFacts {
            open: true,
            line: None,
        }
    );
}

/// Row 5: two USD lines, ids 1 and 2 -> the line with id 1.
#[test]
fn row5_facts_with_duplicate_currency_lines_picks_first_line() {
    let view = view_with(
        true,
        vec![
            counter_rate(1, "USD", "32.1", "32.6"),
            counter_rate(2, "USD", "33.0", "33.5"),
        ],
    );
    let currencies = KioskCurrencies::new(&[]);

    let got = view.facts(&currencies, code("USD"), Side::Buy);

    let line = got.line.expect("first USD line should be returned");
    assert_eq!(line.line_id, LineId(1));
    assert_eq!(line.rate, rate("32.1"));
}

/// Row 6: `open: false` -> `open` false, line still filled.
#[test]
fn row6_facts_when_closed_preserves_open_false_and_fills_line() {
    let view = view_with(false, vec![counter_rate(1, "USD", "32.1", "32.6")]);
    let currencies = KioskCurrencies::new(&[]);

    let got = view.facts(&currencies, code("USD"), Side::Buy);

    assert!(!got.open);
    assert_eq!(
        got.line,
        Some(CounterLine {
            line_id: LineId(1),
            rate: rate("32.1"),
            unit_size: UnitSize::ONE,
            minor_unit: MinorUnit::TWO,
        })
    );
}

/// Row 7: `offered` with GBP off -> the view's order, GBP dropped.
#[test]
fn row7_offered_drops_switched_off_currency_and_preserves_view_order() {
    let eur = counter_rate(10, "EUR", "37.0", "37.5");
    let gbp = counter_rate(20, "GBP", "42.0", "42.5");
    let usd = counter_rate(30, "USD", "32.1", "32.6");
    // Not alphabetical, so an `offered` that sorted by code instead of keeping the view's order goes red.
    let view = view_with(true, vec![usd, gbp, eur]);
    let currencies = KioskCurrencies::new(&[CurrencySwitch {
        code: code("GBP"),
        enabled: false,
    }]);

    let got = view.offered(&currencies);

    assert_eq!(got, vec![usd, eur]);
}

/// Row 8: `facts_without_rate` -> `{open, line: None}` for both open and closed views.
#[test]
fn row8_facts_without_rate_returns_open_flag_and_no_line() {
    let open_view = view_with(true, vec![counter_rate(1, "USD", "32.1", "32.6")]);
    let closed_view = view_with(false, vec![counter_rate(1, "USD", "32.1", "32.6")]);

    assert_eq!(
        open_view.facts_without_rate(),
        CounterFacts {
            open: true,
            line: None,
        }
    );
    assert_eq!(
        closed_view.facts_without_rate(),
        CounterFacts {
            open: false,
            line: None,
        }
    );
}
