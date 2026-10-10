//! Ticket #675 (TX-7) — the kiosk quotes through TX-1's `QuoteService`: every stored kiosk request
//! names a `quotes` row (`channel KIOSK`, no customer) whose one line carries the request's figures.
//! Written before the implementation by the ticket author for the TX chain (claude/f03345fe, on the
//! lane controller's order). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` D-8 (one quote service for every channel),
//! §3.1 (the `quotes`/`quote_lines` row: `ACTIVE`, `INDICATIVE`, the branch's current publication,
//! `expires_at = now + quote_ttl_seconds`, `base_rate = final_rate` = the published rate, fee 0
//! here, `settle_thb` = `thb_amount`), §4.6 (the kiosk asks `QuoteService` through its port, with
//! `channel KIOSK` and no customer, and stores `kiosk_requests.quote_id`; its 300 s and 3 s rules
//! stay), §8 item 4 (`kiosk_requests.quote_id uuid NULL REFERENCES quotes(id)`; old rows keep NULL).
//!
//! What the kiosk keeps, ruled by the ticket author (#675 body, 2026-10-10):
//! - a SELL kiosk quote is entered in THB. The kiosk still turns it into the foreign quantity
//!   (`KioskMachine::quote`, `foreign_for_thb`, cut to 2 places), because `QuoteService` quotes a
//!   foreign quantity only. It then asks the service for that quantity;
//! - the request's own `thb` is the quote line's `thb_amount` (one arithmetic, `ExchangeArithmetic`);
//! - a refusal writes neither a request nor a quote.
//!
//! Every request goes through the server's route table (`POST /api/kiosk`) with
//! `kiosk_http_common::Counter` (`MAIN` with an open shift and its dollar published at BUY 32.1 /
//! SELL 32.6 per 1), as `stale_policy_kiosk_test.rs` drives it.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.
//!
//! Signatures this test assumes ([required]): the column `kiosk_requests.quote_id` (uuid, NULL,
//! FK `quotes(id)`); TX-1's `quotes`/`quote_lines` columns (merged). Nothing else: the port's
//! method and the repository's are the implementer's within ticket §7.

mod common;
mod compat_common;
mod kiosk_http_common;
mod rpc_common;
mod snapshot_common;

use kiosk_http_common::Counter;
use serde_json::{Value, json};

const MAIN: &str = "MAIN";

async fn count(counter: &Counter, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(&counter.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

/// One text per column of the one row `sql` selects (every column cast to text; NULL as "NULL").
async fn row(counter: &Counter, cols: &str, from: &str) -> Vec<String> {
    let sql = format!("SELECT ARRAY[{cols}]::text[] {from}");
    let rows: Vec<Vec<Option<String>>> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.clone()))
        .fetch_all(&counter.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    assert_eq!(rows.len(), 1, "exactly one row: {sql}");
    rows.into_iter()
        .next()
        .unwrap()
        .into_iter()
        .map(|v| v.unwrap_or_else(|| "NULL".to_owned()))
        .collect()
}

/// A kiosk quote with the id `id`, answered 200.
async fn quote(counter: &Counter, id: &str, side: &str, amount: Value) {
    counter.hands.ids.push(id);
    let answer = counter
        .ask(
            None,
            "quote",
            json!({ "type": side, "currency": "USD", "amount": amount }),
        )
        .await;
    assert_eq!(answer.status, 200, "{side} {amount}: {}", answer.text);
    assert_eq!(answer.json()["id"], json!(id));
}

/// The quote a request names, and its line, side by side with the request.
async fn check_linked(counter: &Counter, id: &str, side: &str) {
    assert_eq!(
        row(
            counter,
            "(q.id IS NOT NULL)::text, q.channel, coalesce(q.customer_id, 'NULL'), q.status, q.type, \
             (q.publication_id = b.current_publication_id)::text",
            &format!(
                "FROM kiosk_requests k LEFT JOIN quotes q ON q.id = k.quote_id \
                 JOIN branches b ON b.id = k.branch_id \
                 WHERE k.branch_id = '{MAIN}' AND k.id = '{id}'"
            ),
        )
        .await,
        ["true", "KIOSK", "NULL", "ACTIVE", "INDICATIVE", "true"],
        "{id}: the request names a KIOSK quote with no customer, on the current publication"
    );
    assert_eq!(
        row(
            counter,
            "l.shop_side, l.currency::text, (l.quantity = k.foreign_amount)::text, \
             (l.thb_amount = k.thb)::text, l.fee_thb::text, (l.settle_thb = l.thb_amount)::text, \
             (l.published_rate_line_id = k.published_rate_line_id)::text, \
             (l.base_rate = l.final_rate)::text, (l.unit_size = k.unit_size)::text, \
             l.line_no::text, (SELECT count(*) FROM quote_lines x WHERE x.quote_id = k.quote_id)::text",
            &format!(
                "FROM kiosk_requests k JOIN quote_lines l ON l.quote_id = k.quote_id \
                 WHERE k.branch_id = '{MAIN}' AND k.id = '{id}'"
            ),
        )
        .await,
        [side, "USD", "true", "true", "0.00", "true", "true", "true", "true", "1", "1"],
        "{id}: one line with the request's quantity, THB, published line and unit size; no fee"
    );
}

#[actix_web::test]
async fn a_kiosk_buy_quote_is_a_kiosk_quote_with_the_requests_figures() {
    let counter = Counter::open(&[]).await;
    quote(&counter, "K-BUY-1", "BUY", json!(100)).await;
    check_linked(&counter, "K-BUY-1", "BUY").await;
    // BUY 100 USD at 32.1 per 1 is 3,210.00 THB (the kiosk's answer before this ticket too)
    assert_eq!(
        row(
            &counter,
            "k.thb::text, l.quantity::text, l.base_rate::text",
            "FROM kiosk_requests k JOIN quote_lines l ON l.quote_id = k.quote_id \
                 WHERE k.id = 'K-BUY-1'",
        )
        .await,
        ["3210.00", "100.0000", "32.10000000"]
    );
    counter.finish().await;
}

#[actix_web::test]
async fn a_kiosk_sell_quote_entered_in_baht_quotes_the_foreign_quantity_the_kiosk_derives() {
    let counter = Counter::open(&[]).await;
    // 1,000 THB at 32.6 per 1 is 30.674… USD, cut to 30.67; 30.67 × 32.6 = 999.84
    quote(&counter, "K-SELL-1", "SELL", json!(1000)).await;
    check_linked(&counter, "K-SELL-1", "SELL").await;
    assert_eq!(
        row(
            &counter,
            "k.foreign_amount::text, k.thb::text, l.quantity::text, l.thb_amount::text",
            "FROM kiosk_requests k JOIN quote_lines l ON l.quote_id = k.quote_id \
                 WHERE k.id = 'K-SELL-1'",
        )
        .await,
        ["30.67", "999.84", "30.6700", "999.84"]
    );
    counter.finish().await;
}

#[actix_web::test]
async fn a_kiosk_quote_lives_as_long_as_the_organisations_setting_says() {
    let counter = Counter::open(&[]).await;
    quote(&counter, "K-TTL-300", "BUY", json!(10)).await;
    sqlx::query("UPDATE exchange_settings SET quote_ttl_seconds = 600")
        .execute(&counter.desk.db.pool)
        .await
        .expect("set the organisation's quote lifetime");
    quote(&counter, "K-TTL-600", "BUY", json!(10)).await;
    for (id, seconds) in [("K-TTL-300", "300"), ("K-TTL-600", "600")] {
        assert_eq!(
            row(
                &counter,
                "extract(epoch FROM q.expires_at - q.created_at)::bigint::text",
                &format!(
                    "FROM kiosk_requests k JOIN quotes q ON q.id = k.quote_id WHERE k.id = '{id}'"
                ),
            )
            .await,
            [seconds],
            "{id}: expires_at = now + quote_ttl_seconds (T1)"
        );
    }
    counter.finish().await;
}

#[actix_web::test]
async fn a_refused_kiosk_quote_writes_neither_a_request_nor_a_quote() {
    let counter = Counter::open(&[]).await;
    for (what, payload) in [
        (
            "an amount of 0",
            json!({ "type": "BUY", "currency": "USD", "amount": 0 }),
        ),
        (
            "a currency nobody published",
            json!({ "type": "BUY", "currency": "EUR", "amount": 10 }),
        ),
        ("no direction", json!({ "currency": "USD", "amount": 10 })),
    ] {
        let answer = counter.ask(None, "quote", payload).await;
        assert_eq!(answer.status, 400, "{what}: {}", answer.text);
    }
    assert_eq!(
        count(&counter, "SELECT count(*) FROM kiosk_requests").await,
        0
    );
    assert_eq!(
        count(&counter, "SELECT count(*) FROM quotes").await,
        0,
        "no quote is left behind by a refused kiosk quote"
    );
    counter.finish().await;
}

#[actix_web::test]
async fn each_kiosk_quote_has_its_own_quote_and_old_requests_keep_none() {
    let counter = Counter::open(&[]).await;
    quote(&counter, "K-A", "BUY", json!(10)).await;
    quote(&counter, "K-B", "BUY", json!(10)).await;
    assert_eq!(
        count(
            &counter,
            "SELECT count(DISTINCT quote_id) FROM kiosk_requests WHERE quote_id IS NOT NULL"
        )
        .await,
        2,
        "two requests, two quotes"
    );
    // migration 4 (§8 item 4): the column is NULL-able, so a request written before it stays valid
    assert_eq!(
        row(
            &counter,
            "is_nullable::text",
            "FROM information_schema.columns WHERE table_schema = current_schema() \
             AND table_name = 'kiosk_requests' AND column_name = 'quote_id'",
        )
        .await,
        ["YES"]
    );
    counter.finish().await;
}
