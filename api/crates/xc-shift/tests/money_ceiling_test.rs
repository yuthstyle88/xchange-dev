//! Provided with ticket #37 (T-37). Written before the implementation: do not edit,
//! delete or weaken.
//!
//! Re-expected for CA-10 #626 by the ticket author (ruling Q7 A, design `cash-shift-transfer.md`
//! §6): the custody machine's safe and its command engine are retired, so only the shift ledger's
//! cases stay.
//!
//! The shift ledger and the safe of the custody machine on amounts no shop reaches:
//! above 64 bits of hundredths they must be exact, and past what `Decimal` holds
//! they must be refused — never left out of a result, never cut to `i64`
//! (`docs/design/xchange-v3.md` §5.1, *Money arithmetic is checked in the core*).

mod custody_common;

use custody_common::code;
use rust_decimal::Decimal;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
use xc_shift::domain::shift::entities::shift_ledger::ShiftLedger;
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::domain::shift::value_objects::stock_delta::{Direction, StockDelta, StockError};

/// The largest number of hundredths `Decimal` holds at scale 2 (2^96 − 1).
const DECIMAL_MAX_MINOR: i128 = (1_i128 << 96) - 1;
/// An amount of hundredths that does not fit in 64 bits but fits `Decimal`.
const ABOVE_I64: i128 = 1_i128 << 70;

fn money(text: &str) -> ForeignAmount {
    let m = custody_common::money(text);
    ForeignAmount::from_decimal(Decimal::from_i128_with_scale(m.minor_i128(), 2))
}

fn huge(minor: i128) -> ForeignAmount {
    ForeignAmount::from_decimal(Decimal::from_i128_with_scale(minor, 2))
}

// ── the shift ledger ─────────────────────────────────────────────────────────

#[test]
fn the_ledger_refuses_an_end_balance_past_the_decimal_range_instead_of_dropping_it() {
    let opening = Balances::of(&[
        (code("USD"), huge(DECIMAL_MAX_MINOR)),
        (CurrencyCode::THB, money("100")),
    ]);
    let one_more = StockDelta::for_movement(code("USD"), Direction::In, money("0.01"));
    match ShiftLedger::expected(&opening, &[one_more]) {
        Err(StockError::Insufficient(c)) => panic!("not a shortage, but refused as one: {c:?}"),
        Err(_) => {}
        Ok(b) => panic!(
            "an end balance past the Decimal range was not refused; USD came back as {} and THB as {}",
            b.get(code("USD")),
            b.get(CurrencyCode::THB)
        ),
    }
}

#[test]
fn the_ledger_is_exact_above_sixty_four_bits() {
    let opening = Balances::of(&[(code("USD"), huge(ABOVE_I64))]);
    let one_more = StockDelta::for_movement(code("USD"), Direction::In, money("0.01"));
    let end = ShiftLedger::expected(&opening, &[one_more]).expect("inside the range");
    assert_eq!(
        end.get(code("USD")).value(),
        Decimal::from_i128_with_scale(ABOVE_I64 + 1, 2)
    );
}

#[test]
fn a_negative_end_balance_is_still_a_shortage() {
    let opening = Balances::of(&[(code("USD"), money("1"))]);
    let out = StockDelta::for_movement(code("USD"), Direction::Out, money("1.01"));
    assert_eq!(
        ShiftLedger::expected(&opening, &[out]),
        Err(StockError::Insufficient(code("USD")))
    );
}
