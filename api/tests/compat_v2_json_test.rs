//! T11n — v3's domain types printed as the JSON v2 printed.
//!
//! No database. What is expected comes from the recorded v2 answers under
//! `tests/fixtures/snapshot/` (`empty`, `rates`, `shift`, `branches`,
//! `custody`): the values through [`Step::json`], and v2's own text through
//! [`Fixture::body_text`] wherever key order is part of what a page shows.
//!
//! Known differences from v2, decided before this ticket and asserted here as
//! decided: `row`, `startRow` and `endRow` of a shift are left out.

mod compat_common;

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use compat_common::Fixture;
use serde_json::{Map, Value, json};
use xc_customer::domain::customer::entities::customer::Customer;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta, Rate, Side};
use xc_rate::domain::branch::entities::branch::Branch;
use xc_rate::domain::rate::entities::counter_rate::CounterRate;
use xc_rate::domain::rate::entities::rate_change::RateChange;
use xc_shift::domain::custody::entities::attendance::Attendance;
use xc_shift::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use xc_shift::domain::custody::entities::custody_request::{CustodyRequest, Decision};
use xc_shift::domain::custody::entities::customer_hold::{CustomerHold, HoldStatus};
use xc_shift::domain::custody::entities::stored_event::StoredEvent;
use xc_shift::domain::custody::entities::tx_draft::{DraftStatus, TxDraft};
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::domain::custody::value_objects::cash_target::CashTarget;
use xc_shift::domain::custody::value_objects::count_sheet::CountSheet;
use xc_shift::domain::custody::value_objects::custody_event::EventKind;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::cutoff::Cutoff;
use xc_shift::domain::custody::value_objects::denomination_row::DenominationRow;
use xc_shift::domain::custody::value_objects::entity::Entity;
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::domain::custody::value_objects::request_kind::RequestKind;
use xc_shift::domain::custody::value_objects::request_status::{
    Reconciliation, RequestStatus, TransferStage,
};
use xc_shift::domain::custody::value_objects::signature::Signature;
use xc_shift::domain::shift::entities::shift::{Shift, ShiftStatus};
use xc_shift::domain::shift::entities::stock_movement::StockMovement;
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::domain::shift::value_objects::stock_delta::{Direction, StockDelta};
use xchange::application::compat::v2_json::{
    People, PersonLabel, V2Json, balances_json, branches_json, custody_time_json, customers_json,
    delta_json, money_json, movements_json, operations_json, rate_history_json, rate_json,
    rates_json, shifts_json, stock_delta_json, time_json, time_or_empty_json,
};

// ── helpers: building domain values ──────────────────────────────────────────

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).unwrap_or_else(|_| panic!("{text} is not a register code"))
}

fn money(text: &str) -> Money {
    Money::parse(text).unwrap_or_else(|e| panic!("{text} is not money: {e}"))
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).unwrap_or_else(|e| panic!("{text} is not a rate: {e}"))
}

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("{text} is not a time: {e}"))
        .with_timezone(&Utc)
}

fn custody_at(text: &str) -> CustodyTime {
    CustodyTime::from_epoch_millis(at(text).timestamp_millis())
}

fn balances(pairs: &[(&str, &str)]) -> Balances {
    let pairs: Vec<(CurrencyCode, Money)> =
        pairs.iter().map(|(c, m)| (code(c), money(m))).collect();
    Balances::of(&pairs)
}

fn delta(pairs: &[(&str, i128)]) -> StockDelta {
    let pairs: Vec<(CurrencyCode, MoneyDelta)> = pairs
        .iter()
        .map(|(c, minor)| (code(c), MoneyDelta::from_minor(*minor)))
        .collect();
    StockDelta::of(&pairs)
}

// ── helpers: reading a recorded answer ───────────────────────────────────────

fn text_of(value: &Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("`{key}` is not text in {value}"))
        .to_string()
}

fn optional_text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// A recorded JSON number as hundredths.
fn minor_of(number: &Value) -> i128 {
    let n = number
        .as_f64()
        .unwrap_or_else(|| panic!("{number} is not a number"));
    (n * 100.0).round() as i128
}

fn money_of(number: &Value) -> Money {
    let minor = i64::try_from(minor_of(number)).expect("a recorded amount fits");
    Money::from_minor(minor).unwrap_or_else(|e| panic!("{number}: {e}"))
}

fn rate_of(number: &Value) -> Rate {
    rate(&number.to_string())
}

/// A recorded 153-code object as `Balances` (what is zero is not kept).
fn balances_of(object: &Value) -> Balances {
    let map = object
        .as_object()
        .unwrap_or_else(|| panic!("{object} is not an object"));
    assert_eq!(map.len(), 153, "v2 prints every register code");
    let pairs: Vec<(CurrencyCode, Money)> =
        map.iter().map(|(c, n)| (code(c), money_of(n))).collect();
    Balances::of(&pairs)
}

fn optional_balances(value: &Value, key: &str) -> Option<Balances> {
    value.get(key).filter(|v| !v.is_null()).map(balances_of)
}

fn delta_of(object: &Value) -> StockDelta {
    let map = object
        .as_object()
        .unwrap_or_else(|| panic!("{object} is not an object"));
    assert_eq!(map.len(), 153, "v2 prints every register code");
    let pairs: Vec<(CurrencyCode, MoneyDelta)> = map
        .iter()
        .map(|(c, n)| (code(c), MoneyDelta::from_minor(minor_of(n))))
        .collect();
    StockDelta::of(&pairs)
}

fn optional_delta(value: &Value, key: &str) -> Option<StockDelta> {
    value.get(key).filter(|v| !v.is_null()).map(delta_of)
}

fn optional_custody_time(value: &Value, key: &str) -> Option<CustodyTime> {
    value.get(key).and_then(Value::as_str).map(custody_at)
}

fn array<'a>(value: &'a Value, key: &str) -> &'a Vec<Value> {
    value[key]
        .as_array()
        .unwrap_or_else(|| panic!("`{key}` is not an array in {value}"))
}

/// The keys of a JSON object in the order its text has them. `text` must be an
/// object whose text was produced without spaces.
fn keys_in_text_order(text: &str) -> Vec<String> {
    assert!(text.starts_with('{') && text.ends_with('}'), "{text}");
    let bytes = text.as_bytes();
    let mut keys = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    let mut expect_key = false;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let start = i;
                i += 1;
                while bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                if depth == 1 && expect_key {
                    let key: String = serde_json::from_str(&text[start..=i]).expect("a key");
                    keys.push(key);
                    expect_key = false;
                }
            }
            b'{' | b'[' => {
                depth += 1;
                if depth == 1 {
                    expect_key = true;
                }
            }
            b'}' | b']' => depth -= 1,
            b',' if depth == 1 => expect_key = true,
            _ => {}
        }
        i += 1;
    }
    keys
}

/// The pairs of a printed object; panics when it is not one.
fn pairs(printed: &V2Json) -> &[(String, V2Json)] {
    match printed {
        V2Json::Object(pairs) => pairs,
        other => panic!("not an object: {other:?}"),
    }
}

fn items(printed: &V2Json) -> &[V2Json] {
    match printed {
        V2Json::Array(items) => items,
        other => panic!("not an array: {other:?}"),
    }
}

fn member<'a>(printed: &'a V2Json, key: &str) -> &'a V2Json {
    pairs(printed)
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
        .unwrap_or_else(|| panic!("no `{key}` in {printed:?}"))
}

fn keys(printed: &V2Json) -> Vec<&str> {
    pairs(printed).iter().map(|(k, _)| k.as_str()).collect()
}

/// A recorded value without the three keys v3 leaves out by decision.
fn without_row_keys(value: &Value) -> Value {
    match value {
        Value::Array(list) => Value::Array(list.iter().map(without_row_keys).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "row" | "startRow" | "endRow"))
                .map(|(k, v)| (k.clone(), without_row_keys(v)))
                .collect::<Map<String, Value>>(),
        ),
        other => other.clone(),
    }
}

fn main_branch() -> BranchId {
    BranchId::new("MAIN")
}

// ── numbers ──────────────────────────────────────────────────────────────────

#[test]
fn money_prints_as_v8_prints_a_number() {
    let cases = [
        ("100.00", "100"),
        ("100", "100"),
        ("1000.50", "1000.5"),
        ("0.01", "0.01"),
        ("0.10", "0.1"),
        ("0", "0"),
        ("10", "10"),
        ("1200", "1200"),
        ("100000000", "100000000"),
        ("99999999.99", "99999999.99"),
    ];
    for (typed, printed) in cases {
        assert_eq!(money_json(money(typed)).to_text(), printed, "{typed}");
    }
    // a principal the system computed, above what a person may type
    let principal = Money::from_minor(320_999_999_968).expect("non-negative");
    assert_eq!(money_json(principal).to_text(), "3209999999.68");
    assert_eq!(money_json(principal).to_value(), json!(3209999999.68));
}

#[test]
fn a_whole_amount_is_an_integer_never_a_float_and_never_a_string() {
    let whole = money_json(money("100.00"));
    assert_ne!(whole.to_text(), "100.0");
    assert_ne!(whole.to_text(), "\"100\"");
    let value = whole.to_value();
    assert!(
        value.is_u64() || value.is_i64(),
        "{value:?} is not an integer"
    );
    assert_eq!(value, json!(100));
    assert_ne!(value, json!(100.0), "serde_json tells 100 from 100.0");

    let half = money_json(money("1000.50")).to_value();
    assert!(half.is_f64(), "{half:?}");
    assert_eq!(half, json!(1000.5));
    assert!(!money_json(money("0.01")).to_value().is_string());
}

#[test]
fn rate_prints_the_shortest_decimal() {
    let cases = [
        ("0.2105", "0.2105"),
        ("32.1", "32.1"),
        ("32.1000", "32.1"),
        ("32.15", "32.15"),
        ("32", "32"),
        ("0.0001", "0.0001"),
        ("0.2100", "0.21"),
        ("40", "40"),
        ("100000000", "100000000"),
    ];
    for (typed, printed) in cases {
        assert_eq!(rate_json(rate(typed)).to_text(), printed, "{typed}");
    }
    assert_eq!(rate_json(rate("0.2105")).to_value(), json!(0.2105));
    assert_eq!(rate_json(rate("32")).to_value(), json!(32));
}

#[test]
fn delta_prints_its_sign_and_the_shortest_decimal() {
    let cases: [(i128, &str); 9] = [
        (-50, "-0.5"),
        (-10_000, "-100"),
        (100, "1"),
        (0, "0"),
        (1, "0.01"),
        (-1, "-0.01"),
        (-1_000, "-10"),
        (150_050, "1500.5"),
        (-320_999_999_968, "-3209999999.68"),
    ];
    for (minor, printed) in cases {
        let json = delta_json(MoneyDelta::from_minor(minor));
        assert_eq!(json.to_text(), printed, "{minor}");
    }
    assert_eq!(
        delta_json(MoneyDelta::from_minor(-50)).to_value(),
        json!(-0.5)
    );
    assert_eq!(
        delta_json(MoneyDelta::from_minor(-10_000)).to_value(),
        json!(-100)
    );
}

// ── the printed tree ─────────────────────────────────────────────────────────

#[test]
fn text_is_escaped_as_json_stringify_escapes_it() {
    let samples = [
        "plain",
        "",
        "say \"hi\"",
        "back\\slash",
        "line\nbreak\ttab\rreturn",
        "bell\u{7}unit\u{1f}null\u{0}",
        "form\u{c}feed back\u{8}space",
        "ดอลลาร์สหรัฐ €£¥",
        "a/b <script>",
        "sep\u{2028}arator\u{2029}",
        "=Partner starting with an equals sign",
    ];
    for sample in samples {
        let expected = serde_json::to_string(sample).expect("text");
        assert_eq!(V2Json::text(sample).to_text(), expected, "{sample:?}");
        assert_eq!(V2Json::text(sample).to_value(), json!(sample));
    }
    assert_eq!(
        V2Json::text("a\"b\\c\n\u{1}").to_text(),
        r#""a\"b\\c\n\u0001""#
    );
}

#[test]
fn an_object_is_printed_in_the_order_it_was_built() {
    let printed = V2Json::object(vec![
        ("zeta", V2Json::int(1)),
        ("alpha", V2Json::Null),
        ("mid", V2Json::Bool(true)),
        (
            "list",
            V2Json::Array(vec![
                V2Json::Bool(false),
                V2Json::text("x"),
                V2Json::int(-7),
            ]),
        ),
        ("empty", V2Json::object(vec![])),
        ("none", V2Json::Array(vec![])),
    ]);
    assert_eq!(
        printed.to_text(),
        r#"{"zeta":1,"alpha":null,"mid":true,"list":[false,"x",-7],"empty":{},"none":[]}"#
    );
    assert_eq!(
        printed.to_value(),
        json!({"zeta": 1, "alpha": null, "mid": true, "list": [false, "x", -7], "empty": {}, "none": []})
    );
}

#[test]
fn a_stored_value_is_printed_with_whole_numbers_as_integers() {
    let stored = json!({
        "unit": 100,
        "whole": 37.0,
        "selling": 0.219874,
        "original": {"selling": "21.9874"},
        "rows": [1, 2.5, null, true, "t"],
        "negative": -3.0,
    });
    let printed = V2Json::from_value(&stored);
    assert_eq!(
        printed.to_text(),
        r#"{"negative":-3,"original":{"selling":"21.9874"},"rows":[1,2.5,null,true,"t"],"selling":0.219874,"unit":100,"whole":37}"#
    );
    let back = printed.to_value();
    assert_eq!(back["whole"], json!(37));
    assert_eq!(back["selling"], json!(0.219874));
    assert_eq!(back["original"], stored["original"]);
}

// ── balances ─────────────────────────────────────────────────────────────────

#[test]
fn balances_reproduce_the_text_of_the_opening_of_shift_json() {
    let fixture = Fixture::load("snapshot/shift.json");
    let recorded = fixture.body_text(3).expect("shift.json#3 is JSON");
    let printed = balances_json(&balances(&[("THB", "50000"), ("USD", "1000.50")]));

    let text = printed.to_text();
    assert!(
        recorded.contains(&format!("\"opening\":{text},\"closedAt\"")),
        "v2 printed another text than {text}"
    );
    assert!(text.starts_with(
        r#"{"THB":50000,"USD":1000.5,"EUR":0,"GBP":0,"JPY":0,"CNY":0,"AUD":0,"AED":0,"#
    ));
    assert!(text.ends_with(r#""ZAR":0,"ZMW":0,"ZWG":0}"#));

    let register: Vec<&str> = CurrencyCode::all().iter().map(|c| c.as_str()).collect();
    assert_eq!(keys(&printed), register, "153 keys in register order");
    assert_eq!(keys_in_text_order(&text), register);
    assert_eq!(
        printed.to_value(),
        fixture.step(3).json()["stock"]["shifts"][0]["opening"]
    );
}

#[test]
fn balances_of_nothing_are_153_zeros() {
    let printed = balances_json(&Balances::empty());
    assert_eq!(pairs(&printed).len(), 153);
    assert!(pairs(&printed).iter().all(|(_, v)| v.to_text() == "0"));
    assert_eq!(printed.to_text().matches(":0").count(), 153);
}

#[test]
fn a_stock_delta_reproduces_the_difference_of_a_closed_shift() {
    let fixture = Fixture::load("snapshot/shift.json");
    let recorded = fixture.body_text(38).expect("shift.json#38 is JSON");
    let printed = stock_delta_json(&delta(&[("THB", -50)]));

    let text = printed.to_text();
    assert!(
        text.starts_with(r#"{"THB":-0.5,"USD":0,"EUR":0,"#),
        "{text}"
    );
    assert!(
        recorded.contains(&format!("\"difference\":{text},\"note\"")),
        "v2 printed another text than {text}"
    );
    assert_eq!(pairs(&printed).len(), 153);
    assert_eq!(
        printed.to_value(),
        fixture.step(38).json()["stock"]["shifts"][0]["difference"]
    );

    let none = stock_delta_json(&StockDelta::none());
    assert_eq!(none.to_text(), balances_json(&Balances::empty()).to_text());
}

// ── time ─────────────────────────────────────────────────────────────────────

#[test]
fn time_is_iso_with_milliseconds_or_empty() {
    let five = at("2026-10-06T05:00:00Z");
    assert_eq!(time_json(five).to_text(), r#""2026-10-06T05:00:00.000Z""#);
    assert_eq!(
        time_json(at("2026-10-06T05:00:00.120Z")).to_text(),
        r#""2026-10-06T05:00:00.120Z""#
    );
    // what is finer than a millisecond is cut, not rounded
    assert_eq!(
        time_json(at("2026-12-31T23:59:59.999600Z")).to_text(),
        r#""2026-12-31T23:59:59.999Z""#
    );
    // another offset is printed in UTC
    assert_eq!(
        time_json(at("2026-10-06T12:00:00.000+07:00")).to_text(),
        r#""2026-10-06T05:00:00.000Z""#
    );
    assert_eq!(
        time_or_empty_json(Some(five)).to_text(),
        r#""2026-10-06T05:00:00.000Z""#
    );
    assert_eq!(time_or_empty_json(None).to_text(), r#""""#);
    assert_eq!(time_or_empty_json(None).to_value(), json!(""));
    assert_eq!(
        custody_time_json(CustodyTime::from_epoch_millis(five.timestamp_millis())).to_text(),
        r#""2026-10-06T05:00:00.000Z""#
    );
    assert_eq!(
        custody_time_json(CustodyTime::from_epoch_millis(1_791_263_700_007)).to_value(),
        json!("2026-10-06T05:15:00.007Z")
    );
}

// ── rates ────────────────────────────────────────────────────────────────────

/// The rows `RateService::current` gives for a recorded `rates` list.
fn counter_rates_of(recorded: &Value) -> Vec<CounterRate> {
    recorded
        .as_array()
        .expect("rates is an array")
        .iter()
        .map(|row| CounterRate {
            code: code(&text_of(row, "code")),
            name: text_of(row, "name"),
            symbol: optional_text(row, "symbol"),
            buy: row.get("buy").filter(|v| !v.is_null()).map(rate_of),
            sell: row.get("sell").filter(|v| !v.is_null()).map(rate_of),
            updated_at: Some(text_of(row, "updated"))
                .filter(|t| !t.is_empty())
                .map(|t| at(&t)),
            request_id: Some(text_of(row, "requestId")).filter(|t| !t.is_empty()),
            partner_name: text_of(row, "partnerName"),
            partner_cost: row.get("partnerCost").filter(|v| !v.is_null()).map(rate_of),
            reference: row.get("botReference").filter(|v| !v.is_null()).cloned(),
        })
        .collect()
}

#[test]
fn rates_json_reproduces_the_152_unset_rows_of_empty_json() {
    let fixture = Fixture::load("snapshot/empty.json");
    let recorded = &fixture.step(1).json()["rates"];
    let printed = rates_json(&counter_rates_of(recorded));

    assert_eq!(items(&printed).len(), 152);
    assert_eq!(&printed.to_value(), recorded);
    // nothing in an unset row comes from a stored value, so the text is v2's
    let text = fixture.body_text(1).expect("empty.json#1 is JSON");
    assert!(
        text.contains(&format!("\"rates\":{},\"stock\"", printed.to_text())),
        "the text of the 152 rows is not v2's"
    );
    assert!(
        items(&printed)
            .iter()
            .all(|row| member(row, "code").to_text() != "\"THB\"")
    );
}

#[test]
fn an_unset_rate_is_null_and_empty_text_and_a_missing_symbol_is_absent() {
    let with_symbol = CounterRate {
        code: code("USD"),
        name: "ดอลลาร์สหรัฐ".to_string(),
        symbol: Some("$".to_string()),
        buy: None,
        sell: None,
        updated_at: None,
        request_id: None,
        partner_name: String::new(),
        partner_cost: None,
        reference: None,
    };
    let without_symbol = CounterRate {
        code: code("AED"),
        name: "เดอร์แฮมสหรัฐอาหรับเอมิเรตส์".to_string(),
        symbol: None,
        ..with_symbol.clone()
    };
    let printed = rates_json(&[with_symbol, without_symbol]);
    assert_eq!(
        printed.to_text(),
        concat!(
            r#"[{"code":"USD","name":"ดอลลาร์สหรัฐ","symbol":"$","buy":null,"sell":null,"updated":"","requestId":"","partnerName":"","partnerCost":null,"botReference":null},"#,
            r#"{"code":"AED","name":"เดอร์แฮมสหรัฐอาหรับเอมิเรตส์","buy":null,"sell":null,"updated":"","requestId":"","partnerName":"","partnerCost":null,"botReference":null}]"#
        )
    );
    let aed = &items(&printed)[1];
    assert!(!keys(aed).contains(&"symbol"), "absent, not null");
}

#[test]
fn rates_json_leaves_the_baht_out() {
    let row = |c: &str| CounterRate {
        code: code(c),
        name: c.to_string(),
        symbol: None,
        buy: None,
        sell: None,
        updated_at: None,
        request_id: None,
        partner_name: String::new(),
        partner_cost: None,
        reference: None,
    };
    let printed = rates_json(&[row("THB"), row("USD"), row("EUR")]);
    let codes: Vec<String> = items(&printed)
        .iter()
        .map(|r| member(r, "code").to_text())
        .collect();
    assert_eq!(codes, ["\"USD\"", "\"EUR\""]);
}

#[test]
fn rates_json_reproduces_the_set_rates_of_rates_json() {
    let fixture = Fixture::load("snapshot/rates.json");
    let recorded = &fixture.step(8).json()["rates"];
    let printed = rates_json(&counter_rates_of(recorded));
    assert_eq!(&printed.to_value(), recorded);
    assert_eq!(items(&printed).len(), 152);

    // the dollar, set twice, the second time with a partner — built by hand
    let reference = recorded[0]["botReference"].clone();
    let dollar = CounterRate {
        code: code("USD"),
        name: "ดอลลาร์สหรัฐ".to_string(),
        symbol: Some("$".to_string()),
        buy: Some(rate("32.15")),
        sell: Some(rate("32.6500")),
        updated_at: Some(at("2026-10-06T05:05:00.000Z")),
        request_id: Some("00000000-0000-4000-8000-0000000a0005".to_string()),
        partner_name: "=Partner starting with an equals sign".to_string(),
        partner_cost: Some(rate("31.9")),
        reference: Some(reference),
    };
    let printed = rates_json(&[dollar]);
    let row = &items(&printed)[0];
    assert_eq!(row.to_value(), recorded[0]);
    assert_eq!(
        keys(row),
        [
            "code",
            "name",
            "symbol",
            "buy",
            "sell",
            "updated",
            "requestId",
            "partnerName",
            "partnerCost",
            "botReference"
        ]
    );
    let head = concat!(
        r#"{"code":"USD","name":"ดอลลาร์สหรัฐ","symbol":"$","buy":32.15,"sell":32.65,"#,
        r#""updated":"2026-10-06T05:05:00.000Z","requestId":"00000000-0000-4000-8000-0000000a0005","#,
        r#""partnerName":"=Partner starting with an equals sign","partnerCost":31.9,"botReference":{"#
    );
    assert!(row.to_text().starts_with(head), "{}", row.to_text());
    let text = fixture.body_text(8).expect("rates.json#8 is JSON");
    assert!(text.contains(head), "v2 printed the dollar row otherwise");
}

#[test]
fn rate_history_json_is_newest_first_and_carries_no_request_id() {
    let fixture = Fixture::load("snapshot/rates.json");
    let recorded = &fixture.step(8).json()["rateHistory"];
    let reference = |i: usize| Some(recorded[i]["botReference"].clone());
    let change = |c: &str, buy: &str, sell: &str, when: &str, by: &str, n: u8| RateChange {
        code: code(c),
        buy: rate(buy),
        sell: rate(sell),
        updated_at: at(when),
        updated_by: by.to_string(),
        request_id: format!("00000000-0000-4000-8000-0000000a000{n}"),
        partner_name: String::new(),
        partner_cost: None,
        reference: None,
    };
    // as `RateService::history` gives them: newest first
    let history = vec![
        RateChange {
            partner_name: "=Partner starting with an equals sign".to_string(),
            partner_cost: Some(rate("31.9")),
            reference: reference(0),
            ..change(
                "USD",
                "32.15",
                "32.65",
                "2026-10-06T05:05:00.000Z",
                "rt1",
                5,
            )
        },
        RateChange {
            reference: reference(1),
            ..change("GBP", "42.5", "43.75", "2026-10-06T05:04:00.000Z", "rt1", 4)
        },
        RateChange {
            reference: reference(2),
            ..change(
                "JPY",
                "0.2105",
                "0.2199",
                "2026-10-06T05:03:00.000Z",
                "owner",
                3,
            )
        },
        RateChange {
            partner_name: "Partner FX".to_string(),
            partner_cost: Some(rate("37.4")),
            reference: reference(3),
            ..change(
                "EUR",
                "37.5",
                "38.25",
                "2026-10-06T05:02:00.000Z",
                "owner",
                2,
            )
        },
        RateChange {
            reference: reference(4),
            ..change(
                "USD",
                "32.1",
                "32.6",
                "2026-10-06T05:01:00.000Z",
                "owner",
                1,
            )
        },
    ];
    let printed = rate_history_json(&history);
    assert_eq!(&printed.to_value(), recorded);

    let updated: Vec<String> = items(&printed)
        .iter()
        .map(|row| member(row, "updated").to_text())
        .collect();
    let mut newest_first = updated.clone();
    newest_first.sort();
    newest_first.reverse();
    assert_eq!(updated, newest_first);

    for row in items(&printed) {
        assert_eq!(
            keys(row),
            [
                "code",
                "buy",
                "sell",
                "updated",
                "actor",
                "partnerName",
                "partnerCost",
                "botReference"
            ]
        );
    }
    assert!(!printed.to_text().contains("requestId"));
    assert!(
        !printed.to_text().contains("0000000a000"),
        "no request id leaks"
    );
    let head = r#"{"code":"JPY","buy":0.2105,"sell":0.2199,"updated":"2026-10-06T05:03:00.000Z","actor":"owner","partnerName":"","partnerCost":null,"botReference":{"#;
    assert!(items(&printed)[2].to_text().starts_with(head));
    assert!(fixture.body_text(8).expect("JSON").contains(head));

    assert_eq!(rate_history_json(&[]).to_text(), "[]");
}

// ── branches and customers ───────────────────────────────────────────────────

#[test]
fn branches_json_prints_main_without_a_request_id() {
    let fixture = Fixture::load("snapshot/branches.json");
    let branches = vec![
        Branch {
            id: main_branch(),
            name: "XC-B".to_string(),
            request_id: None,
            // OR-1 (#550 §6): the branch profile; branches_json prints none of it.
            code: None,
            address: None,
            timezone: "Asia/Bangkok".to_string(),
            opening_hours: None,
            booking_enabled: false,
        },
        Branch {
            id: BranchId::new("B00000000-0000-4000-8000-0000000a0002"),
            name: "Airport".to_string(),
            request_id: Some("00000000-0000-4000-8000-0000000a0002".to_string()),
            // OR-1 (#550 §6): the branch profile; branches_json prints none of it.
            code: None,
            address: None,
            timezone: "Asia/Bangkok".to_string(),
            opening_hours: None,
            booking_enabled: false,
        },
        Branch {
            id: BranchId::new("B00000000-0000-4000-8000-0000000a0003"),
            name: "Old Town".to_string(),
            request_id: Some("00000000-0000-4000-8000-0000000a0003".to_string()),
            // OR-1 (#550 §6): the branch profile; branches_json prints none of it.
            code: None,
            address: None,
            timezone: "Asia/Bangkok".to_string(),
            opening_hours: None,
            booking_enabled: false,
        },
    ];
    let printed = branches_json(&branches);
    assert_eq!(printed.to_value(), fixture.step(11).json()["branches"]);
    assert_eq!(keys(&items(&printed)[0]), ["id", "name"]);
    assert_eq!(keys(&items(&printed)[1]), ["id", "name", "requestId"]);
    let text = fixture.body_text(11).expect("branches.json#11 is JSON");
    assert!(text.contains(&format!("\"branches\":{},", printed.to_text())));
    assert!(
        printed
            .to_text()
            .starts_with(r#"[{"id":"MAIN","name":"XC-B"},{"id":"B0"#)
    );

    assert_eq!(
        branches_json(&branches[..1]).to_value(),
        Fixture::load("snapshot/empty.json").step(1).json()["branches"]
    );
}

fn customer(id: &str, name: &str, mode: Mode, created: &str) -> Customer {
    Customer {
        id: id.to_string(),
        name: name.to_string(),
        nationality: "DEU".to_string(),
        mode,
        created_at: at(created),
        last_document_ref: Some("XC-20261006-000001".to_string()),
    }
}

#[test]
fn customers_json_reproduces_branches_json_and_shows_no_passport() {
    let fixture = Fixture::load("snapshot/branches.json");
    let live = customer(
        "CX-00000001",
        "Customer 1",
        Mode::Live,
        "2026-10-06T05:09:00.000Z",
    );
    let printed = customers_json(std::slice::from_ref(&live));
    assert_eq!(printed.to_value(), fixture.step(11).json()["customers"]);
    assert_eq!(
        printed.to_text(),
        r#"[{"id":"CX-00000001","name":"Customer 1","nationality":"DEU","created":"2026-10-06T05:09:00.000Z","member":false,"memberVerifiedAt":null}]"#
    );
    let text = fixture.body_text(11).expect("branches.json#11 is JSON");
    assert!(text.contains(&format!("\"customers\":{},", printed.to_text())));
    for private in ["P1000001", "1990", "2031", "XC-2026"] {
        assert!(!printed.to_text().contains(private), "{private} leaks");
    }
}

#[test]
fn customers_json_never_shows_a_demo_customer() {
    let list = [
        customer(
            "CX-00000001",
            "Live one",
            Mode::Live,
            "2026-10-06T05:09:00.000Z",
        ),
        customer(
            "DM-00000001",
            "Demo one",
            Mode::Demo,
            "2026-10-06T05:10:00.000Z",
        ),
        customer(
            "CX-00000002",
            "Live two",
            Mode::Live,
            "2026-10-06T05:11:00.000Z",
        ),
    ];
    let printed = customers_json(&list);
    let ids: Vec<String> = items(&printed)
        .iter()
        .map(|c| member(c, "id").to_text())
        .collect();
    assert_eq!(ids, ["\"CX-00000001\"", "\"CX-00000002\""]);
}

// ── shifts and movements ─────────────────────────────────────────────────────

const SHIFT_KEYS: [&str; 13] = [
    "id",
    "status",
    "openedAt",
    "staff",
    "opening",
    "closedAt",
    "closedBy",
    "expected",
    "counted",
    "difference",
    "note",
    "openRequest",
    "closeRequest",
];

fn first_shift_open() -> Shift {
    Shift {
        id: "SH-00000000-0000-4000-8000-00000000000e".to_string(),
        branch_id: main_branch(),
        status: ShiftStatus::Open,
        opened_at: at("2026-10-06T05:02:00.000Z"),
        opened_by: "owner".to_string(),
        opening: balances(&[("THB", "50000"), ("USD", "1000.50")]),
        // v3 always holds what the drawer should hold; v2 shows it only once closed
        expected: balances(&[("THB", "48765.50"), ("USD", "1500.50")]),
        closed_at: None,
        closed_by: None,
        counted: None,
        difference: None,
        note: String::new(),
        open_request_id: "00000000-0000-4000-8000-0000000a0002".to_string(),
        close_request_id: None,
    }
}

fn first_shift_closed() -> Shift {
    Shift {
        status: ShiftStatus::Closed,
        closed_at: Some(at("2026-10-06T05:05:00.000Z")),
        closed_by: Some("mg1".to_string()),
        counted: Some(balances(&[("THB", "48765"), ("USD", "1500.5")])),
        difference: Some(delta(&[("THB", -50)])),
        note: "half a baht short".to_string(),
        close_request_id: Some("00000000-0000-4000-8000-0000000a0005".to_string()),
        ..first_shift_open()
    }
}

fn second_shift_open() -> Shift {
    Shift {
        id: "SH-00000000-0000-4000-8000-000000000015".to_string(),
        opened_at: at("2026-10-06T05:06:00.000Z"),
        opened_by: "be1".to_string(),
        opening: balances(&[("THB", "48765"), ("USD", "1500.5")]),
        expected: balances(&[("THB", "48765"), ("USD", "1500.5"), ("EUR", "0.01")]),
        open_request_id: "00000000-0000-4000-8000-0000000a0006".to_string(),
        ..first_shift_open()
    }
}

#[test]
fn an_open_shift_shows_no_expected_no_count_and_empty_close_fields() {
    let fixture = Fixture::load("snapshot/shift.json");
    let printed = shifts_json(&[first_shift_open()]);
    let shift = &items(&printed)[0];

    assert_eq!(keys(shift), SHIFT_KEYS);
    assert_eq!(member(shift, "status").to_text(), r#""OPEN""#);
    assert_eq!(member(shift, "closedAt").to_text(), r#""""#);
    assert_eq!(member(shift, "closedBy").to_text(), r#""""#);
    assert_eq!(member(shift, "closeRequest").to_text(), r#""""#);
    for hidden in ["expected", "counted", "difference"] {
        assert_eq!(member(shift, hidden), &V2Json::Null, "{hidden} while OPEN");
    }
    assert_eq!(
        printed.to_value(),
        without_row_keys(&fixture.step(3).json()["stock"]["shifts"])
    );
    let tail = concat!(
        r#""closedAt":"","closedBy":"","expected":null,"counted":null,"difference":null,"#,
        r#""note":"","openRequest":"00000000-0000-4000-8000-0000000a0002","closeRequest":""}]"#
    );
    assert!(printed.to_text().ends_with(tail), "{}", printed.to_text());
}

#[test]
fn a_closed_shift_shows_the_three_objects_of_shift_json() {
    let fixture = Fixture::load("snapshot/shift.json");
    let recorded = &fixture.step(38).json()["stock"]["shifts"];
    let printed = shifts_json(&[first_shift_closed()]);
    let shift = &items(&printed)[0];

    assert_eq!(printed.to_value(), without_row_keys(recorded));
    assert_eq!(keys(shift), SHIFT_KEYS);
    for shown in ["opening", "expected", "counted", "difference"] {
        assert_eq!(pairs(member(shift, shown)).len(), 153, "{shown}");
    }
    assert_eq!(
        member(member(shift, "expected"), "THB").to_text(),
        "48765.5"
    );
    assert_eq!(member(member(shift, "counted"), "THB").to_text(), "48765");
    assert_eq!(member(member(shift, "difference"), "THB").to_text(), "-0.5");

    // each of the four objects is v2's text, key for key
    let text = fixture.body_text(38).expect("shift.json#38 is JSON");
    let opening = member(shift, "opening").to_text();
    let expected = member(shift, "expected").to_text();
    let counted = member(shift, "counted").to_text();
    let difference = member(shift, "difference").to_text();
    assert!(text.contains(&format!(
        "\"opening\":{opening},\"closedAt\":\"2026-10-06T05:05:00.000Z\",\"closedBy\":\"mg1\","
    )));
    assert!(text.contains(&format!(
        "\"expected\":{expected},\"counted\":{counted},\"difference\":{difference},\"note\":\"half a baht short\",\"openRequest\":\"00000000-0000-4000-8000-0000000a0002\",\"closeRequest\":\"00000000-0000-4000-8000-0000000a0005\"}}"
    )));
    assert!(printed.to_text().contains(&format!(
        "\"expected\":{expected},\"counted\":{counted},\"difference\":{difference},\"note\":"
    )));
}

#[test]
fn a_closed_shift_without_a_count_prints_null_for_what_is_missing() {
    let odd = Shift {
        counted: None,
        difference: None,
        ..first_shift_closed()
    };
    let printed = shifts_json(&[odd]);
    let shift = &items(&printed)[0];
    assert_eq!(pairs(member(shift, "expected")).len(), 153);
    assert_eq!(member(shift, "counted"), &V2Json::Null);
    assert_eq!(member(shift, "difference"), &V2Json::Null);
}

#[test]
fn shifts_json_keeps_the_order_given_oldest_first() {
    let fixture = Fixture::load("snapshot/shift.json");
    let printed = shifts_json(&[first_shift_closed(), second_shift_open()]);
    assert_eq!(
        printed.to_value(),
        without_row_keys(&fixture.step(46).json()["stock"]["shifts"])
    );
    assert_eq!(member(&items(&printed)[1], "expected"), &V2Json::Null);
    assert_eq!(shifts_json(&[]).to_text(), "[]");
}

fn recorded_movements() -> Vec<StockMovement> {
    let first = "SH-00000000-0000-4000-8000-00000000000e";
    let second = "SH-00000000-0000-4000-8000-000000000015";
    vec![
        StockMovement {
            id: "MV-00000000-0000-4000-8000-000000000010".to_string(),
            shift_id: first.to_string(),
            at: at("2026-10-06T05:03:00.000Z"),
            by: "fi1".to_string(),
            currency: code("USD"),
            direction: Direction::In,
            amount: money("500"),
            reason: "top up from the safe".to_string(),
            request_id: "00000000-0000-4000-8000-0000000a0003".to_string(),
        },
        StockMovement {
            id: "MV-00000000-0000-4000-8000-000000000012".to_string(),
            shift_id: first.to_string(),
            at: at("2026-10-06T05:04:00.000Z"),
            by: "owner".to_string(),
            currency: code("THB"),
            direction: Direction::Out,
            amount: money("1234.50"),
            reason: "-deposit at the bank".to_string(),
            request_id: "00000000-0000-4000-8000-0000000a0004".to_string(),
        },
        StockMovement {
            id: "MV-00000000-0000-4000-8000-000000000017".to_string(),
            shift_id: second.to_string(),
            at: at("2026-10-06T05:07:00.000Z"),
            by: "owner".to_string(),
            currency: code("EUR"),
            direction: Direction::In,
            amount: money("0.01"),
            reason: "one cent found".to_string(),
            request_id: "00000000-0000-4000-8000-0000000a0007".to_string(),
        },
    ]
}

#[test]
fn movements_json_reproduces_the_text_of_shift_json() {
    let fixture = Fixture::load("snapshot/shift.json");
    let printed = movements_json(&recorded_movements());
    assert_eq!(
        printed.to_value(),
        fixture.step(46).json()["stock"]["movements"]
    );
    // a movement holds no object and no left-out key: the whole text is v2's
    let text = fixture.body_text(46).expect("shift.json#46 is JSON");
    assert!(
        text.contains(&format!("\"movements\":{}}}", printed.to_text())),
        "{}",
        printed.to_text()
    );
    assert_eq!(
        items(&printed)[1].to_text(),
        concat!(
            r#"{"id":"MV-00000000-0000-4000-8000-000000000012","shiftId":"SH-00000000-0000-4000-8000-00000000000e","#,
            r#""created":"2026-10-06T05:04:00.000Z","staff":"owner","currency":"THB","direction":"OUT","#,
            r#""amount":1234.5,"reason":"-deposit at the bank","requestId":"00000000-0000-4000-8000-0000000a0004"}"#
        )
    );
    assert_eq!(
        member(&items(&printed)[0], "direction").to_text(),
        r#""IN""#
    );
    assert_eq!(member(&items(&printed)[0], "amount").to_text(), "500");
    assert_eq!(member(&items(&printed)[2], "amount").to_text(), "0.01");
    assert_eq!(movements_json(&[]).to_text(), "[]");
}

#[test]
fn text_that_starts_with_a_formula_sign_is_printed_as_typed() {
    // v2 stored a leading apostrophe and took it off again; v3 stores what was typed
    for typed in [
        "=SUM(A1)",
        "+66 81",
        "-deposit at the bank",
        "@home",
        "'quoted",
    ] {
        let movement = StockMovement {
            reason: typed.to_string(),
            ..recorded_movements().remove(0)
        };
        let printed = movements_json(&[movement]);
        assert_eq!(
            member(&items(&printed)[0], "reason").to_value(),
            json!(typed)
        );

        let shift = Shift {
            note: typed.to_string(),
            ..first_shift_closed()
        };
        let printed = shifts_json(&[shift]);
        assert_eq!(member(&items(&printed)[0], "note").to_value(), json!(typed));

        let branch = Branch {
            id: BranchId::new("B1"),
            name: typed.to_string(),
            request_id: None,
            // OR-1 (#550 §6): the branch profile; branches_json prints none of it.
            code: None,
            address: None,
            timezone: "Asia/Bangkok".to_string(),
            opening_hours: None,
            booking_enabled: false,
        };
        let printed = branches_json(&[branch]);
        assert_eq!(member(&items(&printed)[0], "name").to_value(), json!(typed));
    }
    let printed = movements_json(&recorded_movements());
    assert!(
        !printed.to_text().contains("\"'"),
        "an apostrophe was added"
    );
}

// ── custody: a recorded `operations` as the state #72 will hand over ──────────

fn role_of(text: &str) -> CustodyRole {
    match text {
        "OWNER" => CustodyRole::Owner,
        "FRONT" => CustodyRole::Front,
        "FINANCE" => CustodyRole::Finance,
        "MONITOR" => CustodyRole::Monitor,
        "MANAGER" => CustodyRole::Manager,
        "BACKEND" => CustodyRole::Backend,
        _ => CustodyRole::Other,
    }
}

fn phase_of(text: &str) -> Phase {
    [
        Phase::Closed,
        Phase::Ready,
        Phase::Open,
        Phase::CutOff,
        Phase::Counting,
    ]
    .into_iter()
    .find(|p| p.as_str() == text)
    .unwrap_or_else(|| panic!("no phase {text}"))
}

fn kind_of(text: &str) -> RequestKind {
    [
        RequestKind::Open,
        RequestKind::Handover,
        RequestKind::Close,
        RequestKind::SafeOpen,
        RequestKind::SafeIn,
        RequestKind::SafeOut,
        RequestKind::ExternalIn,
        RequestKind::ExternalOut,
    ]
    .into_iter()
    .find(|k| k.as_str() == text)
    .unwrap_or_else(|| panic!("no request kind {text}"))
}

fn event_kind_of(text: &str) -> EventKind {
    [
        EventKind::SetEntity,
        EventKind::Enable,
        EventKind::ClockIn,
        EventKind::ClockOut,
        EventKind::CashTarget,
        EventKind::ServiceOpen,
        EventKind::CutOff,
        EventKind::Propose,
        EventKind::HoldCustomer,
        EventKind::ReleaseCustomer,
        EventKind::Reject,
        EventKind::Sign,
        EventKind::Dispatch,
        EventKind::Commit,
        EventKind::Reconcile,
    ]
    .into_iter()
    .find(|k| k.as_str() == text)
    .unwrap_or_else(|| panic!("no event kind {text}"))
}

fn target_of(value: &Value) -> CashTarget {
    CashTarget {
        amounts: balances_of(&value["amounts"]),
        by: text_of(value, "by"),
        at: custody_at(&text_of(value, "at")),
    }
}

fn cutoff_of(value: &Value) -> Cutoff {
    Cutoff {
        at: custody_at(&text_of(value, "at")),
        by: text_of(value, "by"),
        shift_id: text_of(value, "shiftId"),
        last_ref: optional_text(value, "lastRef"),
        pending: array(value, "pending")
            .iter()
            .map(|p| p.as_str().expect("a pending ref is text").to_string())
            .collect(),
    }
}

fn sheet_of(request: &Value, totals: &str, rows: &str) -> Option<CountSheet> {
    let totals = optional_balances(request, totals)?;
    let denominations = array(request, rows)
        .iter()
        .map(|row| DenominationRow {
            currency: code(&text_of(row, "currency")),
            denomination: money_of(&row["denomination"]),
            count: u32::try_from(row["count"].as_u64().expect("a count")).expect("a small count"),
        })
        .collect();
    Some(CountSheet {
        totals,
        denominations,
    })
}

fn decision_of(value: &Value, text_key: &str) -> Decision {
    Decision {
        by: text_of(value, "by"),
        at: custody_at(&text_of(value, "at")),
        text: text_of(value, text_key),
    }
}

fn request_of(r: &Value) -> CustodyRequest {
    CustodyRequest {
        id: text_of(r, "id"),
        request_id: text_of(r, "requestId"),
        payload_hash: text_of(r, "hash"),
        kind: kind_of(&text_of(r, "kind")),
        status: match text_of(r, "status").as_str() {
            "PENDING" => RequestStatus::Pending,
            "REJECTED" => RequestStatus::Rejected,
            "COMPLETED" => RequestStatus::Completed,
            other => panic!("no request status {other}"),
        },
        at: custody_at(&text_of(r, "at")),
        sender: text_of(r, "sender"),
        receiver: optional_text(r, "receiver"),
        note: text_of(r, "note"),
        evidence_keys: array(r, "evidenceKeys")
            .iter()
            .map(|k| k.as_str().expect("a key is text").to_string())
            .collect(),
        signatures: array(r, "signatures")
            .iter()
            .map(|s| Signature {
                employee_id: text_of(s, "employeeId"),
                role: role_of(&text_of(s, "role")),
                at: custody_at(&text_of(s, "at")),
            })
            .collect(),
        shift_id: optional_text(r, "shiftId"),
        counted: sheet_of(r, "counted", "denominations"),
        target: r.get("target").map(target_of),
        target_difference: optional_delta(r, "targetDifference"),
        expected: optional_balances(r, "expected"),
        variance: optional_delta(r, "variance"),
        cutoff: r.get("cutoff").map(cutoff_of),
        safe_count: sheet_of(r, "safeCount", "safeDenominations"),
        safe_expected: optional_balances(r, "safeExpected"),
        safe_variance: optional_delta(r, "safeVariance"),
        currency: optional_text(r, "currency").map(|c| code(&c)),
        amount: r.get("amount").map(money_of),
        stage: optional_text(r, "stage").map(|s| match s.as_str() {
            "IN_TRANSIT" => TransferStage::InTransit,
            "RECEIVED" => TransferStage::Received,
            other => panic!("no stage {other}"),
        }),
        dispatched_at: optional_custody_time(r, "dispatchedAt"),
        dispatched_by: optional_text(r, "dispatchedBy"),
        rejected: r.get("rejected").map(|d| decision_of(d, "reason")),
        completed_at: optional_custody_time(r, "completedAt"),
        reconciliation: optional_text(r, "reconciliation").map(|s| match s.as_str() {
            "PENDING" => Reconciliation::Pending,
            "CHECKED" => Reconciliation::Checked,
            other => panic!("no reconciliation {other}"),
        }),
        reconciled: r.get("reconciled").map(|d| decision_of(d, "note")),
    }
}

fn state_of(ops: &Value) -> CustodyState {
    CustodyState {
        enabled: ops["enabled"].as_bool().expect("enabled"),
        phase: phase_of(&text_of(ops, "phase")),
        safe: optional_balances(ops, "safe"),
        target: ops.get("target").filter(|v| !v.is_null()).map(target_of),
        entity: ops.get("entity").filter(|v| !v.is_null()).map(|e| Entity {
            company: text_of(e, "company"),
            site: text_of(e, "site"),
            confirmed_by: text_of(e, "confirmedBy"),
            at: custody_at(&text_of(e, "at")),
        }),
        custodian: optional_text(ops, "custodian"),
        attendance: array(ops, "attendance")
            .iter()
            .map(|a| Attendance {
                id: text_of(a, "id"),
                employee_id: text_of(a, "employeeId"),
                role: role_of(&text_of(a, "role")),
                clock_in: custody_at(&text_of(a, "in")),
                clock_out: optional_custody_time(a, "out"),
            })
            .collect(),
        requests: array(ops, "requests").iter().map(request_of).collect(),
        cutoff: ops.get("cutoff").filter(|v| !v.is_null()).map(cutoff_of),
        exceptions: array(ops, "exceptions")
            .iter()
            .map(|x| CashException {
                id: text_of(x, "id"),
                request_id: text_of(x, "requestId"),
                status: match text_of(x, "status").as_str() {
                    "OPEN" => ExceptionStatus::Open,
                    "REVIEWED" => ExceptionStatus::Reviewed,
                    other => panic!("no exception status {other}"),
                },
                target_difference: optional_delta(x, "targetDifference"),
                cash_variance: optional_delta(x, "cashVariance"),
                safe_variance: optional_delta(x, "safeVariance"),
                note: text_of(x, "note"),
                at: custody_at(&text_of(x, "at")),
                reviewed_by: optional_text(x, "reviewedBy"),
                reviewed_at: optional_custody_time(x, "reviewedAt"),
            })
            .collect(),
        holds: array(ops, "holds")
            .iter()
            .map(|h| CustomerHold {
                id: text_of(h, "id"),
                customer_id: text_of(h, "customerId"),
                reason: text_of(h, "reason"),
                by: text_of(h, "by"),
                at: custody_at(&text_of(h, "at")),
                status: match text_of(h, "status").as_str() {
                    "OPEN" => HoldStatus::Open,
                    "RELEASED" => HoldStatus::Released,
                    other => panic!("no hold status {other}"),
                },
                reviewed_by: optional_text(h, "reviewedBy"),
                reviewed_at: optional_custody_time(h, "reviewedAt"),
                note: optional_text(h, "note"),
            })
            .collect(),
        drafts: array(ops, "drafts")
            .iter()
            .map(|d| TxDraft {
                id: text_of(d, "id"),
                shift_id: text_of(d, "shiftId"),
                employee_id: text_of(d, "employeeId"),
                side: match text_of(d, "type").as_str() {
                    "BUY" => Side::Buy,
                    "SELL" => Side::Sell,
                    other => panic!("no side {other}"),
                },
                currency: code(&text_of(d, "currency")),
                amount: money_of(&d["amount"]),
                at: custody_at(&text_of(d, "at")),
                status: match text_of(d, "status").as_str() {
                    "OPEN" => DraftStatus::Open,
                    "COMPLETED" => DraftStatus::Completed,
                    other => panic!("no draft status {other}"),
                },
                document_ref: optional_text(d, "documentRef"),
            })
            .collect(),
        service_opened_at: optional_custody_time(ops, "serviceOpenedAt"),
    }
}

/// The names and roles of the people of a fixture, by employee id.
fn people_of(fixture: &Fixture) -> People {
    fixture
        .people
        .iter()
        .map(|p| {
            (
                p.id.clone(),
                PersonLabel {
                    name: p.name.clone(),
                    role: p.role.clone(),
                },
            )
        })
        .collect()
}

/// The custody log of a recorded `operations`, as `CustodyService::events`
/// gives it: custody knows six roles and holds every other as `Other`.
fn events_of(ops: &Value) -> Vec<StoredEvent> {
    array(ops, "events")
        .iter()
        .map(|e| StoredEvent {
            id: text_of(e, "id"),
            kind: event_kind_of(&text_of(e, "type")),
            at: at(&text_of(e, "at")),
            actor: text_of(e, "actor"),
            role: role_of(&text_of(e, "role")),
            details: e["details"].clone(),
        })
        .collect()
}

const INITIAL_OPERATION_KEYS: [&str; 13] = [
    "enabled",
    "phase",
    "safe",
    "target",
    "entity",
    "custodian",
    "attendance",
    "requests",
    "events",
    "cutoff",
    "exceptions",
    "holds",
    "drafts",
];

const REQUEST_HEAD_KEYS: [&str; 13] = [
    "id",
    "requestId",
    "hash",
    "kind",
    "status",
    "at",
    "sender",
    "senderName",
    "receiver",
    "note",
    "evidenceKeys",
    "signatures",
    "shiftId",
];

// ── operations ───────────────────────────────────────────────────────────────

#[test]
fn operations_of_a_branch_that_never_used_custody_is_the_initial_object_of_empty_json() {
    let fixture = Fixture::load("snapshot/empty.json");
    let printed = operations_json(&CustodyState::initial(), &[], &People::new());

    assert_eq!(keys(&printed), INITIAL_OPERATION_KEYS);
    assert_eq!(
        printed.to_text(),
        concat!(
            r#"{"enabled":false,"phase":"CLOSED","safe":null,"target":null,"entity":null,"custodian":null,"#,
            r#""attendance":[],"requests":[],"events":[],"cutoff":null,"exceptions":[],"holds":[],"drafts":[]}"#
        )
    );
    assert_eq!(printed.to_value(), fixture.step(1).json()["operations"]);
    let text = fixture.body_text(1).expect("empty.json#1 is JSON");
    assert!(text.ends_with(&format!("\"operations\":{}}}", printed.to_text())));
    assert!(
        !printed.to_text().contains("serviceOpenedAt"),
        "absent, not null"
    );
}

#[test]
fn operations_json_reproduces_every_view_of_custody_json() {
    let fixture = Fixture::load("snapshot/custody.json");
    let people = people_of(&fixture);
    let mut views = 0;
    let mut service_seen_open = false;
    for step in fixture.steps() {
        let owner_snapshot = step.view.is_some()
            && step.sent_as == "owner"
            && step.action.as_deref() == Some("adminSnapshot");
        if !owner_snapshot {
            continue;
        }
        views += 1;
        let view = step.view.as_deref().unwrap_or_default();
        let recorded = &step.json()["operations"];
        let events = events_of(recorded);
        let printed = operations_json(&state_of(recorded), &events, &people);

        assert_eq!(
            &printed.to_value(),
            recorded,
            "{view} (custody.json#{})",
            step.n
        );

        // `serviceOpenedAt` is appended the first time service starts
        let opened = recorded.get("serviceOpenedAt").is_some();
        let mut expected_keys: Vec<&str> = INITIAL_OPERATION_KEYS.to_vec();
        if opened {
            expected_keys.push("serviceOpenedAt");
            service_seen_open = true;
        }
        assert_eq!(keys(&printed), expected_keys, "{view}");

        // everything but `events` is built key by key: its text is v2's
        let text = fixture.body_text(step.n).expect("a view is JSON");
        for (key, value) in pairs(&printed) {
            if key == "events" {
                continue;
            }
            let piece = format!("\"{key}\":{}", value.to_text());
            assert!(
                text.contains(&piece),
                "{view}: `{key}` is not v2's text: {}",
                value.to_text()
            );
        }
        let head = format!(
            "\"operations\":{{\"enabled\":{},\"phase\":{},\"safe\":{},\"target\":{},\"entity\":{},\"custodian\":{},\"attendance\":{},\"requests\":{},\"events\":[",
            member(&printed, "enabled").to_text(),
            member(&printed, "phase").to_text(),
            member(&printed, "safe").to_text(),
            member(&printed, "target").to_text(),
            member(&printed, "entity").to_text(),
            member(&printed, "custodian").to_text(),
            member(&printed, "attendance").to_text(),
            member(&printed, "requests").to_text(),
        );
        assert!(text.contains(&head), "{view}: the first eight keys");
        let mut tail = format!(
            "],\"cutoff\":{},\"exceptions\":{},\"holds\":{},\"drafts\":{}",
            member(&printed, "cutoff").to_text(),
            member(&printed, "exceptions").to_text(),
            member(&printed, "holds").to_text(),
            member(&printed, "drafts").to_text(),
        );
        if opened {
            tail.push_str(&format!(
                ",\"serviceOpenedAt\":{}",
                member(&printed, "serviceOpenedAt").to_text()
            ));
        }
        tail.push_str("}}");
        assert!(text.ends_with(&tail), "{view}: the last keys");

        for event in items(member(&printed, "events")) {
            assert_eq!(
                keys(event),
                ["id", "type", "at", "actor", "role", "details"]
            );
        }
    }
    assert_eq!(views, 9, "custody.json shows nine states to the owner");
    assert!(service_seen_open);
}

#[test]
fn a_request_has_its_optional_keys_only_once_they_happened() {
    let fixture = Fixture::load("snapshot/custody.json");
    let people = people_of(&fixture);
    let requests_at = |n: usize| -> Vec<Vec<String>> {
        let recorded = &fixture.step(n).json()["operations"];
        let printed = operations_json(&state_of(recorded), &[], &people);
        items(member(&printed, "requests"))
            .iter()
            .map(|r| keys(r).into_iter().map(str::to_string).collect())
            .collect()
    };
    let with = |extra: &[&str]| -> Vec<String> {
        REQUEST_HEAD_KEYS
            .iter()
            .chain(extra)
            .map(|k| k.to_string())
            .collect()
    };

    // the safe count waits for a signature
    assert_eq!(requests_at(24), [with(&["counted", "denominations"])]);
    // the shift is open: both are completed
    assert_eq!(
        requests_at(51),
        [
            with(&["counted", "denominations", "completedAt", "reconciliation"]),
            with(&[
                "counted",
                "denominations",
                "target",
                "targetDifference",
                "completedAt",
                "reconciliation"
            ]),
        ]
    );
    // money from the safe is in transit
    assert_eq!(
        requests_at(70)[2],
        with(&[
            "currency",
            "amount",
            "expected",
            "stage",
            "dispatchedAt",
            "dispatchedBy"
        ])
    );
    // a refused transfer was never dispatched; a completed one was
    let on_hold = requests_at(109);
    assert_eq!(
        on_hold[3],
        with(&["currency", "amount", "expected", "rejected"])
    );
    assert_eq!(
        on_hold[4],
        with(&[
            "currency",
            "amount",
            "expected",
            "stage",
            "dispatchedAt",
            "dispatchedBy",
            "completedAt",
            "reconciliation"
        ])
    );
    // a handover being counted, and the close once reconciled
    let handover = [
        "counted",
        "denominations",
        "expected",
        "variance",
        "cutoff",
        "safeCount",
        "safeDenominations",
        "safeExpected",
        "safeVariance",
    ];
    assert_eq!(requests_at(125)[7], with(&handover));
    let mut reconciled = handover.to_vec();
    reconciled.extend(["completedAt", "reconciliation", "reconciled"]);
    assert_eq!(requests_at(206)[8], with(&reconciled));
}

fn pending_transfer() -> CustodyRequest {
    CustodyRequest {
        id: "R-1".to_string(),
        request_id: "00000000-0000-4000-8000-0000000a0005".to_string(),
        payload_hash: "abc123".to_string(),
        kind: RequestKind::SafeOut,
        status: RequestStatus::Pending,
        at: custody_at("2026-10-06T05:34:00.000Z"),
        sender: "E-FRONT".to_string(),
        receiver: Some("E-FINANCE".to_string()),
        note: "top up the drawer".to_string(),
        evidence_keys: vec!["K-1".to_string(), "K-2".to_string()],
        signatures: vec![Signature {
            employee_id: "E-FRONT".to_string(),
            role: CustodyRole::Front,
            at: custody_at("2026-10-06T05:34:00.000Z"),
        }],
        shift_id: Some("SH-1".to_string()),
        counted: None,
        target: None,
        target_difference: None,
        expected: Some(balances(&[("THB", "91975"), ("USD", "2140")])),
        variance: None,
        cutoff: None,
        safe_count: None,
        safe_expected: None,
        safe_variance: None,
        currency: Some(code("USD")),
        amount: Some(money("300.25")),
        stage: None,
        dispatched_at: None,
        dispatched_by: None,
        rejected: None,
        completed_at: None,
        reconciliation: None,
        reconciled: None,
    }
}

fn hand_people() -> People {
    let label = |name: &str, role: &str| PersonLabel {
        name: name.to_string(),
        role: role.to_string(),
    };
    BTreeMap::from([
        ("E-FRONT".to_string(), label("Fah Front", "FRONT")),
        ("E-FINANCE".to_string(), label("Fin Finance", "FINANCE")),
        ("E-RATES".to_string(), label("Rat Rates", "RATES")),
    ])
}

fn state_with(request: CustodyRequest) -> CustodyState {
    CustodyState {
        enabled: true,
        requests: vec![request],
        ..CustodyState::initial()
    }
}

#[test]
fn a_pending_transfer_is_printed_key_for_key() {
    let printed = operations_json(&state_with(pending_transfer()), &[], &hand_people());
    let request = &items(member(&printed, "requests"))[0];
    let expected = member(request, "expected").to_text();
    assert_eq!(
        request.to_text(),
        format!(
            concat!(
                r#"{{"id":"R-1","requestId":"00000000-0000-4000-8000-0000000a0005","hash":"abc123","#,
                r#""kind":"SAFE_OUT","status":"PENDING","at":"2026-10-06T05:34:00.000Z","#,
                r#""sender":"E-FRONT","senderName":"Fah Front","receiver":"E-FINANCE","#,
                r#""note":"top up the drawer","evidenceKeys":["K-1","K-2"],"#,
                r#""signatures":[{{"employeeId":"E-FRONT","role":"FRONT","at":"2026-10-06T05:34:00.000Z"}}],"#,
                r#""shiftId":"SH-1","currency":"USD","amount":300.25,"expected":{}}}"#
            ),
            expected
        )
    );
    assert!(expected.starts_with(r#"{"THB":91975,"USD":2140,"EUR":0,"#));
}

#[test]
fn what_happens_to_a_transfer_is_appended_in_the_order_it_happens() {
    let people = hand_people();
    let tail_of = |request: CustodyRequest| -> String {
        let printed = operations_json(&state_with(request), &[], &people);
        let request = &items(member(&printed, "requests"))[0];
        let text = request.to_text();
        let from = text.find(",\"expected\":").expect("expected is printed");
        let after = text[from + 1..].find("},").map(|i| from + 1 + i + 2);
        after.map(|i| text[i..].to_string()).unwrap_or_default()
    };

    assert_eq!(tail_of(pending_transfer()), "");

    let dispatched = CustodyRequest {
        stage: Some(TransferStage::InTransit),
        dispatched_at: Some(custody_at("2026-10-06T05:35:00.000Z")),
        dispatched_by: Some("E-BACK".to_string()),
        ..pending_transfer()
    };
    assert_eq!(
        tail_of(dispatched.clone()),
        r#""stage":"IN_TRANSIT","dispatchedAt":"2026-10-06T05:35:00.000Z","dispatchedBy":"E-BACK"}"#
    );

    let rejected = CustodyRequest {
        status: RequestStatus::Rejected,
        rejected: Some(Decision {
            by: "E-MON".to_string(),
            at: custody_at("2026-10-06T05:41:00.000Z"),
            text: "the drawer never held that much".to_string(),
        }),
        ..pending_transfer()
    };
    assert_eq!(
        tail_of(rejected),
        r#""rejected":{"by":"E-MON","at":"2026-10-06T05:41:00.000Z","reason":"the drawer never held that much"}}"#
    );

    let reconciled = CustodyRequest {
        status: RequestStatus::Completed,
        stage: Some(TransferStage::Received),
        completed_at: Some(custody_at("2026-10-06T05:37:00.000Z")),
        reconciliation: Some(Reconciliation::Checked),
        reconciled: Some(Decision {
            by: "E-FINANCE".to_string(),
            at: custody_at("2026-10-06T06:20:00.000Z"),
            text: "checked".to_string(),
        }),
        ..dispatched
    };
    assert_eq!(
        tail_of(reconciled.clone()),
        concat!(
            r#""stage":"RECEIVED","dispatchedAt":"2026-10-06T05:35:00.000Z","dispatchedBy":"E-BACK","#,
            r#""completedAt":"2026-10-06T05:37:00.000Z","reconciliation":"CHECKED","#,
            r#""reconciled":{"by":"E-FINANCE","at":"2026-10-06T06:20:00.000Z","note":"checked"}}"#
        )
    );
    let pending_check = CustodyRequest {
        reconciliation: Some(Reconciliation::Pending),
        reconciled: None,
        ..reconciled
    };
    assert!(tail_of(pending_check).ends_with(r#""reconciliation":"PENDING"}"#));
}

#[test]
fn the_proposer_signs_without_a_name_and_everyone_after_with_one() {
    let request = CustodyRequest {
        signatures: vec![
            Signature {
                employee_id: "E-FRONT".to_string(),
                role: CustodyRole::Front,
                at: custody_at("2026-10-06T05:34:00.000Z"),
            },
            Signature {
                employee_id: "E-FINANCE".to_string(),
                role: CustodyRole::Finance,
                at: custody_at("2026-10-06T05:36:00.000Z"),
            },
            Signature {
                employee_id: "E-GONE".to_string(),
                role: CustodyRole::Backend,
                at: custody_at("2026-10-06T05:37:00.000Z"),
            },
        ],
        ..pending_transfer()
    };
    let printed = operations_json(&state_with(request), &[], &hand_people());
    let request = &items(member(&printed, "requests"))[0];
    assert_eq!(
        member(request, "signatures").to_text(),
        concat!(
            r#"[{"employeeId":"E-FRONT","role":"FRONT","at":"2026-10-06T05:34:00.000Z"},"#,
            r#"{"employeeId":"E-FINANCE","name":"Fin Finance","role":"FINANCE","at":"2026-10-06T05:36:00.000Z"},"#,
            r#"{"employeeId":"E-GONE","name":"","role":"BACKEND","at":"2026-10-06T05:37:00.000Z"}]"#
        )
    );
}

#[test]
fn a_person_nobody_knows_is_printed_with_an_empty_name() {
    let state = CustodyState {
        attendance: vec![Attendance {
            id: "A-1".to_string(),
            employee_id: "E-GONE".to_string(),
            role: CustodyRole::Monitor,
            clock_in: custody_at("2026-10-06T05:03:00.000Z"),
            clock_out: None,
        }],
        ..state_with(CustodyRequest {
            sender: "E-GONE".to_string(),
            receiver: None,
            shift_id: None,
            ..pending_transfer()
        })
    };
    let printed = operations_json(&state, &[], &hand_people());
    assert_eq!(
        member(&printed, "attendance").to_text(),
        r#"[{"id":"A-1","employeeId":"E-GONE","name":"","role":"MONITOR","in":"2026-10-06T05:03:00.000Z","out":null}]"#
    );
    let request = &items(member(&printed, "requests"))[0];
    assert_eq!(member(request, "senderName").to_text(), r#""""#);
    assert_eq!(member(request, "receiver"), &V2Json::Null);
    assert_eq!(member(request, "shiftId"), &V2Json::Null);
}

#[test]
fn a_role_custody_does_not_know_is_named_from_the_account() {
    let clocked = |id: &str, employee: &str, role: CustodyRole| Attendance {
        id: id.to_string(),
        employee_id: employee.to_string(),
        role,
        clock_in: custody_at("2026-10-06T05:11:00.000Z"),
        clock_out: Some(custody_at("2026-10-06T06:21:00.000Z")),
    };
    let state = CustodyState {
        attendance: vec![
            clocked("A-1", "E-RATES", CustodyRole::Other),
            clocked("A-2", "E-GONE", CustodyRole::Other),
            // a role custody knows is its own, whatever the account says today
            clocked("A-3", "E-RATES", CustodyRole::Manager),
            clocked("A-4", "owner", CustodyRole::Owner),
        ],
        ..CustodyState::initial()
    };
    let printed = operations_json(&state, &[], &hand_people());
    let roles: Vec<String> = items(member(&printed, "attendance"))
        .iter()
        .map(|a| member(a, "role").to_text())
        .collect();
    assert_eq!(
        roles,
        ["\"RATES\"", "\"OTHER\"", "\"MANAGER\"", "\"OWNER\""]
    );
    let first = &items(member(&printed, "attendance"))[0];
    assert_eq!(
        first.to_text(),
        r#"{"id":"A-1","employeeId":"E-RATES","name":"Rat Rates","role":"RATES","in":"2026-10-06T05:11:00.000Z","out":"2026-10-06T06:21:00.000Z"}"#
    );
}

#[test]
fn every_custody_role_and_status_is_printed_in_v2s_word() {
    let roles = [
        (CustodyRole::Owner, "OWNER"),
        (CustodyRole::Front, "FRONT"),
        (CustodyRole::Finance, "FINANCE"),
        (CustodyRole::Monitor, "MONITOR"),
        (CustodyRole::Manager, "MANAGER"),
        (CustodyRole::Backend, "BACKEND"),
    ];
    for (role, word) in roles {
        let state = CustodyState {
            attendance: vec![Attendance {
                id: "A".to_string(),
                employee_id: "E".to_string(),
                role,
                clock_in: custody_at("2026-10-06T05:11:00.000Z"),
                clock_out: None,
            }],
            ..CustodyState::initial()
        };
        let printed = operations_json(&state, &[], &People::new());
        let shown = member(&items(member(&printed, "attendance"))[0], "role").to_value();
        assert_eq!(shown, json!(word));
    }
    for (phase, word) in [
        (Phase::Closed, "CLOSED"),
        (Phase::Ready, "READY"),
        (Phase::Open, "OPEN"),
        (Phase::CutOff, "CUT_OFF"),
        (Phase::Counting, "COUNTING"),
    ] {
        let state = CustodyState {
            phase,
            ..CustodyState::initial()
        };
        let printed = operations_json(&state, &[], &People::new());
        assert_eq!(member(&printed, "phase").to_value(), json!(word));
    }
}

#[test]
fn holds_drafts_and_exceptions_gain_their_keys_once_reviewed_or_saved() {
    let state = CustodyState {
        holds: vec![
            CustomerHold {
                id: "H-1".to_string(),
                customer_id: "CX-00000002".to_string(),
                reason: "asked to wait".to_string(),
                by: "E-MON".to_string(),
                at: custody_at("2026-10-06T06:00:00.000Z"),
                status: HoldStatus::Open,
                reviewed_by: None,
                reviewed_at: None,
                note: None,
            },
            CustomerHold {
                id: "H-2".to_string(),
                customer_id: "CX-00000001".to_string(),
                reason: "passport looks altered".to_string(),
                by: "E-FRONT".to_string(),
                at: custody_at("2026-10-06T05:59:00.000Z"),
                status: HoldStatus::Released,
                reviewed_by: Some("E-MON".to_string()),
                reviewed_at: Some(custody_at("2026-10-06T06:03:00.000Z")),
                note: Some("fine".to_string()),
            },
        ],
        drafts: vec![
            TxDraft {
                id: "D-1".to_string(),
                shift_id: "SH-1".to_string(),
                employee_id: "E-FRONT".to_string(),
                side: Side::Sell,
                currency: code("USD"),
                amount: money("100"),
                at: custody_at("2026-10-06T05:28:00.000Z"),
                status: DraftStatus::Open,
                document_ref: None,
            },
            TxDraft {
                id: "D-2".to_string(),
                shift_id: "SH-1".to_string(),
                employee_id: "E-FRONT".to_string(),
                side: Side::Buy,
                currency: code("EUR"),
                amount: money("250.50"),
                at: custody_at("2026-10-06T05:24:00.000Z"),
                status: DraftStatus::Completed,
                document_ref: Some("XC-20261006-000001".to_string()),
            },
        ],
        exceptions: vec![
            CashException {
                id: "X-1".to_string(),
                request_id: "R-1".to_string(),
                status: ExceptionStatus::Open,
                target_difference: Some(delta(&[("USD", -1000)])),
                cash_variance: None,
                safe_variance: None,
                note: "ten dollars short".to_string(),
                at: custody_at("2026-10-06T05:22:00.000Z"),
                reviewed_by: None,
                reviewed_at: None,
            },
            CashException {
                id: "X-2".to_string(),
                request_id: "R-2".to_string(),
                status: ExceptionStatus::Reviewed,
                target_difference: None,
                cash_variance: Some(delta(&[("THB", -10_000)])),
                safe_variance: Some(delta(&[("EUR", 100)])),
                note: String::new(),
                at: custody_at("2026-10-06T06:19:00.000Z"),
                reviewed_by: Some("E-FINANCE".to_string()),
                reviewed_at: Some(custody_at("2026-10-06T06:20:00.000Z")),
            },
        ],
        cutoff: Some(Cutoff {
            at: custody_at("2026-10-06T06:04:00.000Z"),
            by: "E-BACK".to_string(),
            shift_id: "SH-1".to_string(),
            last_ref: None,
            pending: vec!["XC-20261006-000004".to_string()],
        }),
        ..CustodyState::initial()
    };
    let printed = operations_json(&state, &[], &hand_people());

    assert_eq!(
        member(&printed, "holds").to_text(),
        concat!(
            r#"[{"id":"H-1","customerId":"CX-00000002","reason":"asked to wait","by":"E-MON","at":"2026-10-06T06:00:00.000Z","status":"OPEN"},"#,
            r#"{"id":"H-2","customerId":"CX-00000001","reason":"passport looks altered","by":"E-FRONT","at":"2026-10-06T05:59:00.000Z","status":"RELEASED","#,
            r#""reviewedBy":"E-MON","reviewedAt":"2026-10-06T06:03:00.000Z","note":"fine"}]"#
        )
    );
    assert_eq!(
        member(&printed, "drafts").to_text(),
        concat!(
            r#"[{"id":"D-1","shiftId":"SH-1","employeeId":"E-FRONT","type":"SELL","currency":"USD","amount":100,"at":"2026-10-06T05:28:00.000Z","status":"OPEN"},"#,
            r#"{"id":"D-2","shiftId":"SH-1","employeeId":"E-FRONT","type":"BUY","currency":"EUR","amount":250.5,"at":"2026-10-06T05:24:00.000Z","status":"COMPLETED","documentRef":"XC-20261006-000001"}]"#
        )
    );
    let exceptions = items(member(&printed, "exceptions"));
    assert_eq!(
        keys(&exceptions[0]),
        [
            "id",
            "requestId",
            "status",
            "targetDifference",
            "cashVariance",
            "safeVariance",
            "note",
            "at"
        ]
    );
    assert_eq!(member(&exceptions[0], "status").to_text(), r#""OPEN""#);
    assert_eq!(
        member(member(&exceptions[0], "targetDifference"), "USD").to_text(),
        "-10"
    );
    assert_eq!(member(&exceptions[0], "cashVariance"), &V2Json::Null);
    assert_eq!(keys(&exceptions[1])[8..], ["reviewedBy", "reviewedAt"]);
    assert_eq!(member(&exceptions[1], "status").to_text(), r#""REVIEWED""#);
    assert_eq!(member(&exceptions[1], "targetDifference"), &V2Json::Null);
    assert_eq!(
        member(member(&exceptions[1], "cashVariance"), "THB").to_text(),
        "-100"
    );
    assert_eq!(
        member(member(&exceptions[1], "safeVariance"), "EUR").to_text(),
        "1"
    );
    assert_eq!(
        member(&printed, "cutoff").to_text(),
        r#"{"at":"2026-10-06T06:04:00.000Z","by":"E-BACK","shiftId":"SH-1","lastRef":null,"pending":["XC-20261006-000004"]}"#
    );
}

#[test]
fn events_are_printed_in_the_order_given_with_their_stored_details() {
    let details = json!({"id": "R-1", "kind": "SAFE_OUT", "currency": "USD", "amount": 500});
    let event =
        |id: &str, kind: EventKind, when: &str, actor: &str, role: CustodyRole| StoredEvent {
            id: id.to_string(),
            kind,
            at: at(when),
            actor: actor.to_string(),
            role,
            details: json!({}),
        };
    let events = [
        event(
            "EV-1",
            EventKind::Enable,
            "2026-10-06T05:02:00.000Z",
            "owner",
            CustodyRole::Owner,
        ),
        // a role custody does not know is named from the account, as in `attendance`
        event(
            "EV-2",
            EventKind::ClockIn,
            "2026-10-06T05:11:00.000Z",
            "E-RATES",
            CustodyRole::Other,
        ),
        StoredEvent {
            details: details.clone(),
            ..event(
                "EV-3",
                EventKind::Dispatch,
                "2026-10-06T05:35:00.000Z",
                "E-BACK",
                CustodyRole::Backend,
            )
        },
        event(
            "EV-4",
            EventKind::ClockOut,
            "2026-10-06T06:21:00.123Z",
            "E-GONE",
            CustodyRole::Other,
        ),
    ];
    let printed = operations_json(&CustodyState::initial(), &events, &hand_people());
    assert_eq!(
        member(&printed, "events").to_text(),
        concat!(
            r#"[{"id":"EV-1","type":"ENABLE","at":"2026-10-06T05:02:00.000Z","actor":"owner","role":"OWNER","details":{}},"#,
            r#"{"id":"EV-2","type":"CLOCK_IN","at":"2026-10-06T05:11:00.000Z","actor":"E-RATES","role":"RATES","details":{}},"#,
            r#"{"id":"EV-3","type":"DISPATCH","at":"2026-10-06T05:35:00.000Z","actor":"E-BACK","role":"BACKEND","#,
            r#""details":{"amount":500,"currency":"USD","id":"R-1","kind":"SAFE_OUT"}},"#,
            r#"{"id":"EV-4","type":"CLOCK_OUT","at":"2026-10-06T06:21:00.123Z","actor":"E-GONE","role":"OTHER","details":{}}]"#
        )
    );
    assert_eq!(member(&printed, "events").to_value()[2]["details"], details);
}

#[test]
fn the_safe_the_target_and_the_entity_are_printed_once_they_exist() {
    let state = CustodyState {
        enabled: true,
        phase: Phase::Ready,
        safe: Some(balances(&[
            ("THB", "500000"),
            ("USD", "9800.25"),
            ("EUR", "2001"),
        ])),
        target: Some(CashTarget {
            amounts: balances(&[("THB", "100000"), ("USD", "2000")]),
            by: "E-FINANCE".to_string(),
            at: custody_at("2026-10-06T05:15:00.000Z"),
        }),
        entity: Some(Entity {
            company: "XC Exchange Co., Ltd.".to_string(),
            site: "Riverside B".to_string(),
            confirmed_by: "owner".to_string(),
            at: custody_at("2026-10-06T05:01:00.000Z"),
        }),
        custodian: Some("E-FRONT".to_string()),
        service_opened_at: Some(custody_at("2026-10-06T05:23:00.000Z")),
        ..CustodyState::initial()
    };
    let printed = operations_json(&state, &[], &hand_people());
    assert_eq!(member(&printed, "enabled"), &V2Json::Bool(true));
    assert!(
        member(&printed, "safe")
            .to_text()
            .starts_with(r#"{"THB":500000,"USD":9800.25,"EUR":2001,"GBP":0,"#)
    );
    let target = member(&printed, "target");
    assert_eq!(keys(target), ["amounts", "by", "at"]);
    assert!(
        target
            .to_text()
            .ends_with(r#""ZWG":0},"by":"E-FINANCE","at":"2026-10-06T05:15:00.000Z"}"#)
    );
    assert_eq!(
        member(&printed, "entity").to_text(),
        r#"{"company":"XC Exchange Co., Ltd.","site":"Riverside B","confirmedBy":"owner","at":"2026-10-06T05:01:00.000Z"}"#
    );
    assert_eq!(member(&printed, "custodian").to_text(), r#""E-FRONT""#);
    assert_eq!(keys(&printed).last(), Some(&"serviceOpenedAt"));
    assert!(
        printed
            .to_text()
            .ends_with(r#""drafts":[],"serviceOpenedAt":"2026-10-06T05:23:00.000Z"}"#)
    );
}
