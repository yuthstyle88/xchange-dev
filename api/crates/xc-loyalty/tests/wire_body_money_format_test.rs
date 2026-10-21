use std::str::FromStr;

use chrono::{NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_kernel::Mode;
use xc_kernel::money::Side;
use xc_loyalty::domain::customer_ref::LoyaltyCustomerRef;
use xc_loyalty::domain::facts::{CompletedFacts, CompletedLine, ReversedFacts};
use xc_loyalty::domain::wire_body::LoyaltyWireBody;

const TENANT_ID: &str = "6f1c2d3e-0000-4000-8000-00000000a001";
const EVENT_ID: &str = "6f1c2d3e-0000-4000-8000-00000000e001";
const PHONE: &str = "+66812345678";

fn uuid(s: &str) -> Uuid {
    Uuid::parse_str(s).expect("a uuid literal")
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal literal")
}

fn base_completed_facts(lines: Vec<CompletedLine>) -> CompletedFacts {
    CompletedFacts {
        organization_id: uuid("00000000-0000-0000-0000-000000000001"),
        transaction_id: uuid("6f1c2d3e-0000-4000-8000-0000000000c1"),
        document_ref: "XC-B01-20261001-0007".to_owned(),
        customer_id: Some("cust-42".to_owned()),
        mode: Mode::Live,
        branch_id: "XC-B01".to_owned(),
        completed_at: Utc.with_ymd_and_hms(2026, 10, 1, 3, 4, 5).unwrap(),
        business_date: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        reversible_until: Utc.with_ymd_and_hms(2026, 10, 1, 16, 59, 59).unwrap(),
        lines,
    }
}

fn base_reversed_facts(amount_thb: Decimal) -> ReversedFacts {
    ReversedFacts {
        organization_id: uuid("00000000-0000-0000-0000-000000000001"),
        reversal_id: uuid("6f1c2d3e-0000-4000-8000-0000000000d1"),
        transaction_id: uuid("6f1c2d3e-0000-4000-8000-0000000000c1"),
        customer_id: Some("cust-42".to_owned()),
        posted_at: Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap(),
        amount_thb,
        full: true,
    }
}

#[test]
fn completed_wire_zero_amounts_formatted_with_trailing_zeros() {
    let facts = base_completed_facts(vec![CompletedLine {
        currency: "THB".to_owned(),
        shop_side: Side::Buy,
        quantity: dec("0"),
        unit_size: dec("1"),
        thb_amount: dec("0"),
        settle_thb: dec("0"),
        realized_margin_thb: None,
    }]);

    let body = LoyaltyWireBody::completed(uuid(EVENT_ID), &facts, uuid(TENANT_ID), PHONE);
    let line = &body["data"]["lines"][0];

    assert_eq!(line["thbAmount"], "0.00");
    assert_eq!(line["settleThb"], "0.00");
    assert_eq!(line["quantity"], "0.0000");
    assert_eq!(line["unitSize"], "1.0000");
}

#[test]
fn completed_wire_negative_amounts_preserve_sign() {
    let facts = base_completed_facts(vec![CompletedLine {
        currency: "USD".to_owned(),
        shop_side: Side::Sell,
        quantity: dec("100"),
        unit_size: dec("1"),
        thb_amount: dec("-5.50"),
        settle_thb: dec("-5.50"),
        realized_margin_thb: None,
    }]);

    let body = LoyaltyWireBody::completed(uuid(EVENT_ID), &facts, uuid(TENANT_ID), PHONE);
    let line = &body["data"]["lines"][0];

    assert_eq!(line["thbAmount"], "-5.50");
    assert_eq!(line["settleThb"], "-5.50");
}

#[test]
fn completed_wire_quantity_unit_size_four_decimal_places() {
    let facts = base_completed_facts(vec![CompletedLine {
        currency: "EUR".to_owned(),
        shop_side: Side::Buy,
        quantity: dec("1.5"),
        unit_size: dec("1"),
        thb_amount: dec("50"),
        settle_thb: dec("50"),
        realized_margin_thb: None,
    }]);

    let body = LoyaltyWireBody::completed(uuid(EVENT_ID), &facts, uuid(TENANT_ID), PHONE);
    let line = &body["data"]["lines"][0];

    assert_eq!(line["quantity"], "1.5000");
    assert_eq!(line["unitSize"], "1.0000");
}

#[test]
fn completed_wire_realized_margin_optional_none_and_some() {
    let facts = base_completed_facts(vec![
        CompletedLine {
            currency: "USD".to_owned(),
            shop_side: Side::Buy,
            quantity: dec("10"),
            unit_size: dec("1"),
            thb_amount: dec("340"),
            settle_thb: dec("340"),
            realized_margin_thb: None,
        },
        CompletedLine {
            currency: "JPY".to_owned(),
            shop_side: Side::Sell,
            quantity: dec("1000"),
            unit_size: dec("100"),
            thb_amount: dec("230"),
            settle_thb: dec("230"),
            realized_margin_thb: Some(dec("12.0")),
        },
    ]);

    let body = LoyaltyWireBody::completed(uuid(EVENT_ID), &facts, uuid(TENANT_ID), PHONE);
    let lines = &body["data"]["lines"];

    assert!(lines[0]["realizedMarginThb"].is_null());
    assert_eq!(lines[1]["realizedMarginThb"], "12.00");
}

#[test]
fn completed_wire_missing_customer_id_uses_empty_string_ref() {
    let mut facts = base_completed_facts(vec![]);
    facts.customer_id = None;

    let body = LoyaltyWireBody::completed(uuid(EVENT_ID), &facts, uuid(TENANT_ID), PHONE);
    let expected_ref = LoyaltyCustomerRef::of(facts.organization_id, "").to_string();

    assert_eq!(body["data"]["customerRef"], expected_ref);
}

#[test]
fn reversed_wire_zero_and_negative_amount_formatted() {
    let facts_zero = base_reversed_facts(dec("0"));
    let body_zero = LoyaltyWireBody::reversed(uuid(EVENT_ID), &facts_zero, uuid(TENANT_ID), PHONE);
    assert_eq!(body_zero["data"]["amountThb"], "0.00");

    let facts_neg = base_reversed_facts(dec("-100.5"));
    let body_neg = LoyaltyWireBody::reversed(uuid(EVENT_ID), &facts_neg, uuid(TENANT_ID), PHONE);
    assert_eq!(body_neg["data"]["amountThb"], "-100.50");
}

// ---------------------------------------------------------------- #4131: half-up rounding (owner ruling)
//
// #4131 (owner 2026-10-21, "ทำตามที่แนะนำทั้ง 5 ข้อ", recommendation 3): amounts sent to Loyalty round half-up
// (`RoundingStrategy::MidpointAwayFromZero`) — THB fields to 2 places, quantity and unit size to 4 — before they are
// formatted. Today `format!("{:.2}", Decimal)` cuts the extra places off (1.019 reads "1.01").
//
// Written before the implementation by the BE-R6 sub-controller (claude/studio-claude-8). Do not edit, delete or
// weaken; add tests of your own below them. The rows above (#4007) are row 4 of #4131: unchanged.

fn line_with(thb: &str, quantity: &str, unit_size: &str, margin: Option<&str>) -> CompletedLine {
    CompletedLine {
        currency: "USD".to_owned(),
        shop_side: Side::Buy,
        quantity: dec(quantity),
        unit_size: dec(unit_size),
        thb_amount: dec(thb),
        settle_thb: dec(thb),
        realized_margin_thb: margin.map(dec),
    }
}

fn completed_line(line: CompletedLine) -> serde_json::Value {
    let body = LoyaltyWireBody::completed(
        uuid(EVENT_ID),
        &base_completed_facts(vec![line]),
        uuid(TENANT_ID),
        PHONE,
    );
    body["data"]["lines"][0].clone()
}

/// Row 1: a half satang rounds up, on every THB field of a completed line.
#[test]
fn completed_thb_fields_round_half_up_to_two_places() {
    for (thb, expected) in [
        ("1.005", "1.01"),
        ("1.015", "1.02"),
        ("2.675", "2.68"),
        ("1.019", "1.02"),
        ("1.999", "2.00"),
        ("1.004", "1.00"),
    ] {
        let line = completed_line(line_with(thb, "1", "1", Some(thb)));
        assert_eq!(line["thbAmount"], expected, "thbAmount of {thb}");
        assert_eq!(line["settleThb"], expected, "settleThb of {thb}");
        assert_eq!(
            line["realizedMarginThb"], expected,
            "realizedMarginThb of {thb}"
        );
    }
}

/// Row 2: a negative half satang rounds away from zero.
#[test]
fn completed_negative_thb_rounds_away_from_zero() {
    for (thb, expected) in [
        ("-1.005", "-1.01"),
        ("-2.675", "-2.68"),
        ("-1.004", "-1.00"),
    ] {
        let line = completed_line(line_with(thb, "1", "1", Some(thb)));
        assert_eq!(line["thbAmount"], expected, "thbAmount of {thb}");
        assert_eq!(line["settleThb"], expected, "settleThb of {thb}");
        assert_eq!(
            line["realizedMarginThb"], expected,
            "realizedMarginThb of {thb}"
        );
    }
}

/// Row 3: quantity and unit size round half-up to four places.
#[test]
fn completed_quantity_and_unit_size_round_half_up_to_four_places() {
    for (raw, expected) in [
        ("0.00005", "0.0001"),
        ("0.00004", "0.0000"),
        ("1.23455", "1.2346"),
        ("9.99999", "10.0000"),
    ] {
        let line = completed_line(line_with("1", raw, raw, None));
        assert_eq!(line["quantity"], expected, "quantity of {raw}");
        assert_eq!(line["unitSize"], expected, "unitSize of {raw}");
    }
}

/// Rows 1–2 on the reversal: `amountThb` rounds the same way.
#[test]
fn reversed_amount_rounds_half_up_away_from_zero() {
    for (thb, expected) in [
        ("100.005", "100.01"),
        ("100.999", "101.00"),
        ("-100.005", "-100.01"),
        ("100.004", "100.00"),
    ] {
        let body = LoyaltyWireBody::reversed(
            uuid(EVENT_ID),
            &base_reversed_facts(dec(thb)),
            uuid(TENANT_ID),
            PHONE,
        );
        assert_eq!(body["data"]["amountThb"], expected, "amountThb of {thb}");
    }
}
