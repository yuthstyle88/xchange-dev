//! Provided with ticket #37 (T-37). Written before the implementation: do not edit,
//! delete or weaken.
//!
//! The shift ledger and the safe of the custody machine on amounts no shop reaches:
//! above 64 bits of hundredths they must be exact, and past what `Decimal` holds
//! they must be refused — never left out of a result, never cut to `i64`
//! (`docs/design/xchange-v3.md` §5.1, *Money arithmetic is checked in the core*).

mod custody_common;

use custody_common::*;
use xc_kernel::money::{CurrencyCode, Money};
use xc_shift::domain::shift::entities::shift_ledger::ShiftLedger;
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::domain::shift::value_objects::stock_delta::{Direction, StockDelta, StockError};

/// The largest number of hundredths `Decimal` holds at scale 2 (2^96 − 1).
const DECIMAL_MAX_MINOR: i128 = (1_i128 << 96) - 1;
/// An amount of hundredths that does not fit in 64 bits but fits `Decimal`.
const ABOVE_I64: i128 = 1_i128 << 70;

fn huge(minor: i128) -> Money {
    Money::try_from_minor_i128(minor).expect("inside the Decimal range")
}

fn safe_amount(d: &Desk, currency: &str) -> Option<i128> {
    d.state
        .safe
        .as_ref()
        .map(|safe| safe.get(code(currency)).minor_i128())
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
    assert_eq!(end.get(code("USD")).minor_i128(), ABOVE_I64 + 1);
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

// ── the safe of the custody machine ──────────────────────────────────────────

#[test]
fn money_into_a_safe_above_sixty_four_bits_lands_exactly() {
    let (mut d, id) = Desk::transfer(RequestKind::SafeIn); // 200 USD from the drawer
    d.ok("BE1", CustodyCommand::Dispatch { id: id.clone() });
    d.sign("FIN1", &id);
    d.state.safe = Some(Balances::of(&[(code("USD"), huge(ABOVE_I64))]));
    d.commit("BE1", &id);
    assert_eq!(safe_amount(&d, "USD"), Some(ABOVE_I64 + 20_000));
}

#[test]
fn money_into_a_safe_at_the_decimal_ceiling_is_refused_and_the_safe_is_unchanged() {
    let (mut d, id) = Desk::transfer(RequestKind::SafeIn);
    d.ok("BE1", CustodyCommand::Dispatch { id: id.clone() });
    d.sign("FIN1", &id);
    d.state.safe = Some(Balances::of(&[(code("USD"), huge(DECIMAL_MAX_MINOR))]));
    let before = d.state.safe.clone();
    let got = d.act("BE1", CustodyCommand::Commit { id: id.clone() });
    assert_eq!(got.err(), Some(CustodyError::AmountInvalid));
    assert_eq!(d.state.safe, before);
}

#[test]
fn money_out_of_a_safe_above_sixty_four_bits_leaves_the_exact_rest() {
    let mut d = Desk::open();
    d.state.safe = Some(Balances::of(&[(code("EUR"), huge(ABOVE_I64))]));
    let id = d.propose(
        "F1",
        Proposal {
            receiver: Some("MGR1".into()),
            currency: Some(code("EUR")),
            amount: Some(money("300")),
            note: "float".into(),
            ..proposal(RequestKind::SafeOut, 3)
        },
    );
    d.ok("BE1", CustodyCommand::Dispatch { id: id.clone() });
    assert_eq!(safe_amount(&d, "EUR"), Some(ABOVE_I64 - 30_000));
}
