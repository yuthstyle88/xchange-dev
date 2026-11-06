//! R6 / F27 (#564): on the kiosk, a customer buying foreign currency (the shop's SELL) may type the
//! foreign amount they want instead of the baht they give.
//!
//! Written before the implementation by the ticket author (claude/33691c06). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Owner ruling R6 on #564, issuecomment-6673699296 (2026-11-06 07:56 ICT): "Add a foreign-amount
//! entry on the kiosk." The symptom (#564 issuecomment-6657891388, F27): a customer who wants
//! USD 300 must type THB 10,209, because on SELL the kiosk takes only a THB amount.
//!
//! Pinned here (the kiosk `quote` of `POST /api/kiosk`):
//! - the quote input takes an optional `amountIn`: `"FOREIGN"` or `"THB"`. Absent (or `null`)
//!   keeps today's meaning: BUY's amount is foreign, SELL's is baht. `"THB"` on SELL is today's
//!   answer, byte for byte;
//! - SELL + `"FOREIGN"`: `foreign` is the typed amount, `thb` is that foreign amount at the
//!   published SELL rate by `ExchangeArithmetic::try_thb` (`amount × rate ÷ unit`, HALF_UP to two
//!   places once, at the end), the same figure the kiosk's BUY side and its SELL-by-baht branch
//!   already charge for a foreign quantity (`kiosk_machine.rs`). `inputAmount` is the typed amount.
//!   The answer has exactly today's keys, in today's order;
//! - the 10,000,000.00 cap is on the typed amount, as today;
//! - BUY + `"THB"` and any other value of `amountIn` (another string, a number, lower case) are
//!   refused 400 with `KioskError::AmountOrCurrencyInvalid`'s sentence, as an unreadable `amount`
//!   is refused today, and store nothing;
//! - a confirmed FOREIGN quote keeps the dollars the customer asked for.
//!
//! The harness is `kiosk_http_common::Counter`: `MAIN` with an open shift, an open queue and the
//! dollar published at 32.1 (we buy) / 32.6 (we sell) per 1. `MAIN` is opened all day
//! (`xc_testdb::open_all_day`) so the R2 opening-hours rule (#564) cannot close the kiosk here.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod kiosk_http_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use chrono::Duration;
use kiosk_http_common::Counter;
use rpc_common::{Answer, MAIN};
use rust_decimal::Decimal;
use serde_json::{Value, json};

/// `KioskError::AmountOrCurrencyInvalid`'s sentence.
const AMOUNT_OR_CURRENCY_INVALID: &str = "Please enter a valid amount and currency.";

/// The keys of today's quote answer, in order (`the_answers_keep_v2s_key_order`).
const QUOTE_KEYS: [&str; 13] = [
    "id",
    "queue",
    "orderNo",
    "transactionNo",
    "status",
    "paymentMethod",
    "type",
    "currency",
    "rate",
    "foreign",
    "thb",
    "inputAmount",
    "branchName",
];

async fn open() -> Counter {
    let fr1 = kiosk_http_common::person("FR1", "FRONT", 0x11);
    let counter = Counter::open(std::slice::from_ref(&fr1)).await;
    xc_testdb::open_all_day(&counter.desk.db.pool, MAIN)
        .await
        .expect("open MAIN all day");
    counter
}

async fn quote(counter: &Counter, id: &str, payload: Value) -> Answer {
    counter.hands.ids.push(id);
    counter.ask(None, "quote", payload).await
}

fn quoted(answer: &Answer, label: &str) -> Value {
    assert_eq!(answer.status, 200, "{label}: {}", answer.text);
    let body = answer.json();
    let count = body
        .as_object()
        .unwrap_or_else(|| panic!("{label}: an object: {}", answer.text))
        .len();
    assert_eq!(
        count,
        QUOTE_KEYS.len(),
        "{label}: today's keys only: {}",
        answer.text
    );
    // `Value` sorts its keys, so the order is read from the bytes.
    let at: Vec<Option<usize>> = QUOTE_KEYS
        .iter()
        .map(|key| answer.text.find(&format!("\"{key}\":")))
        .collect();
    assert!(
        at.iter().all(Option::is_some) && at.windows(2).all(|w| w[0] < w[1]),
        "{label}: today's keys in today's order: {}",
        answer.text
    );
    body
}

async fn requests(counter: &Counter) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kiosk_requests")
        .fetch_one(&counter.desk.db.pool)
        .await
        .expect("count kiosk_requests")
}

/// The headline: USD 300 asked for in dollars costs 300 × 32.6 = 9,780.00 THB.
#[actix_web::test]
async fn f27_sell_foreign_300_dollars_costs_9780_baht() {
    let counter = open().await;
    let answer = quote(
        &counter,
        "00000000-0000-4000-8000-0000000f2701",
        json!({ "type": "SELL", "currency": "USD", "amount": 300, "amountIn": "FOREIGN" }),
    )
    .await;
    quoted(&answer, "SELL FOREIGN 300");
    assert_eq!(
        answer.text,
        "{\"id\":\"00000000-0000-4000-8000-0000000f2701\",\"queue\":\"K-00000000\",\
         \"orderNo\":\"ORD-00000000-0000-4000-8000-0000000F2701\",\"transactionNo\":null,\
         \"status\":\"QUOTED\",\"paymentMethod\":\"Cash\",\"type\":\"SELL\",\"currency\":\"USD\",\
         \"rate\":32.6,\"foreign\":300,\"thb\":9780,\"inputAmount\":300,\"branchName\":\"XC-B\"}",
        "the dollars typed are the dollars quoted, and the baht is what the shop charges for them"
    );
    counter.finish().await;
}

/// The amount may be a string, as the page sends it (`$('amount').value`).
#[actix_web::test]
async fn f27_sell_foreign_takes_the_amount_as_a_string() {
    let counter = open().await;
    let answer = quote(
        &counter,
        "00000000-0000-4000-8000-0000000f2702",
        json!({ "type": "SELL", "currency": "USD", "amount": "300", "amountIn": "FOREIGN",
                "paymentMethod": "Cash" }),
    )
    .await;
    let body = quoted(&answer, "SELL FOREIGN \"300\"");
    assert_eq!(
        (&body["foreign"], &body["thb"], &body["inputAmount"]),
        (&json!(300), &json!(9780), &json!(300)),
        "{}",
        answer.text
    );
    counter.finish().await;
}

/// `thb` is `try_thb`'s figure: rounded once to the satang, not truncated. 0.33 × 32.6 = 10.758
/// is 10.76 (truncating would charge 10.75).
#[actix_web::test]
async fn f27_sell_foreign_rounds_the_baht_like_try_thb() {
    let counter = open().await;
    let answer = quote(
        &counter,
        "00000000-0000-4000-8000-0000000f2703",
        json!({ "type": "SELL", "currency": "USD", "amount": "0.33", "amountIn": "FOREIGN" }),
    )
    .await;
    let body = quoted(&answer, "SELL FOREIGN 0.33");
    assert_eq!(
        (&body["foreign"], &body["thb"], &body["inputAmount"]),
        (&json!(0.33), &json!(10.76), &json!(0.33)),
        "{}",
        answer.text
    );
    counter.finish().await;
}

/// One cent of a dollar is quotable in dollars (0.326 THB → 0.33), though today's SELL of
/// 0.01 THB buys no cent and is refused as too small.
#[actix_web::test]
async fn f27_sell_foreign_one_cent_is_quoted() {
    let counter = open().await;
    let answer = quote(
        &counter,
        "00000000-0000-4000-8000-0000000f2704",
        json!({ "type": "SELL", "currency": "USD", "amount": "0.01", "amountIn": "FOREIGN" }),
    )
    .await;
    let body = quoted(&answer, "SELL FOREIGN 0.01");
    assert_eq!(
        (&body["foreign"], &body["thb"]),
        (&json!(0.01), &json!(0.33)),
        "{}",
        answer.text
    );
    counter.finish().await;
}

/// Without `amountIn`, with `"THB"`, and with `null`, SELL's amount is baht, as today:
/// 9,780 THB buys 9780 ÷ 32.6 = 300 dollars. The three answers are the same bytes but the id.
#[actix_web::test]
async fn f27_sell_without_foreign_is_todays_answer() {
    let counter = open().await;
    let mut answers = Vec::new();
    for (n, amount_in) in [None, Some(json!("THB")), Some(Value::Null)]
        .into_iter()
        .enumerate()
    {
        let mut payload = json!({ "type": "SELL", "currency": "USD", "amount": 9780 });
        if let Some(value) = amount_in.clone() {
            payload["amountIn"] = value;
        }
        let id = format!("00000000-0000-4000-8000-0000000f271{n}");
        let answer = quote(&counter, &id, payload).await;
        let body = quoted(&answer, &format!("SELL amountIn {amount_in:?}"));
        assert_eq!(
            (&body["foreign"], &body["thb"], &body["inputAmount"]),
            (&json!(300), &json!(9780), &json!(9780)),
            "amountIn {amount_in:?}: {}",
            answer.text
        );
        answers.push(
            answer
                .text
                .replace(&id, "ID")
                .replace(&id.to_uppercase(), "ID"),
        );
    }
    assert_eq!(answers[0], answers[1], "\"THB\" is today's answer");
    assert_eq!(answers[0], answers[2], "null is today's answer");
    counter.finish().await;
}

/// BUY's amount is already foreign: `"FOREIGN"` is today's BUY answer.
#[actix_web::test]
async fn f27_buy_foreign_is_todays_buy() {
    let counter = open().await;
    let plain = quote(
        &counter,
        "00000000-0000-4000-8000-0000000f2720",
        json!({ "type": "BUY", "currency": "USD", "amount": 250 }),
    )
    .await;
    let foreign = quote(
        &counter,
        "00000000-0000-4000-8000-0000000f2721",
        json!({ "type": "BUY", "currency": "USD", "amount": 250, "amountIn": "FOREIGN" }),
    )
    .await;
    let (plain, foreign) = (quoted(&plain, "BUY"), quoted(&foreign, "BUY FOREIGN"));
    for key in [
        "type",
        "rate",
        "foreign",
        "thb",
        "inputAmount",
        "paymentMethod",
    ] {
        assert_eq!(plain[key], foreign[key], "{key}");
    }
    assert_eq!(
        (&foreign["foreign"], &foreign["thb"]),
        (&json!(250), &json!(8025))
    );
    counter.finish().await;
}

/// BUY in baht is not offered (the page never sends it): refused, nothing stored.
#[actix_web::test]
async fn f27_buy_in_thb_is_refused() {
    let counter = open().await;
    let answer = quote(
        &counter,
        "00000000-0000-4000-8000-0000000f2730",
        json!({ "type": "BUY", "currency": "USD", "amount": 250, "amountIn": "THB" }),
    )
    .await;
    answer.assert_refused(400, AMOUNT_OR_CURRENCY_INVALID, "BUY amountIn THB");
    assert_eq!(
        requests(&counter).await,
        0,
        "a refused quote stores nothing"
    );
    counter.finish().await;
}

/// Any other `amountIn` is refused, on either side, and stores nothing.
#[actix_web::test]
async fn f27_an_unknown_amount_in_is_refused() {
    let counter = open().await;
    for (n, (side, value)) in [
        ("SELL", json!("foreign")),
        ("SELL", json!("USD")),
        ("SELL", json!("")),
        ("SELL", json!(1)),
        ("SELL", json!(true)),
        ("SELL", json!(["FOREIGN"])),
        ("BUY", json!("BAHT")),
    ]
    .into_iter()
    .enumerate()
    {
        let answer = quote(
            &counter,
            &format!("00000000-0000-4000-8000-0000000f274{n}"),
            json!({ "type": side, "currency": "USD", "amount": 300, "amountIn": value }),
        )
        .await;
        answer.assert_refused(
            400,
            AMOUNT_OR_CURRENCY_INVALID,
            &format!("{side} amountIn {value}"),
        );
    }
    assert_eq!(
        requests(&counter).await,
        0,
        "a refused quote stores nothing"
    );
    counter.finish().await;
}

/// The cap is on the typed amount: 10,000,000.00 dollars is the most, one cent more is refused.
#[actix_web::test]
async fn f27_sell_foreign_cap_is_on_the_typed_amount() {
    let counter = open().await;
    let over = quote(
        &counter,
        "00000000-0000-4000-8000-0000000f2750",
        json!({ "type": "SELL", "currency": "USD", "amount": "10000000.01", "amountIn": "FOREIGN" }),
    )
    .await;
    over.assert_refused(400, AMOUNT_OR_CURRENCY_INVALID, "past the cap");
    let at_cap = quote(
        &counter,
        "00000000-0000-4000-8000-0000000f2751",
        json!({ "type": "SELL", "currency": "USD", "amount": "10000000", "amountIn": "FOREIGN" }),
    )
    .await;
    let body = quoted(&at_cap, "at the cap");
    assert_eq!(
        (&body["foreign"], &body["thb"], &body["inputAmount"]),
        (&json!(10_000_000), &json!(326_000_000), &json!(10_000_000)),
        "{}",
        at_cap.text
    );
    counter.finish().await;
}

/// A confirmed FOREIGN quote waits at the desk with the dollars asked for, and stores the typed
/// amount as `input_amount` (the amount typed, in the currency of `amountIn`).
#[actix_web::test]
async fn f27_a_confirmed_foreign_quote_keeps_300_dollars() {
    let counter = open().await;
    let id = "00000000-0000-4000-8000-0000000f2760";
    let answer = quote(
        &counter,
        id,
        json!({ "type": "SELL", "currency": "USD", "amount": 300, "amountIn": "FOREIGN" }),
    )
    .await;
    quoted(&answer, "SELL FOREIGN 300");
    counter.desk.clock.set(counter.now() + Duration::seconds(4));
    let confirmed = counter
        .ask(Some("FR1"), "confirm", json!({ "id": id }))
        .await;
    assert_eq!(confirmed.status, 200, "confirm: {}", confirmed.text);
    let stored: (String, String, Decimal, Decimal, Decimal) = sqlx::query_as(
        "SELECT status, side, foreign_amount, thb, input_amount FROM kiosk_requests WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&counter.desk.db.pool)
    .await
    .expect("the confirmed request");
    assert_eq!(
        stored,
        (
            "WAITING".to_string(),
            "SELL".to_string(),
            Decimal::new(30000, 2),
            Decimal::new(978000, 2),
            Decimal::new(30000, 2),
        ),
        "the desk serves USD 300 for 9,780.00 THB"
    );
    counter.finish().await;
}
