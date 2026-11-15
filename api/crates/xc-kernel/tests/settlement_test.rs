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
//!
//! Restated for GA-4 (#6928, `docs/design/blueprint/owner-set-values.md` row 5, G-7 to G-9): the step is
//! no longer the constant `THB_CASH_STEP` but an input, the Owner's `thb_cash_step`. Every case above
//! passes a step of 0.25 and keeps its assertions; `the_step_is_a_quarter_baht` is dropped, because the
//! step is now an input. The GA-4 cases are at the end.

use rust_decimal::Decimal;
use std::str::FromStr;
use xc_kernel::money::{CashStep, CashStepError, Money, Settlement, SettlementError, Side};

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

/// The step every pre-GA-4 case was written against: a quarter baht, now passed in.
fn quarter() -> Option<CashStep> {
    Some(CashStep::new(d("0.25")).expect("a cash step"))
}

fn settle(side: Side, thb: &str, fee: &str, cash: bool) -> (Money, Decimal) {
    let s = Settlement::of(side, m(thb), m(fee), cash, quarter()).expect("a settlement");
    (s.settle(), s.rounding())
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
        Settlement::of(Side::Sell, m("0.10"), m("0.00"), true, quarter()).unwrap_err(),
        SettlementError::BelowSmallestCoin
    );
    assert_eq!(
        Settlement::of(Side::Sell, m("0.24"), m("0.00"), true, quarter()).unwrap_err(),
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
            let s =
                Settlement::of(side, m(&thb), m("1.00"), cash, quarter()).expect("a settlement");
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
            Settlement::of(Side::Buy, m("10.00"), m("10.01"), cash, quarter()).unwrap_err(),
            SettlementError::FeeAbovePrincipal,
            "cash={cash}"
        );
    }
}

// --- GA-4 (#6928): the step is the Owner's rule ---------------------------------------------------

fn step(text: &str) -> Option<CashStep> {
    Some(CashStep::new(d(text)).expect("a cash step"))
}

/// G-7: with the Owner's step 0.25, a line of 1,000.13 settles to 1,000.00 when the customer buys (the
/// shop's `Side::Sell`: the amount due rounds down) and to 1,000.25 when the customer sells (the shop's
/// `Side::Buy`: the payout rounds up). The design's "buy"/"sell" are the customer's. The settlement reports
/// the step it used.
#[test]
fn g7_a_quarter_step_from_the_rules_settles_a_line() {
    let buy = Settlement::of(Side::Buy, m("1000.13"), m("0.00"), true, step("0.25")).unwrap();
    assert_eq!((buy.settle(), buy.rounding()), (m("1000.25"), d("0.12")));
    assert_eq!(buy.cash_step(), step("0.25"));
    let sell = Settlement::of(Side::Sell, m("1000.13"), m("0.00"), true, step("0.25")).unwrap();
    assert_eq!((sell.settle(), sell.rounding()), (m("1000.00"), d("-0.13")));
    assert_eq!(sell.cash_step(), step("0.25"));
}

/// G-8: the same line with a step of 0.50 settles to the 0.50 step.
#[test]
fn g8_a_half_baht_step_settles_to_the_half_baht() {
    let buy = Settlement::of(Side::Buy, m("1000.13"), m("0.00"), true, step("0.50")).unwrap();
    assert_eq!((buy.settle(), buy.rounding()), (m("1000.50"), d("0.37")));
    assert_eq!(buy.cash_step(), step("0.50"));
    let sell = Settlement::of(Side::Sell, m("1000.13"), m("0.00"), true, step("0.50")).unwrap();
    assert_eq!((sell.settle(), sell.rounding()), (m("1000.00"), d("-0.13")));
    let sell = Settlement::of(Side::Sell, m("1000.63"), m("0.00"), true, step("0.50")).unwrap();
    assert_eq!((sell.settle(), sell.rounding()), (m("1000.50"), d("-0.13")));
}

/// G-9: cash with no step is refused, on either side.
#[test]
fn g9_cash_without_a_step_is_cash_step_not_set() {
    for side in [Side::Buy, Side::Sell] {
        assert_eq!(
            Settlement::of(side, m("1000.13"), m("0.00"), true, None).unwrap_err(),
            SettlementError::CashStepNotSet,
            "{side:?}"
        );
    }
}

/// G-9, second half: a non-cash settlement needs no step, never rounds and records none.
#[test]
fn g9_non_cash_needs_no_step_and_never_rounds() {
    for side in [Side::Buy, Side::Sell] {
        for given in [None, step("0.25")] {
            let s = Settlement::of(side, m("1000.13"), m("0.00"), false, given).unwrap();
            assert!(s.rounding().is_zero(), "{side:?} {given:?}");
            assert_eq!(s.cash_step(), None, "{side:?} {given:?}");
        }
    }
}

#[test]
fn a_cash_step_is_positive_with_at_most_two_decimals() {
    assert_eq!(CashStep::new(d("0")), Err(CashStepError::NotPositive));
    assert_eq!(CashStep::new(d("-0.25")), Err(CashStepError::NotPositive));
    assert_eq!(
        CashStep::new(d("0.125")),
        Err(CashStepError::TooManyDecimals)
    );
    assert_eq!(CashStep::new(d("10000")), Err(CashStepError::TooLarge));
    assert_eq!(CashStep::new(d("0.5")).unwrap().value(), d("0.50"));
    assert_eq!(CashStep::new(d("1.000")).unwrap().value(), d("1.00"));
}
