//! LA-2 up-front tests (design `docs/design/blueprint/ledger-inventory.md` §13.4, §13.9 rows LA-T5 and LA-T6):
//! the adjustment payload's codec, its validation, and the posting requests it builds.
//!
//! 🔴 Written by the sub-controller claude/f32b85db before the implementation. Do not edit, delete or weaken;
//! add tests in a file of your own. No database.

use std::collections::HashMap;
use std::str::FromStr;

use rust_decimal::Decimal;
use serde_json::{Value, json};
use xc_cash::domain::adjustment::{
    AdjustmentKind, CostLine, Evidence, LedgerAdjustment, QuantityLine,
};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::CashPlace;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, MinorUnit, MoneyDelta};

const ID: &str = "ADJ-00000000-0000-4000-8000-000000000001";

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn cur(code: &str) -> CurrencyCode {
    CurrencyCode::parse(code).unwrap()
}

/// THB 2, USD 2, EUR 2, JPY 0, and 21 more two-place codes for the 21-line case.
fn minor_units() -> HashMap<CurrencyCode, MinorUnit> {
    let mut m = HashMap::new();
    m.insert(cur("JPY"), MinorUnit::ZERO);
    for code in [
        "THB", "USD", "EUR", "GBP", "AUD", "CAD", "CHF", "CNY", "HKD", "SGD", "MYR", "NZD", "SEK",
        "NOK", "DKK", "INR", "PHP", "TWD", "AED", "SAR", "QAR", "ZAR", "MXN", "BRL",
    ] {
        m.insert(cur(code), MinorUnit::TWO);
    }
    m
}

fn evidence() -> Evidence {
    Evidence {
        files: vec!["FILE-B01-1".to_owned()],
        reference: "slip 004512".to_owned(),
    }
}

fn quantity(lines: &[(&str, &str)]) -> LedgerAdjustment {
    LedgerAdjustment {
        id: ID.to_owned(),
        branch: BranchId::new("B01"),
        kind: AdjustmentKind::Quantity {
            place: "SAFE".to_owned(),
            lines: lines
                .iter()
                .map(|(c, q)| QuantityLine {
                    currency: cur(c),
                    quantity: d(q),
                })
                .collect(),
        },
        reason: "Bank deposit keyed as 1,100 USD; the slip says 1,000".to_owned(),
        evidence: evidence(),
        exception_id: Some("EX-1".to_owned()),
    }
}

fn cost(lines: &[(&str, &str)]) -> LedgerAdjustment {
    LedgerAdjustment {
        id: ID.to_owned(),
        branch: BranchId::new("B01"),
        kind: AdjustmentKind::Cost {
            lines: lines
                .iter()
                .map(|(c, x)| CostLine {
                    currency: cur(c),
                    cost_delta_thb: d(x),
                })
                .collect(),
        },
        reason: "Opening cost was the BUY rate; the purchase invoice says 33.00".to_owned(),
        evidence: Evidence {
            files: vec![],
            reference: "invoice 77".to_owned(),
        },
        exception_id: None,
    }
}

fn source() -> SourceRef {
    SourceRef {
        source_type: SourceType::Adjustment,
        source_id: ID.to_owned(),
        source_key: "post".to_owned(),
    }
}

// ── LA-T5 ───────────────────────────────────────────────────────────────────

#[test]
fn la_t5_a_quantity_adjustment_round_trips_through_its_payload() {
    let a = quantity(&[("USD", "-100"), ("THB", "250.00")]);
    let payload = a.to_payload();
    assert_eq!(payload["branch"], "B01", "{payload}");
    assert_eq!(payload["kind"], "QUANTITY", "{payload}");
    assert_eq!(payload["place"], "SAFE", "{payload}");
    assert!(
        payload["lines"][0]["quantity"].is_string(),
        "amounts are decimal strings (§13.4): {payload}"
    );
    assert_eq!(
        LedgerAdjustment::from_payload(ID, &payload).expect("its own payload reads back"),
        a
    );
}

#[test]
fn la_t5_a_cost_adjustment_round_trips_through_its_payload() {
    let a = cost(&[("USD", "450.00"), ("EUR", "-12.50")]);
    let payload = a.to_payload();
    assert_eq!(payload["kind"], "COST", "{payload}");
    assert!(
        payload.get("place").is_none_or(Value::is_null),
        "place is QUANTITY's only: {payload}"
    );
    assert!(
        payload["lines"][0]["costDeltaThb"].is_string(),
        "amounts are decimal strings (§13.4): {payload}"
    );
    assert_eq!(
        LedgerAdjustment::from_payload(ID, &payload).expect("its own payload reads back"),
        a
    );
}

#[test]
fn la_t5_the_design_example_payload_reads() {
    let payload = json!({
        "branch": "B01",
        "kind": "QUANTITY",
        "place": "SAFE",
        "lines": [ { "currency": "USD", "quantity": "-100" }, { "currency": "THB", "quantity": "250.00" } ],
        "reason": "Bank deposit of 2026-10-26 keyed as 1,100 USD; the bank slip says 1,000",
        "evidence": { "files": ["FILE-B01-1"], "reference": "slip 004512" },
        "exceptionId": "EX-1"
    });
    let a = LedgerAdjustment::from_payload(ID, &payload).expect("the §13.4 example reads");
    assert_eq!(a.id, ID);
    assert_eq!(a.branch, BranchId::new("B01"));
    assert_eq!(a.exception_id.as_deref(), Some("EX-1"));
    assert_eq!(a.evidence.files, vec!["FILE-B01-1".to_owned()]);
    match &a.kind {
        AdjustmentKind::Quantity { place, lines } => {
            assert_eq!(place, "SAFE");
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0].currency, cur("USD"));
            assert_eq!(lines[0].quantity, d("-100"));
        }
        other => panic!("a QUANTITY adjustment: {other:?}"),
    }
    assert!(a.validate(&minor_units()).is_ok());
}

#[test]
fn la_t5_quantity_requests_move_a_shortfall_to_variance_and_an_overage_back_to_the_safe() {
    let a = quantity(&[("USD", "-100"), ("THB", "250")]);
    let reqs = a.quantity_requests();
    assert_eq!(reqs.len(), 2, "{reqs:?}");
    let b01 = BranchId::new("B01");
    let usd = reqs
        .iter()
        .find(|r| r.currency == cur("USD"))
        .expect("a USD leg");
    assert_eq!(usd.from, CashPlace::Safe(b01.clone()));
    assert_eq!(usd.to, CashPlace::Variance(b01.clone()));
    assert_eq!(usd.quantity.value(), d("100"));
    let thb = reqs
        .iter()
        .find(|r| r.currency == cur("THB"))
        .expect("a THB leg");
    assert_eq!(thb.from, CashPlace::Variance(b01.clone()));
    assert_eq!(thb.to, CashPlace::Safe(b01));
    assert_eq!(thb.quantity.value(), d("250"));
    for r in &reqs {
        assert_eq!(r.source, source(), "every leg under (ADJUSTMENT, id, post)");
        assert_eq!(r.line, None);
        assert_eq!(r.reverses, None);
    }
    assert!(a.revalue_requests().is_empty(), "QUANTITY revalues nothing");
}

#[test]
fn la_t5_revalue_requests_are_one_per_cost_line_in_line_order() {
    let a = cost(&[("USD", "450.00"), ("EUR", "-12.50")]);
    let reqs = a.revalue_requests();
    assert_eq!(reqs.len(), 2, "{reqs:?}");
    assert_eq!(reqs[0].currency, cur("USD"));
    assert_eq!(reqs[0].cost_delta_thb, MoneyDelta::from_minor(45_000));
    assert_eq!(reqs[1].currency, cur("EUR"));
    assert_eq!(reqs[1].cost_delta_thb, MoneyDelta::from_minor(-1_250));
    for r in &reqs {
        assert_eq!(r.branch, BranchId::new("B01"));
        assert_eq!(r.source, source());
    }
    assert!(a.quantity_requests().is_empty(), "COST moves no quantity");
}

// ── LA-T6 ───────────────────────────────────────────────────────────────────

fn refused_on(a: &LedgerAdjustment, field: &str, what: &str) {
    let e = a
        .validate(&minor_units())
        .expect_err(&format!("{what} is refused"));
    assert_eq!(e.field, field, "{what}: {e:?}");
    assert!(!e.message.is_empty(), "{what}: a message");
}

#[test]
fn la_t6_lines_count_and_repeats_are_refused() {
    refused_on(&quantity(&[]), "lines", "no line");
    let codes = [
        "USD", "EUR", "GBP", "AUD", "CAD", "CHF", "CNY", "HKD", "SGD", "MYR", "NZD", "SEK", "NOK",
        "DKK", "INR", "PHP", "TWD", "AED", "SAR", "QAR", "ZAR",
    ];
    assert_eq!(codes.len(), 21);
    let lines: Vec<(&str, &str)> = codes.iter().map(|c| (*c, "1")).collect();
    refused_on(&quantity(&lines), "lines", "21 lines");
    let twenty: Vec<(&str, &str)> = codes[..20].iter().map(|c| (*c, "1")).collect();
    assert!(
        quantity(&twenty).validate(&minor_units()).is_ok(),
        "20 lines are allowed"
    );
    refused_on(
        &quantity(&[("USD", "1"), ("USD", "2")]),
        "lines[1].currency",
        "a repeated currency",
    );
}

#[test]
fn la_t6_quantities_are_refused_by_value_and_places() {
    refused_on(
        &quantity(&[("USD", "0")]),
        "lines[0].quantity",
        "quantity 0",
    );
    refused_on(
        &quantity(&[("USD", "0.00001")]),
        "lines[0].quantity",
        "USD 0.00001",
    );
    refused_on(
        &quantity(&[("USD", "1"), ("JPY", "10.5")]),
        "lines[1].quantity",
        "JPY with a fraction beyond its minor unit",
    );
    refused_on(
        &quantity(&[("USD", "1.001")]),
        "lines[0].quantity",
        "USD beyond its two places",
    );
    assert!(
        quantity(&[("JPY", "-1000"), ("USD", "-0.01")])
            .validate(&minor_units())
            .is_ok(),
        "within each minor unit"
    );
}

#[test]
fn la_t6_a_currency_without_a_known_minor_unit_is_refused() {
    refused_on(
        &quantity(&[("USD", "1"), ("KRW", "100")]),
        "lines[1].currency",
        "a currency the register does not know",
    );
}

#[test]
fn la_t6_cost_lines_are_refused_on_thb_and_zero() {
    refused_on(&cost(&[("THB", "10.00")]), "lines[0].currency", "COST on THB");
    refused_on(
        &cost(&[("USD", "0.00")]),
        "lines[0].costDeltaThb",
        "COST delta 0",
    );
    refused_on(
        &cost(&[("USD", "1.005")]),
        "lines[0].costDeltaThb",
        "COST delta beyond two places",
    );
}

#[test]
fn la_t6_only_the_safe_may_be_adjusted() {
    let mut a = quantity(&[("USD", "-1")]);
    if let AdjustmentKind::Quantity { place, .. } = &mut a.kind {
        *place = "DRAWER".to_owned();
    }
    refused_on(&a, "place", "place DRAWER");
}

#[test]
fn la_t6_reason_and_evidence_are_required() {
    let mut a = quantity(&[("USD", "-1")]);
    a.reason = "   ".to_owned();
    refused_on(&a, "reason", "a blank reason");

    let mut a = quantity(&[("USD", "-1")]);
    a.reason = "x".repeat(501);
    refused_on(&a, "reason", "a 501-character reason");
    a.reason = "x".repeat(500);
    assert!(a.validate(&minor_units()).is_ok(), "500 characters are allowed");

    let mut a = quantity(&[("USD", "-1")]);
    a.evidence = Evidence {
        files: vec![],
        reference: "  ".to_owned(),
    };
    refused_on(&a, "evidence", "no file and a blank reference");

    let mut a = quantity(&[("USD", "-1")]);
    a.evidence = Evidence {
        files: vec![],
        reference: "slip 1".to_owned(),
    };
    assert!(a.validate(&minor_units()).is_ok(), "a reference alone is evidence");

    let mut a = quantity(&[("USD", "-1")]);
    a.evidence.files = (0..11).map(|i| format!("F{i}")).collect();
    refused_on(&a, "evidence.files", "11 files");

    let mut a = quantity(&[("USD", "-1")]);
    a.evidence.reference = "r".repeat(201);
    refused_on(&a, "evidence.reference", "a 201-character reference");
}

#[test]
fn la_t6_an_unreadable_payload_is_refused_naming_its_field() {
    let good = quantity(&[("USD", "-100")]).to_payload();
    for (key, value, field) in [
        ("kind", json!("SWAP"), "kind"),
        ("lines", json!("USD -100"), "lines"),
        ("reason", json!(7), "reason"),
        ("branch", json!(null), "branch"),
    ] {
        let mut p = good.clone();
        p[key] = value;
        let e = LedgerAdjustment::from_payload(ID, &p).expect_err(&format!("{key} unreadable"));
        assert_eq!(e.field, field, "{key}: {e:?}");
    }
    let mut p = good.clone();
    p["lines"][0]["quantity"] = json!("a lot");
    let e = LedgerAdjustment::from_payload(ID, &p).expect_err("a non-decimal quantity");
    assert_eq!(e.field, "lines[0].quantity", "{e:?}");
    let mut p = good;
    p["lines"][0]["currency"] = json!("XXX1");
    let e = LedgerAdjustment::from_payload(ID, &p).expect_err("an unknown currency code");
    assert_eq!(e.field, "lines[0].currency", "{e:?}");
}
