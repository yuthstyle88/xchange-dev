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
