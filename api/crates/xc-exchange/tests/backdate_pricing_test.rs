//! Backdated keying (`docs/design/backdated-keying.md` §6, behaviour table B1, B8, B17; owner rule R4): a line
//! priced at the typed rate.
//! Written before the implementation by the sub-controller claude/6bb512ce. Do not edit, delete or weaken;
//! add tests in a file of your own.

use rust_decimal::Decimal;
use xc_exchange::domain::transaction::backdate_pricing::{
    BackdatePricing, PricedLine, PricingRefusal,
};
use xc_kernel::money::{ForeignAmount, Money, Rate, Side, UnitSize};

fn qty(t: &str) -> ForeignAmount {
    ForeignAmount::from_decimal(t.parse::<Decimal>().unwrap())
}
fn rate(t: &str) -> Rate {
    Rate::parse(t).unwrap()
}
fn unit(t: &str) -> UnitSize {
    UnitSize::parse(t).unwrap()
}
fn thb(t: &str) -> Money {
    Money::parse(t).unwrap()
}

/// B1: BUY 500 USD at 32.15, no fee: 16075.00 paid to the customer.
#[test]
fn p1_buy_usd_no_fee() {
    assert_eq!(
        BackdatePricing::price(
            Side::Buy,
            qty("500"),
            rate("32.15"),
            unit("1"),
            thb("0"),
            None
        ),
        Ok(PricedLine {
            thb_amount: thb("16075.00"),
            fee_thb: thb("0.00"),
            settle_thb: thb("16075.00"),
        })
    );
}

/// B17: JPY is quoted per 100: 10000 × 0.2155 ÷ 100 = 21.55.
#[test]
fn p2_jpy_per_unit_size_100() {
    let got = BackdatePricing::price(
        Side::Buy,
        qty("10000"),
        rate("21.55"),
        unit("100"),
        thb("0"),
        None,
    )
    .unwrap();
    assert_eq!(got.thb_amount, thb("2155.00"));
    let got = BackdatePricing::price(
        Side::Buy,
        qty("10000"),
        rate("0.2155"),
        unit("100"),
        thb("0"),
        None,
    )
    .unwrap();
    assert_eq!(got.thb_amount, thb("21.55"));
}

/// HALF_UP once at the end: 3 × 1.005 = 3.015 → 3.02 (never 3.01).
#[test]
fn p3_rounds_half_up_once() {
    let got = BackdatePricing::price(
        Side::Buy,
        qty("3"),
        rate("1.005"),
        unit("1"),
        thb("0"),
        None,
    )
    .unwrap();
    assert_eq!(got.thb_amount, thb("3.02"));
}

/// BUY: the fee is taken from what the shop pays; SELL: it is added to what the customer pays.
#[test]
fn p4_fee_by_side() {
    let buy = BackdatePricing::price(
        Side::Buy,
        qty("100"),
        rate("32"),
        unit("1"),
        thb("50"),
        None,
    )
    .unwrap();
    assert_eq!(
        (buy.thb_amount, buy.fee_thb, buy.settle_thb),
        (thb("3200.00"), thb("50.00"), thb("3150.00"))
    );
    let sell = BackdatePricing::price(
        Side::Sell,
        qty("100"),
        rate("32"),
        unit("1"),
        thb("50"),
        None,
    )
    .unwrap();
    assert_eq!(
        (sell.thb_amount, sell.fee_thb, sell.settle_thb),
        (thb("3200.00"), thb("50.00"), thb("3250.00"))
    );
}

/// A BUY fee as large as the amount leaves nothing to settle.
#[test]
fn p5_settle_not_positive_is_refused() {
    assert_eq!(
        BackdatePricing::price(Side::Buy, qty("1"), rate("32"), unit("1"), thb("32"), None),
        Err(PricingRefusal::SettleNotPositive)
    );
    assert_eq!(PricingRefusal::SettleNotPositive.field(), "feeThb");
}

/// `expectedSettleThb` equal to the computed settle passes.
#[test]
fn p6_expected_settle_equal_passes() {
    assert!(
        BackdatePricing::price(
            Side::Buy,
            qty("500"),
            rate("32.15"),
            unit("1"),
            thb("0"),
            Some(thb("16075.00")),
        )
        .is_ok()
    );
}

/// B8: one satang off is refused, naming the computed figure.
#[test]
fn p7_expected_settle_mismatch_names_the_computed_figure() {
    let got = BackdatePricing::price(
        Side::Buy,
        qty("500"),
        rate("32.15"),
        unit("1"),
        thb("0"),
        Some(thb("16075.01")),
    );
    let refusal = got.unwrap_err();
    assert_eq!(
        refusal,
        PricingRefusal::SettleMismatch {
            computed: thb("16075.00")
        }
    );
    assert_eq!(refusal.field(), "expectedSettleThb");
    assert!(
        refusal.message().contains("16075.00"),
        "{}",
        refusal.message()
    );
}

/// Overflow names the quantity.
#[test]
fn p8_overflow_names_quantity() {
    assert_eq!(PricingRefusal::Overflow.field(), "quantity");
}
