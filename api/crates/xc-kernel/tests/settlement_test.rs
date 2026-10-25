//! Up-front tests for CR-a: `Settlement::of`, the one place cash THB is rounded to the smallest coin
//! (`docs/design/blueprint/quote-transaction.md` §13, the owner's ruling of 2026-10-26: *"ปัดให้ลูกค้าได้"*).
//!
//! Written before the implementation by the ticket author (claude/bc8fd29a, xchange BE lane controller).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! What this file pins (§13.2, §13.7 CR1–CR6) [required]:
//! - BUY (the shop pays baht): `u = thb − fee`; cash rounds `u` **up** to a multiple of 0.25.
//! - SELL (the customer pays baht): `u = thb + fee`; cash rounds `u` **down** to a multiple of 0.25.
//! - `rounding() = settle − u`, a `Decimal` at scale 2 (BUY ≥ 0, SELL ≤ 0).
//! - Non-cash never rounds.
//! - A cash SELL whose rounded settle would be 0.00 is `BelowSmallestCoin`.
//! - A BUY fee above the principal is `FeeAbovePrincipal`, cash or not.

use rust_decimal::Decimal;
use std::str::FromStr;
use xc_kernel::money::{Money, Settlement, SettlementError, Side};

fn m(s: &str) -> Money {
    Money::parse(s).expect("a money literal")
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal literal")
}

/// `Money` as a scale-2 `Decimal` through its public minor-unit accessor (`Money::value` is crate-private).
fn dec(money: Money) -> Decimal {
    Decimal::new(i64::try_from(money.minor_i128()).expect("fits i64"), 2)
}

fn settle(side: Side, thb: &str, fee: &str, cash: bool) -> (Money, Decimal) {
    let s = Settlement::of(side, m(thb), m(fee), cash).expect("a settlement");
    (s.settle(), s.rounding())
}

#[test]
fn the_step_is_a_quarter_baht() {
    assert_eq!(Settlement::THB_CASH_STEP, d("0.25"));
}

#[test]
fn cr1_cash_buy_rounds_the_payout_up() {
    let (s, r) = settle(Side::Buy, "32.10", "0.00", true);
    assert_eq!(s, m("32.25"));
    assert_eq!(r, d("0.15"));
    assert_eq!(r.scale(), 2, "rounding is at scale 2");
}

#[test]
fn cr2_cash_sell_rounds_the_amount_due_down() {
    let (s, r) = settle(Side::Sell, "32.10", "0.00", true);
    assert_eq!(s, m("32.00"));
    assert_eq!(r, d("-0.10"));
    assert_eq!(r.scale(), 2, "rounding is at scale 2");
}

#[test]
fn cr3_a_quarter_multiple_is_unchanged_both_ways() {
    for side in [Side::Buy, Side::Sell] {
        let (s, r) = settle(side, "32.25", "0.00", true);
        assert_eq!(s, m("32.25"), "{side:?}");
        assert!(r.is_zero(), "{side:?}: rounding is zero");
    }
    let (s, r) = settle(Side::Buy, "100.00", "0.00", true);
    assert_eq!((s, r.is_zero()), (m("100.00"), true));
}

#[test]
fn cr4_non_cash_never_rounds() {
    let (s, r) = settle(Side::Buy, "32.10", "0.00", false);
    assert_eq!((s, r.is_zero()), (m("32.10"), true));
    let (s, r) = settle(Side::Sell, "32.10", "0.00", false);
    assert_eq!((s, r.is_zero()), (m("32.10"), true));
    let (s, r) = settle(Side::Sell, "0.10", "0.00", false);
    assert_eq!(
        (s, r.is_zero()),
        (m("0.10"), true),
        "non-cash below a coin is fine"
    );
}

#[test]
fn cr5_cash_sell_below_the_smallest_coin_is_refused() {
    assert_eq!(
        Settlement::of(Side::Sell, m("0.10"), m("0.00"), true).unwrap_err(),
        SettlementError::BelowSmallestCoin
    );
    assert_eq!(
        Settlement::of(Side::Sell, m("0.24"), m("0.00"), true).unwrap_err(),
        SettlementError::BelowSmallestCoin
    );
    let (s, _) = settle(Side::Sell, "0.25", "0.00", true);
    assert_eq!(s, m("0.25"), "exactly one coin is fine");
}

#[test]
fn cr6_fees_combine_before_rounding() {
    // BUY: u = 100.10 − 5.00 = 95.10 → 95.25.
    let (s, r) = settle(Side::Buy, "100.10", "5.00", true);
    assert_eq!((s, r), (m("95.25"), d("0.15")));
    // SELL: u = 100.10 + 5.00 = 105.10 → 105.00.
    let (s, r) = settle(Side::Sell, "100.10", "5.00", true);
    assert_eq!((s, r), (m("105.00"), d("-0.10")));
    // Non-cash keeps today's figures: BUY thb − fee, SELL thb + fee.
    assert_eq!(settle(Side::Buy, "100.10", "5.00", false).0, m("95.10"));
    assert_eq!(settle(Side::Sell, "100.10", "5.00", false).0, m("105.10"));
}

#[test]
fn the_identity_holds_for_every_satang_remainder() {
    for cents in 0..100 {
        let thb = format!("40.{cents:02}");
        for (side, cash) in [
            (Side::Buy, true),
            (Side::Sell, true),
            (Side::Buy, false),
            (Side::Sell, false),
        ] {
            let s = Settlement::of(side, m(&thb), m("1.00"), cash).expect("a settlement");
            let u = match side {
                Side::Buy => d(&thb) - d("1.00"),
                Side::Sell => d(&thb) + d("1.00"),
            };
            assert_eq!(
                dec(s.settle()),
                u + s.rounding(),
                "{side:?} cash={cash} thb={thb}: settle = u + rounding"
            );
            assert!(
                s.rounding() > d("-0.25") && s.rounding() < d("0.25"),
                "{side:?} cash={cash} thb={thb}: |rounding| < 0.25"
            );
            if cash {
                assert!(
                    (dec(s.settle()) % d("0.25")).is_zero(),
                    "{side:?} thb={thb}: cash settle is a multiple of 0.25"
                );
                match side {
                    Side::Buy => assert!(s.rounding() >= Decimal::ZERO, "BUY favours the customer"),
                    Side::Sell => {
                        assert!(s.rounding() <= Decimal::ZERO, "SELL favours the customer")
                    }
                }
            } else {
                assert!(s.rounding().is_zero(), "non-cash never rounds");
            }
        }
    }
}

#[test]
fn a_buy_fee_above_the_principal_is_refused_cash_or_not() {
    for cash in [true, false] {
        assert_eq!(
            Settlement::of(Side::Buy, m("10.00"), m("10.01"), cash).unwrap_err(),
            SettlementError::FeeAbovePrincipal,
            "cash={cash}"
        );
    }
}
