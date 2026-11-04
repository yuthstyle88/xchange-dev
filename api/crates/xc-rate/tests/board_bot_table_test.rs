//! Board currencies BC-1 and BC-2 (`docs/design/board-currencies.md` §5): BoT keeps every registered row.
//! Written before the implementation by the sub-controller (claude/94389a6b). Do not edit, delete or weaken;
//! add tests in a file of your own. `fixtures/bot_live_2026_11_03_detail.json` is BoT's real table, 48 rows.

use pos_error::AppError;
use serde_json::{Value, json};
use xc_rate::domain::rate::entities::rate_source::ReferenceRates;
use xc_rate::domain::reference::reference_provider::ReferenceProvider;
use xc_rate::infrastructure::rate::bot_rate_source::BOT_UNAVAILABLE;
use xc_rate::infrastructure::reference::bot_provider::BotProvider;
use xc_rate::infrastructure::reference_source::bot_rows::bot_rows;
use xc_rate::infrastructure::reference_source::bot_table::bot_unit;

fn live() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/bot_live_2026_11_03_detail.json"))
        .expect("the fixture parses")
}

fn row<'a>(rows: &'a [Value], code: &str) -> &'a Value {
    rows.iter()
        .find(|r| r["code"] == code)
        .unwrap_or_else(|| panic!("no {code} row"))
}

fn edited(code: &str, field: &str, value: Value) -> Vec<Value> {
    let mut rows = live();
    let target = rows
        .iter_mut()
        .find(|r| r["currency_id"] == code)
        .unwrap_or_else(|| panic!("no {code} in the fixture"));
    target[field] = value;
    rows
}

fn assert_refused(result: pos_error::AppResult<Vec<Value>>) {
    match result {
        Err(AppError::Domain(s)) => assert_eq!(s, BOT_UNAVAILABLE),
        other => panic!("expected the BoT refusal, got {other:?}"),
    }
}

#[test]
fn bot_unit_is_the_design_table() {
    for (code, unit) in [
        ("JPY", 100),
        ("IDR", 1000),
        ("VND", 100),
        ("LAK", 100),
        ("KHR", 100),
        ("KRW", 1),
        ("USD", 1),
    ] {
        assert_eq!(bot_unit(code), unit, "{code}");
    }
}

#[test]
fn bc1_the_real_table_keeps_all_48_rows() {
    assert_eq!(bot_rows(&live()).expect("accepted").len(), 48);
}

#[test]
fn bc1_each_row_carries_the_unit_of_the_design_table() {
    let rows = bot_rows(&live()).expect("accepted");
    for (code, unit) in [
        ("USD", 1),
        ("JPY", 100),
        ("IDR", 1000),
        ("VND", 100),
        ("LAK", 100),
        ("KRW", 1),
    ] {
        assert_eq!(row(&rows, code)["unit"], json!(unit), "{code}");
    }
}

#[test]
fn bc1_a_dash_on_a_non_sentinel_side_is_null_and_original_keeps_it() {
    let rows = bot_rows(&live()).expect("accepted");
    let krw = row(&rows, "KRW");
    assert_eq!(krw["buyingSight"], Value::Null);
    assert_eq!(krw["original"]["buyingSight"], json!("-"));
    assert_eq!(krw["buyingTransfer"], json!(0.0249));
}

#[test]
fn bc1_a_dash_on_a_non_sentinel_transfer_side_is_null_and_the_table_stands() {
    let rows = bot_rows(&edited("SGD", "buying_transfer", json!("-"))).expect("accepted");
    assert_eq!(row(&rows, "SGD")["buyingTransfer"], Value::Null);
    assert_eq!(rows.len(), 48);
}

#[test]
fn bc1_a_dash_on_a_sentinel_refuses_the_table() {
    assert_refused(bot_rows(&edited("USD", "buying_sight", json!("-"))));
}

#[test]
fn bc1_a_bad_date_on_a_sentinel_refuses_the_table() {
    assert_refused(bot_rows(&edited("EUR", "period", json!("03-11-2026"))));
}

#[test]
fn bc1_a_bad_date_on_a_non_sentinel_drops_that_row_only() {
    let rows = bot_rows(&edited("SGD", "period", json!("03-11-2026"))).expect("accepted");
    assert_eq!(rows.len(), 47);
    assert!(rows.iter().all(|r| r["code"] != "SGD"));
}

#[test]
fn bc1_a_code_outside_the_register_is_dropped() {
    let rows = bot_rows(&edited("SGD", "currency_id", json!("XAU"))).expect("accepted");
    assert_eq!(rows.len(), 47);
    assert!(rows.iter().all(|r| r["code"] != "XAU"));
}

#[test]
fn bc1_a_table_missing_a_sentinel_is_refused() {
    let rows: Vec<Value> = live()
        .into_iter()
        .filter(|r| r["currency_id"] != "AUD")
        .collect();
    assert_refused(bot_rows(&rows));
}

#[test]
fn bc1_a_table_missing_a_non_sentinel_is_accepted() {
    let rows: Vec<Value> = live()
        .into_iter()
        .filter(|r| r["currency_id"] != "SGD")
        .collect();
    assert_eq!(bot_rows(&rows).expect("accepted").len(), 47);
}

#[test]
fn bc1_a_sentinel_twice_is_refused() {
    let mut rows = live();
    let usd = rows
        .iter()
        .find(|r| r["currency_id"] == "USD")
        .cloned()
        .expect("USD");
    rows.push(usd);
    assert_refused(bot_rows(&rows));
}

fn rates(rows: Vec<Value>) -> ReferenceRates {
    ReferenceRates {
        source: "test".to_owned(),
        fetched_at: "2026-11-04T00:00:00.000Z".to_owned(),
        kind: "DAILY_COMMERCIAL_BANK_AVERAGE".to_owned(),
        rows,
    }
}

fn row_of(code: &str) -> Value {
    json!({ "code": code, "date": "2026-11-03", "unit": 1, "buyingTransfer": 1.5, "selling": 1.6 })
}

#[test]
fn bc2_a_batch_with_every_sentinel_is_complete_whatever_else_is_missing() {
    let rows = ["USD", "EUR", "GBP", "JPY", "CNY", "AUD"]
        .map(row_of)
        .to_vec();
    assert!(BotProvider.parse(&rates(rows)).expect("parsed").complete);
}

#[test]
fn bc2_a_batch_missing_a_sentinel_is_not_complete() {
    let rows = ["USD", "EUR", "GBP", "JPY", "CNY", "SGD"]
        .map(row_of)
        .to_vec();
    assert!(!BotProvider.parse(&rates(rows)).expect("parsed").complete);
}
