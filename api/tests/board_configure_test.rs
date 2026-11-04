//! BC-12 (`docs/design/board-currencies.md` §3, §7): `configure` sets `branch_currencies.display`
//! (`SHOW`, `ASK`, `HIDE`), guards `SHOW` with `RatesFirst` and `SHOW`/`ASK` with `NotesFirst`,
//! refuses `THB` and callers without `RatesDraft`, links notes on a new row, and records
//! `KioskAudit::CurrencyDisplaySet { group, display, from }` in the same transaction.

#[macro_use]
mod auth_http_common;
mod common;
mod publication_seed;

use auth_http_common::*;
use chrono::Utc;
use publication_seed::{both, publish};
use serde_json::{Value, json};
use xc_kiosk::domain::kiosk::value_objects::kiosk_service_error::KioskServiceError;

const KIOSK: &str = "/api/kiosk";

async fn configure(app: &TestApp, token: &str, payload: Value) -> Answer {
    send(
        app,
        Call::post(
            KIOSK,
            json!({
                "action": "configure",
                "payload": payload,
            }),
        )
        .session(token),
    )
    .await
}

/// Row 1: `SHOW` with no published line -> refused `RatesFirst`.
#[actix_web::test]
async fn show_with_no_published_line_is_refused_rates_first() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("owner").await;

    let answer = configure(
        &app,
        &owner,
        json!({ "branchId": "MAIN", "code": "AED", "group": "ALL", "display": "SHOW" }),
    )
    .await;
    assert_eq!(answer.status, 400, "{}", answer.text);
    assert_eq!(
        answer.json()["error"],
        json!(KioskServiceError::RatesFirst.to_string())
    );
    desk.drop_schema().await;
}

/// Row 2: `ASK` with no notes -> refused `NotesFirst`.
#[actix_web::test]
async fn ask_with_no_notes_is_refused_notes_first() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("owner").await;

    let answer = configure(
        &app,
        &owner,
        json!({ "branchId": "MAIN", "code": "AFN", "group": "ALL", "display": "ASK" }),
    )
    .await;
    assert_eq!(answer.status, 400, "{}", answer.text);
    assert_eq!(
        answer.json()["error"],
        json!(KioskServiceError::NotesFirst.to_string())
    );
    desk.drop_schema().await;
}

/// Row 3: `THB` -> refused, as today.
#[actix_web::test]
async fn thb_is_refused() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("owner").await;

    let answer = configure(
        &app,
        &owner,
        json!({ "branchId": "MAIN", "code": "THB", "group": "ALL", "display": "HIDE" }),
    )
    .await;
    assert_eq!(answer.status, 400, "{}", answer.text);
    assert_eq!(
        answer.json()["error"],
        json!(KioskServiceError::InvalidCurrencySettings.to_string())
    );
    desk.drop_schema().await;
}

/// Row 4: any success -> one audit row with `from` and `display`, same transaction.
#[actix_web::test]
async fn any_success_writes_one_audit_row_with_from_and_display() {
    let desk = Desk::start().await;
    publish(&desk.pool, "MAIN", Utc::now(), &[both("AUD", "21.0", "22.0")]).await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("owner").await;

    let answer = configure(
        &app,
        &owner,
        json!({ "branchId": "MAIN", "code": "AUD", "group": "ALL", "display": "SHOW" }),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.text);

    let (display,): (String,) = sqlx::query_as(
        "SELECT display FROM branch_currencies \
         WHERE branch_id = 'MAIN' AND currency_code = 'AUD' AND group_code = 'ALL'",
    )
    .fetch_one(&desk.pool)
    .await
    .expect("AUD row");
    assert_eq!(display, "SHOW");

    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT ref, detail FROM device_events \
         WHERE branch_id = 'MAIN' AND kind = 'KIOSK_CURRENCY_CONFIGURED'",
    )
    .fetch_all(&desk.pool)
    .await
    .expect("audit rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "AUD");
    assert_eq!(
        rows[0].1,
        json!({ "group": "ALL", "display": "SHOW", "from": "HIDE" })
    );
    desk.drop_schema().await;
}

/// Row 5: user without `RatesDraft` -> refused, as today.
#[actix_web::test]
async fn user_without_rates_draft_is_refused() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let teller = desk.staff_session("teller", "TELLER", "MAIN").await;

    let answer = configure(
        &app,
        &teller,
        json!({ "branchId": "MAIN", "code": "USD", "group": "ALL", "display": "ASK" }),
    )
    .await;
    assert_eq!(answer.status, 400, "{}", answer.text);
    assert_eq!(
        answer.json()["error"],
        json!("Owner or Rates access required.")
    );
    desk.drop_schema().await;
}

/// Row 6: new row -> note links written (only notes of that currency and group that no other group takes).
#[actix_web::test]
async fn new_row_writes_note_links() {
    let desk = Desk::start().await;
    desk.branch("B2").await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("owner").await;

    let answer = configure(
        &app,
        &owner,
        json!({ "branchId": "B2", "code": "AUD", "group": "ALL", "display": "ASK" }),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.text);

    let linked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM branch_currency_notes \
         WHERE branch_id = 'B2' AND currency_code = 'AUD' AND group_code = 'ALL'",
    )
    .fetch_one(&desk.pool)
    .await
    .expect("count linked notes");
    assert_eq!(linked, 5, "AUD's 5 seeded notes (5, 10, 20, 50, 100) are linked");
    desk.drop_schema().await;
}

/// Row 7: seeded row `(KRW, 'KRWS')` with no notes yet, note added to `denominations`, `ASK` accepted and links the note.
#[actix_web::test]
async fn seeded_row_with_note_added_accepts_ask_and_links_note() {
    let desk = Desk::start().await;
    sqlx::query(
        "INSERT INTO denominations (organization_id, currency_code, face_value, group_code) \
         SELECT organization_id, 'KRW', 5000, 'KRWS' FROM branches WHERE id = 'MAIN'",
    )
    .execute(&desk.pool)
    .await
    .expect("insert KRWS note");

    let app = desk_app!(desk);
    let owner = desk.owner_session("owner").await;

    let answer = configure(
        &app,
        &owner,
        json!({ "branchId": "MAIN", "code": "KRW", "group": "KRWS", "display": "ASK" }),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.text);

    let linked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM branch_currency_notes \
         WHERE branch_id = 'MAIN' AND currency_code = 'KRW' AND group_code = 'KRWS'",
    )
    .fetch_one(&desk.pool)
    .await
    .expect("count KRWS linked notes");
    assert_eq!(linked, 1);
    desk.drop_schema().await;
}
