//! Shared helpers of the pricing-formula up-front tests. Written before the implementation. Do not edit, delete or
//! weaken.
#![allow(dead_code)]

use rust_decimal::Decimal;
use serde_json::json;
use std::str::FromStr;
use xc_kernel::money::{
    CurrencyCode, CurrencyUnits, MinorUnit, Money, Rate, RatePercent, UnitSize,
};
use xc_rate::domain::formula::price_formula::PriceFormula;
use xc_rate::domain::formula::text::FormulaText;
use xc_rate::domain::policy::policy_tiers::{PolicyTier, PolicyTiers};
use xc_rate::domain::policy::rate_policy::RatePolicy;
use xc_rate::domain::reference::reference_batch::ReferenceLine;

pub fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal")
}

pub fn code(s: &str) -> CurrencyCode {
    CurrencyCode::parse(s).expect("a known code")
}

pub fn rate(s: &str) -> Rate {
    Rate::parse(s).expect("a valid rate")
}

pub fn pct(s: &str) -> RatePercent {
    RatePercent::parse(s).expect("a valid percent")
}

pub fn units(currency: &str, size: &str) -> CurrencyUnits {
    CurrencyUnits::new(
        code(currency),
        UnitSize::parse(size).expect("a unit size"),
        MinorUnit::TWO,
    )
}

pub fn reference(
    currency: &str,
    ref_unit: &str,
    buy: Option<&str>,
    sell: Option<&str>,
) -> ReferenceLine {
    ReferenceLine::new(
        code(currency),
        UnitSize::parse(ref_unit).expect("a unit size"),
        buy.map(rate),
        sell.map(rate),
    )
}

/// A policy whose USD/JPY/EUR guard is `[min, max]` %.
pub fn policy_guarded(buy: &str, sell: &str, min: &str, max: &str) -> RatePolicy {
    let g = json!({ "min_pct": min, "max_pct": max });
    let guards = json!({
        "USD": { "buy": g, "sell": g },
        "JPY": { "buy": g, "sell": g },
        "EUR": { "buy": g, "sell": g },
    });
    RatePolicy::new("policy-under-test", pct(buy), pct(sell), &guards).expect("a valid policy")
}

/// Wide guards (±50 %).
pub fn policy(buy: &str, sell: &str) -> RatePolicy {
    policy_guarded(buy, sell, "-50.0000", "50.0000")
}

pub fn formula(buy: &str, sell: &str) -> PriceFormula {
    PriceFormula::new(
        FormulaText::parse(buy).expect("buy parses"),
        FormulaText::parse(sell).expect("sell parses"),
    )
}

/// `ref_mid - 0.05` / `ref_mid + 0.05` (B3, B22, B26).
pub fn mid_formula() -> PriceFormula {
    formula("ref_mid - 0.05", "ref_mid + 0.05")
}

/// rates.md §12.3's example tiers: T1 (0, 0), T2 (+0.1, −0.1), T3 (+0.2, −0.2), T4 (+0.3, −0.3).
pub fn example_tiers() -> PolicyTiers {
    PolicyTiers::new(vec![
        PolicyTier::new(
            1,
            Some(Money::parse("20000.00").unwrap()),
            pct("0.0000"),
            pct("0.0000"),
        ),
        PolicyTier::new(
            2,
            Some(Money::parse("100000.00").unwrap()),
            pct("+0.1000"),
            pct("-0.1000"),
        ),
        PolicyTier::new(
            3,
            Some(Money::parse("500000.00").unwrap()),
            pct("+0.2000"),
            pct("-0.2000"),
        ),
        PolicyTier::new(4, None, pct("+0.3000"), pct("-0.3000")),
    ])
    .expect("valid example tiers")
}

/// The 10-28 BOT USD line (design §0).
pub fn usd_1028() -> ReferenceLine {
    reference("USD", "1", Some("33.6482"), Some("33.9671"))
}
