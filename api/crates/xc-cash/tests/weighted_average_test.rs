//! Up-front tests for LG-1 (#923; design `docs/design/blueprint/ledger-inventory.md` §3.2, Q10): the
//! weighted average of one position, and a posting's balance per currency. Pure, no database.
//! Written by the ticket author before the implementation. Do not edit, delete or weaken; add tests
//! in a file of your own.

use std::str::FromStr;

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_cash::domain::inventory::weighted_average::WeightedAverage;
use xc_cash::domain::ledger::account::CashAccountId;
use xc_cash::domain::ledger::posting::{Entry, Posting, SourceRef, SourceType};
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal")
}

fn wa(q: &str, c: &str) -> WeightedAverage {
    WeightedAverage {
        quantity: d(q),
        cost_thb: d(c),
    }
}

#[test]
fn two_buys_average_to_32_50_and_a_sale_of_50_costs_1625() {
    let p = WeightedAverage::EMPTY
        .acquire(d("100"), d("3200.00"))
        .acquire(d("100"), d("3300.00"));
    assert_eq!(p, wa("200", "6500.00"));
    assert_eq!(p.cost_thb / p.quantity, d("32.50"), "average 32.50");

    let (cost, left) = p.dispose(d("50"));
    assert_eq!(cost, d("1625.00"));
    assert_eq!(left, wa("150", "4875.00"));
}

#[test]
fn selling_the_last_units_takes_all_the_remaining_cost() {
    let (cost, left) = wa("150", "4875.00").dispose(d("150"));
    assert_eq!(cost, d("4875.00"));
    assert_eq!(left.quantity, Decimal::ZERO);
    assert_eq!(left.cost_thb, Decimal::ZERO);
}

#[test]
fn the_last_unit_takes_what_rounding_left() {
    // 3 units cost 1.00: one unit costs 0.33 (1/3 rounded), the remaining two take 0.67, not 0.66.
    let (first, left) = wa("3", "1.00").dispose(d("1"));
    assert_eq!(first, d("0.33"));
    assert_eq!(left, wa("2", "0.67"));
    let (rest, empty) = left.dispose(d("2"));
    assert_eq!(rest, d("0.67"));
    assert_eq!(empty.cost_thb, Decimal::ZERO);
}

#[test]
fn a_cost_on_a_midpoint_rounds_half_away_from_zero() {
    // 2 units cost 0.05: one costs 0.025 → 0.03 (half away from zero), never 0.02 (half to even).
    let (cost, left) = wa("2", "0.05").dispose(d("1"));
    assert_eq!(cost, d("0.03"));
    assert_eq!(left, wa("1", "0.02"));
    // And 0.035 → 0.04, where half-to-even would also give 0.04: the 0.025 case is the one that
    // tells the strategies apart; this one guards an off-by-one in the other direction.
    let (cost, _) = wa("2", "0.07").dispose(d("1"));
    assert_eq!(cost, d("0.04"));
}

#[test]
fn acquire_adds_quantity_and_cost() {
    let p = wa("10", "320.00").acquire(d("5"), d("170.50"));
    assert_eq!(p, wa("15", "490.50"));
}

fn posting(entries: &[(&str, &str)]) -> Posting {
    Posting {
        organization_id: "00000000-0000-0000-0000-000000000001".to_string(),
        branch: BranchId::new("MAIN"),
        source: SourceRef {
            source_type: SourceType::Exchange,
            source_id: "tx-1".to_string(),
            source_key: "complete".to_string(),
        },
        posted_at: Utc.with_ymd_and_hms(2026, 10, 15, 3, 0, 0).unwrap(),
        posted_by: "test".to_string(),
        entries: entries
            .iter()
            .map(|(cur, q)| Entry {
                account: CashAccountId(Uuid::nil()),
                currency: CurrencyCode::parse(cur).expect("a currency"),
                quantity_signed: d(q),
                valuation_thb: Decimal::ZERO,
            })
            .collect(),
    }
}

#[test]
fn a_posting_is_balanced_only_when_every_currency_sums_to_zero() {
    assert!(
        posting(&[
            ("USD", "100"),
            ("USD", "-100"),
            ("THB", "-3250"),
            ("THB", "3250")
        ])
        .is_balanced()
    );
    assert!(!posting(&[("USD", "100"), ("USD", "-99")]).is_balanced());
    assert!(
        !posting(&[("USD", "100"), ("THB", "-100")]).is_balanced(),
        "balanced in total but not per currency"
    );
    assert!(posting(&[]).is_balanced(), "no entries, nothing unbalanced");
}
