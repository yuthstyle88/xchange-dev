//! Ticket LY-1 (#1381) up-front tests — `LoyaltyCustomerRef` (design `membership-points.md` D-2) and
//! `LoyaltyWireBody` (design §4.5 `[required]`, D-2, D-6, §9.3 row LY-1). Pure, no database. Written before
//! the implementation by the ticket author (claude/29df2ec5). Do not edit, delete or weaken; add tests in a
//! file of your own.
//!
//! `NS_XC_LOYALTY` was chosen once by the test author on the lane controller's ruling on #1381
//! (claude/ad05bf8d, 2026-10-12) and is pinned here: changing it orphans every member reference.
//!
//! Red on `main` by result: the scaffold's `of` answers the nil uuid and the wire builders answer `null`.

use std::str::FromStr;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_kernel::Mode;
use xc_kernel::money::Side;
use xc_loyalty::domain::customer_ref::{LoyaltyCustomerRef, NS_XC_LOYALTY};
use xc_loyalty::domain::facts::{CompletedFacts, CompletedLine, ReversedFacts};
use xc_loyalty::domain::wire_body::LoyaltyWireBody;

/// The pinned namespace. Never change it.
const PINNED_NS: &str = "4d7a67a3-4dc9-48d1-8163-f21a31438408";

const ORG_A: &str = "00000000-0000-0000-0000-000000000001";
const ORG_B: &str = "00000000-0000-0000-0000-0000000000b2";
const TENANT: &str = "6f1c2d3e-0000-4000-8000-00000000a001";
const EVENT: &str = "6f1c2d3e-0000-4000-8000-00000000e001";

fn uuid(s: &str) -> Uuid {
    Uuid::parse_str(s).expect("a uuid literal")
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal literal")
}

fn completed() -> CompletedFacts {
    CompletedFacts {
        organization_id: uuid(ORG_A),
        transaction_id: uuid("6f1c2d3e-0000-4000-8000-0000000000c1"),
        document_ref: "XC-B01-20261001-0007".to_owned(),
        customer_id: Some("cust-42".to_owned()),
        mode: Mode::Live,
        branch_id: "XC-B01".to_owned(),
        completed_at: Utc.with_ymd_and_hms(2026, 10, 1, 3, 4, 5).unwrap(),
        business_date: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        reversible_until: Utc.with_ymd_and_hms(2026, 10, 1, 16, 59, 59).unwrap(),
        lines: vec![
            CompletedLine {
                currency: "USD".to_owned(),
                shop_side: Side::Buy,
                quantity: dec("100"),
                unit_size: dec("1"),
                thb_amount: dec("3400"),
                settle_thb: dec("3400.5"),
                realized_margin_thb: None,
            },
            CompletedLine {
                currency: "JPY".to_owned(),
                shop_side: Side::Sell,
                quantity: dec("10000.5"),
                unit_size: dec("100"),
                thb_amount: dec("2310.25"),
                settle_thb: dec("2310.25"),
                realized_margin_thb: Some(dec("12.3")),
            },
        ],
    }
}

fn reversed() -> ReversedFacts {
    ReversedFacts {
        organization_id: uuid(ORG_A),
        reversal_id: uuid("6f1c2d3e-0000-4000-8000-0000000000d1"),
        transaction_id: uuid("6f1c2d3e-0000-4000-8000-0000000000c1"),
        customer_id: Some("cust-42".to_owned()),
        posted_at: Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap(),
        amount_thb: dec("5710.75"),
        full: true,
    }
}

fn rfc3339(v: &Value) -> DateTime<Utc> {
    let s = v
        .as_str()
        .unwrap_or_else(|| panic!("an RFC 3339 string, got {v}"));
    DateTime::parse_from_rfc3339(s)
        .unwrap_or_else(|e| panic!("{s:?} is not RFC 3339: {e}"))
        .with_timezone(&Utc)
}

/// Every key anywhere in `v`, lowercased.
fn keys(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                out.push(k.to_lowercase());
                keys(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| keys(i, out)),
        _ => {}
    }
}

fn assert_no_other_personal_data(body: &Value) {
    let mut all = Vec::new();
    keys(body, &mut all);
    for k in &all {
        for word in ["passport", "name", "lookup", "nationality", "birth"] {
            assert!(!k.contains(word), "the wire body has a key {k:?}: {body}");
        }
    }
}

// ── LoyaltyCustomerRef ──────────────────────────────────────────────────────

#[test]
fn the_namespace_is_pinned_and_the_ref_is_uuid_v5_of_xchange_org_customer() {
    assert_eq!(
        NS_XC_LOYALTY,
        uuid(PINNED_NS),
        "NS_XC_LOYALTY must never change: every customerRef Loyalty holds derives from it"
    );
    let org = uuid(ORG_A);
    let expected = Uuid::new_v5(
        &uuid(PINNED_NS),
        format!("xchange:{ORG_A}:cust-42").as_bytes(),
    );
    let got = LoyaltyCustomerRef::of(org, "cust-42");
    assert_eq!(
        got, expected,
        "UUIDv5(NS, \"xchange:\" ‖ org ‖ \":\" ‖ customer)"
    );
    assert_eq!(got.get_version_num(), 5);
}

#[test]
fn the_ref_is_stable_and_differs_across_organisations_and_customers() {
    let a = uuid(ORG_A);
    let b = uuid(ORG_B);
    let first = LoyaltyCustomerRef::of(a, "cust-42");
    assert_eq!(
        first,
        LoyaltyCustomerRef::of(a, "cust-42"),
        "stable for one input"
    );
    assert_ne!(
        first,
        LoyaltyCustomerRef::of(b, "cust-42"),
        "another organisation"
    );
    assert_ne!(
        first,
        LoyaltyCustomerRef::of(a, "cust-43"),
        "another customer"
    );
    assert_ne!(first, Uuid::nil());
}

// ── LoyaltyWireBody ─────────────────────────────────────────────────────────

#[test]
fn the_completed_wire_body_is_the_designs_contract() {
    let facts = completed();
    let body = LoyaltyWireBody::completed(uuid(EVENT), &facts, uuid(TENANT), "+66812345678");

    assert_eq!(body["id"], json!(EVENT));
    assert_eq!(body["type"], json!("xchange.exchange_completed.v1"));
    let data = &body["data"];
    assert_eq!(data["tenantId"], json!(TENANT));
    assert_eq!(data["phone"], json!("+66812345678"));
    let expected_ref = Uuid::new_v5(
        &uuid(PINNED_NS),
        format!("xchange:{ORG_A}:cust-42").as_bytes(),
    );
    assert_eq!(data["customerRef"], json!(expected_ref.to_string()));
    assert_eq!(
        data["transactionId"],
        json!(facts.transaction_id.to_string())
    );
    assert_eq!(data["documentRef"], json!("XC-B01-20261001-0007"));
    assert_eq!(data["branchId"], json!("XC-B01"));
    assert_eq!(rfc3339(&data["completedAt"]), facts.completed_at);
    assert_eq!(data["businessDate"], json!("2026-10-01"));
    assert_eq!(rfc3339(&data["reversibleUntil"]), facts.reversible_until);

    assert_eq!(
        data["lines"],
        json!([
            { "currency": "USD", "shopSide": "BUY", "quantity": "100.0000", "unitSize": "1.0000",
              "thbAmount": "3400.00", "settleThb": "3400.50", "realizedMarginThb": null },
            { "currency": "JPY", "shopSide": "SELL", "quantity": "10000.5000", "unitSize": "100.0000",
              "thbAmount": "2310.25", "settleThb": "2310.25", "realizedMarginThb": "12.30" }
        ]),
        "money is a decimal string: quantity 4 places, THB 2 places, margin null until #553"
    );
    let mut data_keys: Vec<&str> = data
        .as_object()
        .expect("data is an object")
        .keys()
        .map(String::as_str)
        .collect();
    data_keys.sort_unstable();
    assert_eq!(
        data_keys,
        vec![
            "branchId",
            "businessDate",
            "completedAt",
            "customerRef",
            "documentRef",
            "lines",
            "phone",
            "reversibleUntil",
            "tenantId",
            "transactionId"
        ],
        "exactly the design §4.5 fields"
    );
    assert_no_other_personal_data(&body);
}

#[test]
fn the_reversed_wire_body_is_the_designs_contract() {
    let facts = reversed();
    let body = LoyaltyWireBody::reversed(uuid(EVENT), &facts, uuid(TENANT), "+66812345678");

    assert_eq!(body["id"], json!(EVENT));
    assert_eq!(body["type"], json!("xchange.exchange_reversed.v1"));
    let data = &body["data"];
    let expected_ref = Uuid::new_v5(
        &uuid(PINNED_NS),
        format!("xchange:{ORG_A}:cust-42").as_bytes(),
    );
    assert_eq!(data["tenantId"], json!(TENANT));
    assert_eq!(data["phone"], json!("+66812345678"));
    assert_eq!(data["customerRef"], json!(expected_ref.to_string()));
    assert_eq!(
        data["transactionId"],
        json!(facts.transaction_id.to_string()),
        "the original transaction"
    );
    assert_eq!(data["reversalId"], json!(facts.reversal_id.to_string()));
    assert_eq!(rfc3339(&data["postedAt"]), facts.posted_at);
    assert_eq!(data["amountThb"], json!("5710.75"));
    assert_eq!(data["full"], json!(true));
    let mut data_keys: Vec<&str> = data
        .as_object()
        .expect("data is an object")
        .keys()
        .map(String::as_str)
        .collect();
    data_keys.sort_unstable();
    assert_eq!(
        data_keys,
        vec![
            "amountThb",
            "customerRef",
            "full",
            "phone",
            "postedAt",
            "reversalId",
            "tenantId",
            "transactionId"
        ],
        "exactly the design §4.5 fields"
    );
    assert_no_other_personal_data(&body);
}

#[test]
fn a_foreign_phone_is_sent_as_its_e164_value() {
    // Most FX customers are tourists (D-2): a non-Thai number must not be rewritten as +66.
    let body = LoyaltyWireBody::completed(uuid(EVENT), &completed(), uuid(TENANT), "+14155550123");
    assert_eq!(body["data"]["phone"], json!("+14155550123"));
    let body = LoyaltyWireBody::reversed(uuid(EVENT), &reversed(), uuid(TENANT), "+14155550123");
    assert_eq!(body["data"]["phone"], json!("+14155550123"));
}
