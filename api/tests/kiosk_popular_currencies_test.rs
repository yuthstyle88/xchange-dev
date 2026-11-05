//! The kiosk `rates` answer lists the branch's popular currencies, `popular`, most popular first
//! (owner 2026-11-05: "ให้ ตั้ง ว่า อันไหน นิยม ซื้อขาย ดูจาก ประวัติ การ ซื้อ ขายแล้วให้ เลือก มาแสดง").
//! The rule is in `api/src/application/kiosk_popular.rs`; its unit tests pin the order, the window
//! and the 10-minute cache (one read per branch). These cases run the real query.
//!
//! Needs `TEST_DATABASE_URL` naming a database ending `_test`.

mod common;
mod compat_common;
#[macro_use]
mod kiosk_http_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use chrono::{Duration, NaiveDate};
use kiosk_http_common::{Counter, rates_re4_common};
use serde_json::{Value, json};
use sqlx::{AssertSqlSafe, PgPool};

const MAIN: &str = "MAIN";

async fn exec(pool: &PgPool, sql: &str) {
    sqlx::query(AssertSqlSafe(sql.to_owned()))
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

/// The counter, with USD, EUR, JPY and CNY published (so its kiosk offers all four).
async fn counter() -> Counter {
    let counter = Counter::open(&[]).await;
    rates_re4_common::publish(
        &counter.desk.db.pool,
        MAIN,
        &[
            ("CNY", "4.5", "4.9"),
            ("EUR", "37.1", "37.9"),
            ("JPY", "0.21", "0.23"),
            ("USD", "32.1", "32.6"),
        ],
    )
    .await;
    counter
}

fn bangkok_today(counter: &Counter) -> NaiveDate {
    counter
        .now()
        .with_timezone(&chrono_tz::Asia::Bangkok)
        .date_naive()
}

/// `n` exchanges of `code` at MAIN on `day`, each worth `thb`, with `status` and `mode`.
async fn exchanges(
    counter: &Counter,
    code: &str,
    n: usize,
    day: NaiveDate,
    thb: &str,
    status: &str,
    mode: &str,
) {
    let pool = &counter.desk.db.pool;
    for _ in 0..n {
        let id = uuid::Uuid::new_v4();
        let (doc, completed, cancelled, reason) = match status {
            "COMPLETED" => (format!("'DOC-{id}'"), "now()", "NULL", "NULL"),
            "CANCELLED" => ("NULL".to_owned(), "NULL", "now()", "'test'"),
            _ => ("NULL".to_owned(), "NULL", "NULL", "NULL"),
        };
        let shift = if mode == "LIVE" {
            format!(
                "(SELECT id FROM shifts WHERE branch_id = '{MAIN}' AND status = 'OPEN' LIMIT 1)"
            )
        } else {
            "NULL".to_owned()
        };
        exec(
            pool,
            &format!(
                "WITH c AS (INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
                     SELECT 'CX-{id}', organization_id, 'CX-{id}', 'Customer', 'LIVE', now() FROM branches WHERE id = '{MAIN}' \
                     RETURNING id, organization_id) \
                 INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
                    customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, \
                    recorded_at, recorded_by, status, completed_at, cancelled_at, cancel_reason, organization_id) \
                 SELECT '{id}', {doc}, '{MAIN}', '{mode}', {shift}, 'sub-{id}', 'hash-{id}', c.id, 'Customer', 'THA', \
                    'teller-1', '', '', '{day}'::date, now(), now(), 'teller-1', '{status}', {completed}, {cancelled}, \
                    {reason}, c.organization_id FROM c"
            ),
        )
        .await;
        exec(
            pool,
            &format!(
                "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
                    quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
                 SELECT gen_random_uuid(), organization_id, id, 'BUY', '{code}', 100, 1, 1, {thb}, 0, {thb}, 1, now(), now() \
                 FROM transactions WHERE id = '{id}'"
            ),
        )
        .await;
    }
}

async fn rates(counter: &Counter) -> Value {
    let answer = counter.ask(None, "rates", json!({})).await;
    assert_eq!(answer.status, 200, "rates: {}", answer.text);
    answer.json()
}

fn codes(answer: &Value, key: &str) -> Vec<String> {
    answer[key]
        .as_array()
        .unwrap_or_else(|| panic!("no `{key}` in {answer}"))
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v["code"].as_str().unwrap().to_owned())
        })
        .collect()
}

/// The quotable (SHOW) currencies of the answer, in order.
fn shown(answer: &Value) -> Vec<String> {
    codes(answer, "rates")
}

/// The ASK currencies, printed apart as `ask`, with no prices.
fn asked(answer: &Value) -> Vec<String> {
    codes(answer, "ask")
}

#[actix_web::test]
async fn the_most_traded_currencies_come_first() {
    let counter = counter().await;
    let today = bangkok_today(&counter);
    exchanges(&counter, "USD", 5, today, "3000", "COMPLETED", "LIVE").await;
    exchanges(
        &counter,
        "EUR",
        3,
        today - Duration::days(10),
        "4000",
        "COMPLETED",
        "LIVE",
    )
    .await;
    exchanges(
        &counter,
        "JPY",
        2,
        today - Duration::days(29),
        "900",
        "COMPLETED",
        "LIVE",
    )
    .await;
    // none of these count: older than 30 days, not completed, or a demo
    exchanges(
        &counter,
        "CNY",
        9,
        today - Duration::days(30),
        "100",
        "COMPLETED",
        "LIVE",
    )
    .await;
    exchanges(&counter, "CNY", 9, today, "100", "CANCELLED", "LIVE").await;
    exchanges(&counter, "CNY", 9, today, "100", "COMPLETED", "DEMO").await;

    let answer = rates(&counter).await;
    assert_eq!(codes(&answer, "popular"), ["USD", "EUR", "JPY"], "{answer}");
    // the rates themselves keep the display order
    assert_eq!(shown(&answer), ["USD", "EUR", "JPY", "CNY"], "{answer}");
    counter.finish().await;
}

#[actix_web::test]
async fn equal_counts_go_by_thb() {
    let counter = counter().await;
    let today = bangkok_today(&counter);
    exchanges(&counter, "JPY", 2, today, "500", "COMPLETED", "LIVE").await;
    exchanges(&counter, "EUR", 2, today, "100", "COMPLETED", "LIVE").await;
    let answer = rates(&counter).await;
    assert_eq!(codes(&answer, "popular"), ["JPY", "EUR"], "{answer}");
    counter.finish().await;
}

#[actix_web::test]
async fn a_currency_the_kiosk_does_not_show_is_never_popular() {
    let counter = counter().await;
    let today = bangkok_today(&counter);
    exchanges(&counter, "EUR", 9, today, "9000", "COMPLETED", "LIVE").await;
    exchanges(&counter, "USD", 1, today, "100", "COMPLETED", "LIVE").await;
    exec(
        &counter.desk.db.pool,
        "UPDATE branch_currencies SET display = 'HIDE' WHERE branch_id = 'MAIN' AND currency_code = 'EUR'",
    )
    .await;
    let answer = rates(&counter).await;
    assert_eq!(codes(&answer, "popular"), ["USD"], "{answer}");
    assert!(
        !codes(&answer, "rates").contains(&"EUR".to_owned()),
        "HIDE is absent: {answer}"
    );
    counter.finish().await;
}

#[actix_web::test]
async fn no_history_lists_none_and_the_kiosk_keeps_the_display_order() {
    let counter = counter().await;
    let answer = rates(&counter).await;
    assert_eq!(codes(&answer, "popular"), Vec::<String>::new(), "{answer}");
    assert_eq!(shown(&answer), ["USD", "EUR", "JPY", "CNY"], "{answer}");
    counter.finish().await;
}

#[actix_web::test]
async fn the_branch_screen_values_are_served_and_set_the_window() {
    // owner 2026-11-05: "ว่า อันไหน คอนฟิก ได้ อย่า ฝังใหน code" (application::kiosk_screen)
    let counter = counter().await;
    let answer = rates(&counter).await;
    assert_eq!(answer["screen"]["set"], json!(false), "{answer}");
    assert_eq!(
        answer["screen"]["quickAmounts"],
        json!([100, 500, 1000, 5000])
    );

    let counter = self::counter().await;
    let today = bangkok_today(&counter);
    exchanges(
        &counter,
        "USD",
        3,
        today - Duration::days(10),
        "100",
        "COMPLETED",
        "LIVE",
    )
    .await;
    exchanges(&counter, "EUR", 1, today, "100", "COMPLETED", "LIVE").await;
    exec(
        &counter.desk.db.pool,
        r#"INSERT INTO branch_settings (branch_id, name, version, body, updated_at, updated_by)
           VALUES ('MAIN', 'kioskScreen', 1,
                   '{"quickAmounts": [200, 700], "tiles": 9, "popularDays": 5}', now(), 'owner')"#,
    )
    .await;
    let answer = rates(&counter).await;
    assert_eq!(answer["screen"]["set"], json!(true), "{answer}");
    assert_eq!(answer["screen"]["quickAmounts"], json!([200, 700]));
    assert_eq!(answer["screen"]["tiles"], json!(9));
    assert_eq!(
        answer["screen"]["idleSeconds"],
        json!(120),
        "not set: the default"
    );
    // a 5-day window: the dollar's exchanges 10 days ago do not count
    assert_eq!(codes(&answer, "popular"), ["EUR"], "{answer}");
    counter.finish().await;
}

#[actix_web::test]
async fn ask_currencies_are_listed_to_contact_staff_and_hide_ones_are_absent() {
    // owner 2026-11-05 ("ทำไม สกุลเงินไม่ครบ"): ASK is listed apart, as `ask`, with no prices, never quoted
    // (quote.rs refuses ASK off the desk) and never popular; HIDE is absent
    let counter = counter().await;
    let today = bangkok_today(&counter);
    exchanges(&counter, "EUR", 9, today, "9000", "COMPLETED", "LIVE").await;
    exchanges(&counter, "USD", 1, today, "100", "COMPLETED", "LIVE").await;
    exec(
        &counter.desk.db.pool,
        "UPDATE branch_currencies SET display = 'ASK' WHERE branch_id = 'MAIN' AND currency_code = 'EUR'",
    )
    .await;
    exec(
        &counter.desk.db.pool,
        "UPDATE branch_currencies SET display = 'HIDE' WHERE branch_id = 'MAIN' AND currency_code = 'JPY'",
    )
    .await;
    let answer = rates(&counter).await;
    assert_eq!(shown(&answer), ["USD", "CNY"], "{answer}");
    assert!(asked(&answer).contains(&"EUR".to_owned()), "{answer}");
    assert!(
        !shown(&answer).contains(&"JPY".to_owned()) && !asked(&answer).contains(&"JPY".to_owned()),
        "HIDE is absent: {answer}"
    );
    let eur = answer["ask"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["code"] == json!("EUR"))
        .unwrap();
    assert_eq!(eur["nameEn"], json!("Euro"), "{answer}");
    assert!(
        eur.get("buy").is_none() && eur.get("sell").is_none(),
        "no prices: {eur}"
    );
    assert_eq!(codes(&answer, "popular"), ["USD"], "{answer}");

    // and the kiosk still refuses to quote it
    let quote = counter
        .ask(
            None,
            "quote",
            json!({ "type": "BUY", "currency": "EUR", "amount": 100 }),
        )
        .await;
    assert_ne!(
        quote.status, 200,
        "an ASK currency is quoted: {}",
        quote.text
    );
    counter.finish().await;
}
