use rust_decimal::Decimal;
use xc_app::domain::assistant::intent::{IntentKind, IntentParser};
use xc_kernel::money::ForeignAmount;

#[test]
fn a1_huge_amount_above_max_bound_yields_unknown() {
    let intent = IntentParser::parse("want 7922816251426433759354395033 USD", "en");
    assert_eq!(intent.kind, IntentKind::Unknown);
    assert_eq!(intent.quantity, None);
}

#[test]
fn a2_amount_one_above_bound_yields_unknown() {
    let intent = IntentParser::parse("buy 100000001 USD", "en");
    assert_eq!(intent.kind, IntentKind::Unknown);
    assert_eq!(intent.quantity, None);
}

#[test]
fn a3_amount_at_bound_yields_quote() {
    let intent = IntentParser::parse("buy 100000000 USD", "en");
    assert_eq!(intent.kind, IntentKind::Quote);
    assert_eq!(
        intent.quantity,
        Some(ForeignAmount::from_decimal(Decimal::from(100_000_000)))
    );
}

#[test]
fn a4_amount_with_comma_yields_quote() {
    let intent = IntentParser::parse("buy 1,500 USD", "en");
    assert_eq!(intent.kind, IntentKind::Quote);
    assert_eq!(
        intent.quantity,
        Some(ForeignAmount::from_decimal(Decimal::from(1500)))
    );
}

#[test]
fn a5_typical_amount_yields_quote() {
    let intent = IntentParser::parse("buy 100 USD", "en");
    assert_eq!(intent.kind, IntentKind::Quote);
    assert_eq!(
        intent.quantity,
        Some(ForeignAmount::from_decimal(Decimal::from(100)))
    );
}

#[test]
fn a6_zero_amount_yields_unknown() {
    let intent = IntentParser::parse("buy 0 USD", "en");
    assert_eq!(intent.kind, IntentKind::Unknown);
    assert_eq!(intent.quantity, None);
}
