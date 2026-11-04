//! Board currencies BC-5, BC-10 and BC-11 (`docs/design/board-currencies.md` §4, §7, D-8). Written before the
//! implementation by the sub-controller (claude/94389a6b). Do not edit, delete or weaken; add tests in a file of
//! your own. Needs `TEST_DATABASE_URL`.

#[macro_use]
mod auth_http_common;
mod common;

use actix_web::http::Method;
use auth_http_common::*;
use serde_json::{Value, json};

const DRAFTABLE: &str = "/api/v3/views/rates_draftable?branch=MAIN";
const BRANCH_CURRENCIES: &str = "/api/v3/views/branch_currencies?branch=MAIN";
const NOTE_ADD: &str = "/api/denomination-notes";

async fn view(app: &TestApp, token: &str, path: &str) -> Value {
    let answer = send(
        app,
        Call {
            method: Some(Method::GET),
            path: path.to_owned(),
            proto: Some("https".to_owned()),
            ..Call::default()
        }
        .session(token),
    )
    .await;
    assert_eq!(answer.status, 200, "{path}: {}", answer.text);
    answer.json()["data"]["rows"].clone()
}

/// `Desk::start` funds every currency into MAIN (`lg4_fund_safe`). These tests choose the stock themselves.
async fn no_stock(desk: &Desk) {
    sqlx::query("DELETE FROM inventory_positions WHERE branch_id = 'MAIN'")
        .execute(&desk.pool)
        .await
        .expect("clear MAIN's stock");
}

fn rows_of(rows: &Value, code: &str) -> Vec<Value> {
    rows.as_array()
        .expect("rows")
        .iter()
        .filter(|r| r["code"] == code)
        .cloned()
        .collect()
}

#[actix_web::test]
async fn bc5_draftable_lists_show_and_ask_rows_with_the_register_unit() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("owner").await;
    sqlx::query("UPDATE branch_currencies SET display = 'SHOW' WHERE branch_id = 'MAIN' AND currency_code = 'KRW' AND group_code = 'ALL'")
        .execute(&desk.pool)
        .await
        .expect("show KRW");
    let rows = view(&app, &owner, DRAFTABLE).await;

    let krw = &rows_of(&rows, "KRW");
    assert!(krw.iter().any(|r| r["group"] == "ALL" && r["unitSize"] == "1000"), "the register's unit: {krw:?}");
    assert!(rows_of(&rows, "TRY").iter().any(|r| r["hasReference"] == false), "ASK, no BoT line");
    assert!(rows_of(&rows, "AED").is_empty(), "HIDE is not draftable");
    assert!(rows_of(&rows, "THB").is_empty(), "THB is never drafted");
}

#[actix_web::test]
async fn bc10_the_set_lists_every_row_by_sort_order() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("owner").await;
    no_stock(&desk).await;
    let rows = view(&app, &owner, BRANCH_CURRENCIES).await;

    assert_eq!(rows.as_array().expect("rows").len(), 45);
    assert_eq!(rows[0]["code"], "THB");
    assert_eq!(rows[1]["code"], "USD");
    assert_eq!(rows_of(&rows, "AED")[0]["display"], "HIDE", "a HIDE row can still be chosen");
    assert_eq!(rows_of(&rows, "AED")[0]["held"], false);
}

#[actix_web::test]
async fn bc10_a_currency_held_with_no_row_is_listed_as_held() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("owner").await;
    no_stock(&desk).await;
    sqlx::query(
        "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb) \
         SELECT organization_id, 'MAIN', 'PLN', 10, 85 FROM branches WHERE id = 'MAIN'",
    )
    .execute(&desk.pool)
    .await
    .expect("hold PLN");
    let rows = view(&app, &owner, BRANCH_CURRENCIES).await;

    let pln = rows_of(&rows, "PLN");
    assert_eq!(pln.len(), 1, "{rows:?}");
    assert_eq!(pln[0]["held"], true);
    assert_eq!(pln[0]["display"], "HIDE");
}

#[actix_web::test]
async fn bc10_a_hidden_currency_with_stock_reads_held() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("owner").await;
    no_stock(&desk).await;
    sqlx::query(
        "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb) \
         SELECT organization_id, 'MAIN', 'AED', 10, 90 FROM branches WHERE id = 'MAIN'",
    )
    .execute(&desk.pool)
    .await
    .expect("hold AED");
    let rows = view(&app, &owner, BRANCH_CURRENCIES).await;
    assert_eq!(rows_of(&rows, "AED")[0]["held"], true);
    assert_eq!(rows_of(&rows, "AED")[0]["display"], "HIDE");
}

async fn add_note(app: &TestApp, token: &str, body: Value) -> Answer {
    send(
        app,
        Call::post(NOTE_ADD, body)
            .session(token)
            .header("idempotency-key", &format!("idem-{}", uuid::Uuid::new_v4())),
    )
    .await
}

async fn notes(desk: &Desk, code: &str, group: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM denominations WHERE currency_code = $1 AND group_code = $2 AND face_value IS NOT NULL",
    )
    .bind(code)
    .bind(group)
    .fetch_one(&desk.pool)
    .await
    .expect("count")
}

#[actix_web::test]
async fn bc11_a_note_is_added_once() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("owner").await;
    let body = json!({ "currency": "KRW", "group": "KRWS", "faceValue": "1000" });

    let first = add_note(&app, &owner, body.clone()).await;
    assert_eq!(first.status, 200, "{}", first.text);
    let second = add_note(&app, &owner, body).await;
    assert_eq!(second.status, 200, "repeating is idempotent: {}", second.text);
    assert_eq!(notes(&desk, "KRW", "KRWS").await, 1);
    assert_eq!(first.json()["data"]["id"], second.json()["data"]["id"]);
}

#[actix_web::test]
async fn bc11_a_bad_face_value_is_refused() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("owner").await;
    for bad in ["0", "-5", "abc", "1.23456"] {
        let answer = add_note(&app, &owner, json!({ "currency": "USD", "group": "ALL", "faceValue": bad })).await;
        assert_eq!(answer.status, 400, "{bad}: {}", answer.text);
    }
}
