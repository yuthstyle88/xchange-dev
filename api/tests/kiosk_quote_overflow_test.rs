//! The kiosk's quote refuses an amount whose THB → foreign conversion overflows `Decimal`,
//! instead of panicking (tracking #2063, K-a caller migration, `kiosk_machine.rs`).
//!
//! Written before the implementation by the ticket author (claude/d7aa64). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! The overflow is real input, not a test-only value. A published line may hold
//! `unit_size` up to `numeric(20,4)` (9,999,999,999,999,999.9999, migration
//! `20261010100000_rate_units.sql`) and a rate down to `numeric(20,8)`'s 0.00000001, and both
//! parse (`UnitSize::parse` has no upper bound, `Rate::parse` admits 0.00000001). A customer
//! selling by baht (`"type": "SELL"`, amount in THB, at most 10,000,000.00) is converted by
//! `ExchangeArithmetic::foreign_for_thb(thb, rate, units)` = thb × unit_size ÷ rate:
//! 10,000,000 × 9,999,999,999,999,999.9999 ÷ 0.00000001 ≈ 1e31, past `Decimal::MAX` (≈ 7.9e28),
//! so `foreign_for_thb` panics "Division overflowed" (rust_decimal `arithmetic_impls.rs`).
//!
//! Today the panic unwinds out of the actix handler: in this harness the test thread
//! panics; on a server the worker drops the connection without an answer.
//!
//! After the fix (`try_foreign_for_thb` / `try_thb`, overflow → `KioskError::AmountOrCurrencyInvalid`):
//! the route answers 400 with v2's sentence for that error, stores no request, and an ordinary
//! quote on an ordinary line is unchanged.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod compat_common;
mod kiosk_http_common;
mod rpc_common;
mod snapshot_common;

use kiosk_http_common::Counter;
use rpc_common::MAIN;
use serde_json::json;

/// v2's sentence for `KioskError::AmountOrCurrencyInvalid`.
const AMOUNT_OR_CURRENCY_INVALID: &str = "Please enter a valid amount and currency.";

/// The largest `unit_size` `numeric(20,4)` holds.
const WIDEST_UNIT: &str = "9999999999999999.9999";
/// The smallest positive rate `numeric(20,8)` holds.
const SMALLEST_RATE: &str = "0.00000001";

/// Gives `MAIN`'s current dollar line the widest unit and the smallest rate on both sides.
async fn widen_the_dollar(counter: &Counter) {
    let changed = sqlx::query(
        "UPDATE published_rate_lines l SET unit_size = $1::numeric, buy_rate = $2::numeric, \
         sell_rate = $2::numeric \
         FROM branches b WHERE b.id = $3 AND l.publication_id = b.current_publication_id \
         AND l.currency_code = 'USD'",
    )
    .bind(WIDEST_UNIT)
    .bind(SMALLEST_RATE)
    .bind(MAIN)
    .execute(&counter.desk.db.pool)
    .await
    .expect("widen MAIN's dollar line")
    .rows_affected();
    assert_eq!(changed, 1, "MAIN has one current dollar line");
}

async fn requests(counter: &Counter) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kiosk_requests")
        .fetch_one(&counter.desk.db.pool)
        .await
        .expect("count kiosk_requests")
}

/// Row 1: selling 10,000,000.00 THB on the widest line overflows thb × unit ÷ rate.
/// The quote is refused as an invalid amount (400), no panic, nothing stored.
#[actix_web::test]
async fn a_sell_by_baht_that_overflows_is_refused_as_an_invalid_amount() {
    let counter = Counter::open(&[]).await;
    widen_the_dollar(&counter).await;
    let before = requests(&counter).await;

    let answer = counter
        .ask(
            None,
            "quote",
            json!({ "type": "SELL", "currency": "USD", "amount": 10000000 }),
        )
        .await;

    answer.assert_refused(
        400,
        AMOUNT_OR_CURRENCY_INVALID,
        "a SELL whose foreign amount overflows",
    );
    assert_eq!(
        requests(&counter).await,
        before,
        "a refused quote stores no request"
    );
    counter.finish().await;
}

/// Row 2 (keep-pin): an ordinary quote on the ordinary dollar (32.1 / 32.6 per 1) is unchanged.
/// BUY 100 USD at 32.1 = 3,210.00 THB; SELL 3,260 THB at 32.6 = 100.00 USD.
#[actix_web::test]
async fn an_ordinary_quote_is_unchanged() {
    let counter = Counter::open(&[]).await;

    let buy = counter
        .ask(
            None,
            "quote",
            json!({ "type": "BUY", "currency": "USD", "amount": 100 }),
        )
        .await;
    assert_eq!(buy.status, 200, "BUY: {}", buy.text);
    let buy = buy.json();
    let sell = counter
        .ask(
            None,
            "quote",
            json!({ "type": "SELL", "currency": "USD", "amount": 3260 }),
        )
        .await;
    assert_eq!(sell.status, 200, "SELL: {}", sell.text);
    let sell = sell.json();

    assert_eq!(
        (buy["thb"].to_string(), buy["foreign"].to_string()),
        ("3210".to_string(), "100".to_string()),
        "BUY 100 USD at 32.1: {buy}"
    );
    assert_eq!(
        (sell["thb"].to_string(), sell["foreign"].to_string()),
        ("3260".to_string(), "100".to_string()),
        "SELL 3,260 THB at 32.6: {sell}"
    );
    counter.finish().await;
}
