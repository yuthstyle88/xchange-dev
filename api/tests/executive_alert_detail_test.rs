//! OL-7 row 64 (verification of 2026-11-07): A01's "ต้องดำเนินการ" items printed `detail`, a sentence the server
//! wrote in English with record ids in it ("RATE_PUBLICATION <uuid>", "COUNT_DIFFERENCE <uuid>: USD -3.00", an
//! exception row's id, "USD: available … floor …"). The alert now carries figures and codes only, and the screen
//! words them in its own language:
//! - `ratePending`, and an exception row the reader cannot decode: nothing beyond kind, branch and time;
//! - `staleData`: `staleMinutes`, the branch's threshold;
//! - `countDifference`: `differences`, each currency and its signed amount (a decimal string);
//! - `lowStock`: `stock`, the currency and its available / on-hand / reserved / floor quantities (decimal strings).
//! There is no `detail` key on the wire. No database: the DTO is built from the application's `Overview`.
//!
//! Written before the fix by claude/bbe9ce8b for the OL sub-controller claude/1cc51615. Do not edit, delete or
//! weaken; add tests in a file of your own.

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use xc_kernel::ids::BranchId;
use xchange::application::executive::drilldown::Drilldown;
use xchange::application::executive::overview::Overview;
use xchange::application::executive::ports::{Alert, AlertDetail, AlertKind, ExecutiveFilter};
use xchange::presentation::dto::v3::executive_overview::ExecutiveOverviewData;

const UUID: &str = "6f1c2a9e-1b2c-4d3e-8f90-123456789abc";

fn dec(s: &str) -> Decimal {
    s.parse().expect("a decimal")
}

fn alert(kind: AlertKind, detail: AlertDetail) -> Alert {
    Alert {
        kind,
        branch: BranchId::new("XC-B01".to_owned()),
        since: Some(Utc.with_ymd_and_hms(2026, 11, 7, 3, 0, 0).unwrap()),
        detail,
        drilldown: Some(Drilldown::to(
            "approval_requests",
            vec![("branch", "XC-B01".to_owned())],
        )),
    }
}

/// The wire form of one alert, as `GET /api/executive/overview` sends it in `data.alerts`.
fn wire(a: Alert) -> Value {
    let overview = Overview {
        kpis: vec![],
        branches: vec![],
        alerts: vec![a],
        definitions: &[],
        buy_reversed_thb: Decimal::ZERO,
        sell_reversed_thb: Decimal::ZERO,
    };
    let data = ExecutiveOverviewData::of(&ExecutiveFilter::default(), &overview);
    let body = serde_json::to_value(&data).expect("serialize");
    body["alerts"][0].clone()
}

#[test]
fn ed_1_rate_pending_carries_no_detail_and_no_figures() {
    let a = wire(alert(AlertKind::RatePending, AlertDetail::None));
    assert_eq!(a["kind"], "ratePending");
    assert!(a.get("detail").is_none(), "no free-text detail: {a}");
    assert_eq!(a["staleMinutes"], Value::Null, "{a}");
    assert_eq!(a["differences"], json!([]), "{a}");
    assert_eq!(a["stock"], Value::Null, "{a}");
}

#[test]
fn ed_2_stale_data_carries_its_minutes_as_a_number() {
    let a = wire(alert(AlertKind::StaleData, AlertDetail::StaleMinutes(45)));
    assert_eq!(a["kind"], "staleData");
    assert_eq!(a["staleMinutes"], json!(45), "{a}");
    assert!(a.get("detail").is_none(), "{a}");
}

#[test]
fn ed_3_count_difference_carries_each_currency_and_its_signed_amount() {
    let a = wire(alert(
        AlertKind::CountDifference,
        AlertDetail::Differences(vec![
            ("THB".to_owned(), dec("12.50")),
            ("USD".to_owned(), dec("-3.00")),
        ]),
    ));
    assert_eq!(a["kind"], "countDifference");
    assert_eq!(
        a["differences"],
        json!([
            { "currency": "THB", "amount": "12.50" },
            { "currency": "USD", "amount": "-3.00" },
        ]),
        "{a}"
    );
    assert_eq!(a["stock"], Value::Null, "{a}");
}

#[test]
fn ed_4_an_undecodable_count_difference_row_is_an_empty_list() {
    let a = wire(alert(AlertKind::CountDifference, AlertDetail::None));
    assert_eq!(a["differences"], json!([]), "{a}");
    assert!(a.get("detail").is_none(), "{a}");
}

#[test]
fn ed_5_low_stock_carries_its_currency_and_four_quantities() {
    let a = wire(alert(
        AlertKind::LowStock,
        AlertDetail::Stock {
            currency: "USD".to_owned(),
            available: dec("40.0000"),
            on_hand: dec("80.0000"),
            reserved: dec("40.0000"),
            floor: dec("50.0000"),
        },
    ));
    assert_eq!(a["kind"], "lowStock");
    assert_eq!(
        a["stock"],
        json!({
            "currency": "USD",
            "available": "40.0000",
            "onHand": "80.0000",
            "reserved": "40.0000",
            "floor": "50.0000",
        }),
        "{a}"
    );
    assert_eq!(a["differences"], json!([]), "{a}");
}

#[test]
fn ed_6_no_alert_shape_can_carry_a_record_id() {
    // The wire has no free-text field: every value is a kind, a branch id, a time, a number, a currency code or a
    // decimal string. A record id (`UUID`) has nowhere to go.
    for detail in [
        AlertDetail::None,
        AlertDetail::StaleMinutes(1),
        AlertDetail::Differences(vec![("THB".to_owned(), dec("1.00"))]),
    ] {
        let a = wire(alert(AlertKind::CountDifference, detail));
        let mut keys: Vec<&str> = a
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "branchId",
                "differences",
                "drilldown",
                "kind",
                "since",
                "staleMinutes",
                "stock"
            ],
            "{a}"
        );
        assert!(!a.to_string().contains(UUID));
    }
}
