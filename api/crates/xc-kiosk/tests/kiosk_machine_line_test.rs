//! Ticket #756 (RE-4b) — the kiosk machine prices from the published line it is handed, keeps
//! that line on the quote, and refuses a confirm once the line is not the current one.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §3.2 (kiosk BUY = `ExchangeArithmetic::thb`; SELL by
//! baht = `foreign_for_thb`, truncated to the minor unit, then `thb`), §3.3 (HALF_UP once),
//! §4.8, §4.9 R1 (the quote keeps the line's id, rate per `unit_size` and `unit_size`), R2, R4
//! (confirm refuses when the current line is not the stored one, with the existing "the rate
//! changed" refusal).
//!
//! The same rules are pinned through the route and the database by
//! `api/tests/rates_re4_kiosk_test.rs`; this file holds the machine alone.
//!
//! Needs no database; nothing here skips.
//!
//! Signatures this test assumes (pinned by these tests and the amended kiosk tests; the ticket
//! names them [proposed]):
//! - `CounterFacts { open: bool, line: Option<CounterLine> }` and
//!   `CounterLine { line_id: LineId, rate: Rate, unit_size: UnitSize, minor_unit: MinorUnit }`
//!   in `xc_kiosk::domain::kiosk::value_objects::counter_facts`, with `LineId(pub u128)`
//!   (`Copy`, a `published_rate_lines.id`);
//! - `KioskQuote` gains `unit_size: UnitSize` and `line_id: Option<LineId>` (`None` on a row
//!   written before RE-4);
//! - `KioskMachine::quote` and `KioskMachine::next` keep their signatures.

mod kiosk_common;

use kiosk_common::*;
use xc_kernel::money::{MinorUnit, Side, UnitSize};

/// Noon in Bangkok on 6 October 2026.
const T0: i64 = 1_791_262_800_000;
const ID: &str = "3fa9c1d2-7e57-4000-8000-0000000756b1";

/// A JPY line as RU-1's register and a publication give it: per 100, whole yen.
fn yen(id: u128, rate_text: &str) -> CounterLine {
    CounterLine {
        line_id: LineId(id),
        rate: rate(rate_text),
        unit_size: UnitSize::parse("100").expect("100"),
        minor_unit: MinorUnit::ZERO,
    }
}

fn open_on(line: CounterLine) -> CounterFacts {
    CounterFacts {
        open: true,
        line: Some(line),
    }
}

fn wants(side: Side, amount: &str) -> QuoteInput {
    QuoteInput {
        side: Some(side),
        currency: Some(code("JPY")),
        amount: Some(money(amount)),
        payment_method: None,
    }
}

fn quote_on(input: &QuoteInput, facts: &CounterFacts) -> KioskRequest {
    KioskMachine::quote(input, facts, ID, at(T0))
        .unwrap_or_else(|e| panic!("the machine quotes: {e}"))
}

fn confirm(request: &KioskRequest, facts: &CounterFacts) -> Result<Transition, KioskError> {
    KioskMachine::next(
        Some(request),
        &KioskCommand::Confirm,
        &KioskActor::Customer,
        facts,
        at(T0 + 10_000),
    )
}

/// R1, R2: 1,000 JPY at 23.1 per 100 is 231.00 THB, and the quote keeps line 7, its rate per
/// 100 and the unit 100.
#[test]
fn a_quote_keeps_the_line_its_rate_and_unit_size() {
    let made = quote_on(&wants(Side::Buy, "1000"), &open_on(yen(7, "23.1")));
    let q = made.quote;
    assert_eq!(q.line_id, Some(LineId(7)));
    assert_eq!(q.rate, rate("23.1"));
    assert_eq!(q.unit_size, UnitSize::parse("100").expect("100"));
    assert_eq!((q.foreign, q.thb), (money("1000"), money("231")));
    assert_eq!(q.input_amount, money("1000"));
}

/// R2, §3.3: 15 JPY at 23.1 per 100 is 3.465 THB, 3.47 half up (3.46 by banker's rounding).
#[test]
fn a_buy_rounds_half_up_once() {
    let q = quote_on(&wants(Side::Buy, "15"), &open_on(yen(7, "23.1"))).quote;
    assert_eq!(q.thb, money("3.47"));
}

/// §3.2: 1,000 THB at 23.5 per 100 buys 4,255.319… JPY, truncated to whole yen = 4,255, which
/// cost 999.925 = 999.93 THB half up (999.92 by banker's rounding).
#[test]
fn a_sell_by_baht_truncates_to_the_minor_unit_then_rounds_half_up() {
    let q = quote_on(&wants(Side::Sell, "1000"), &open_on(yen(8, "23.5"))).quote;
    assert_eq!(q.foreign, money("4255"));
    assert_eq!(q.thb, money("999.93"));
    assert_eq!(q.input_amount, money("1000"));
    assert_eq!(q.line_id, Some(LineId(8)));
}

/// R4: the current line is another one at the very same rate (a new publication): the confirm
/// is refused with the existing "the rate changed" sentence. The quoted line confirms.
#[test]
fn a_confirm_on_another_line_at_the_same_rate_is_refused() {
    let q = quote_on(&wants(Side::Buy, "1000"), &open_on(yen(7, "23.1")));
    assert_eq!(
        confirm(&q, &open_on(yen(9, "23.1"))).map(|t| t.request.status),
        Err(KioskError::RateChangedOrClosed),
        "line 9 is not line 7, whatever its rate"
    );
    let t = confirm(&q, &open_on(yen(7, "23.1"))).expect("the quoted line is current");
    assert_eq!(t.request.status, KioskStatus::Waiting);
    assert_eq!(
        t.request.quote, q.quote,
        "confirming changes nothing on the quote"
    );
}

/// R4: a request stored before RE-4 has no line id; no current line is it, so its confirm is
/// refused even at the same rate.
#[test]
fn a_quote_without_a_line_is_refused_at_confirm() {
    let mut q = quote_on(&wants(Side::Buy, "1000"), &open_on(yen(7, "23.1")));
    q.quote.line_id = None;
    assert_eq!(
        confirm(&q, &open_on(yen(7, "23.1"))).map(|t| t.request.status),
        Err(KioskError::RateChangedOrClosed)
    );
}
