//! XC-V9-PRICING PV-19 (pricing-tiers.md §15.10 "Kiosk BUY", gap 10; `H:` §8 `BUY: foreign = round(amount × 100) /
//! 100`): a kiosk BUY whose foreign amount has more decimals than the currency's minor unit is rounded half up to the
//! minor unit and quoted, not refused (`kiosk_machine.rs` step 4, `ForeignAmount::parse` refuses it today). A SELL by
//! baht keeps truncating the foreign amount to the minor unit (`rates.md` §3.2). `H:` is the owner's handoff
//! `PRICING-DESIGN.md` (kept out of the repo). Written before the implementation by the sub-controller claude/451f0116
//! (XC-V9-PRICING skeleton). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! JPY is quoted per 100 with minor unit 0 (RU-1's register, `migrations/20261010100000_rate_units.sql`: `unit_size =
//! 100`, `minor_unit = 0`), so "the minor unit" is whole yen.
//!
//! Needs no database; nothing here skips.

mod kiosk_common;

use kiosk_common::*;
use xc_kernel::money::{MinorUnit, Side, UnitSize};

/// Noon in Bangkok on 6 October 2026.
const T0: i64 = 1_791_262_800_000;
const ID: &str = "3fa9c1d2-7e57-4000-8000-0000000019b1";

/// A JPY line as RU-1's register and a publication give it: per 100, whole yen.
fn yen(rate_text: &str) -> CounterFacts {
    CounterFacts {
        open: true,
        line: Some(CounterLine {
            line_id: LineId(19),
            rate: rate(rate_text),
            unit_size: UnitSize::parse("100").expect("100"),
            minor_unit: MinorUnit::ZERO,
        }),
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

fn quote(side: Side, amount: &str, rate_text: &str) -> Result<KioskQuote, KioskError> {
    KioskMachine::quote(&wants(side, amount), &yen(rate_text), ID, at(T0)).map(|r| r.quote)
}

/// PV-19 (§15.10; `H:` §8): BUY JPY 1,000.5 at 23.1 per 100 is rounded half up to whole yen = 1,001 JPY and quoted:
/// 1,001 × 23.1 ÷ 100 = 231.231 → 231.23 THB (half up, `rates.md` §3.3).
#[test]
fn a_buy_of_1000_point_5_yen_is_rounded_half_up_to_1001_and_quoted() {
    let q = quote(Side::Buy, "1000.5", "23.1").expect("rounded and quoted, not refused");
    assert_eq!(q.foreign, money("1001"));
    assert_eq!(q.thb, money("231.23"));
    assert_eq!(q.side, Side::Buy);
    assert_eq!(q.line_id, Some(LineId(19)));
}

/// PV-19 (§15.10): half up rounds once at the minor unit — 1,000.49 JPY is 1,000 (not 1,000.5 then 1,001), and 1,000.4
/// is 1,000. 1,000 × 23.1 ÷ 100 = 231.00 THB.
#[test]
fn a_buy_below_the_half_rounds_down_once() {
    for amount in ["1000.49", "1000.4"] {
        let q = quote(Side::Buy, amount, "23.1").unwrap_or_else(|e| panic!("{amount}: {e}"));
        assert_eq!(q.foreign, money("1000"), "{amount}");
        assert_eq!(q.thb, money("231"), "{amount}");
    }
}

/// PV-19 (§15.10 "SELL keeps truncating"; `rates.md` §3.2): next to a rounded BUY, a SELL of 1,005 THB at 23.1 per 100
/// buys 1,005 × 100 ÷ 23.1 = 4,350.649… JPY, truncated to 4,350 (half up would be 4,351); 4,350 × 0.231 = 1,004.85 THB.
#[test]
fn a_buy_rounds_half_up_while_a_sell_still_truncates() {
    let buy = quote(Side::Buy, "4350.65", "23.1").expect("BUY rounded and quoted");
    assert_eq!(
        buy.foreign,
        money("4351"),
        "BUY: 4,350.65 half up to whole yen"
    );
    let sell = quote(Side::Sell, "1005", "23.1").expect("SELL quoted");
    assert_eq!(sell.foreign, money("4350"), "SELL: truncated, not rounded");
    assert_eq!(sell.thb, money("1004.85"));
    assert_eq!(sell.input_amount, money("1005"));
}
