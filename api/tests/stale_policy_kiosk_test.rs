//! Ticket #769 — the kiosk quote judges a stale line by the policy it was published under, read
//! through `PublishedRates::stale_policy_in`; a stale line whose policy cannot be found is a
//! server error and writes nothing.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.8 "Stale at read time" (the policy version the
//! line was published under; `StalePolicy::admits`: STOP refuses, LIMIT refuses a THB amount
//! above `stale_limit_thb`, NULL admits), §4.9 R5 (*a stale line whose policy cannot be found must
//! be an error*; a refusal is the reader's existing "no rate for this currency" answer). Ticket
//! #769 §7 (the kiosk port answers `false`, so the existing "no rate" answer), §9.
//!
//! Every request goes through the server's route table (`POST /api/kiosk`) with the harness of
//! `kiosk_http_common` (`Counter`: `MAIN` with an open shift and its dollar published), as
//! `rates_re4_kiosk_test.rs` drives it. The kiosk answers an `AppError` the service does not map
//! with 500 (`presentation/http/kiosk.rs`, `kiosk_server_error`). Stale lines are seeded with SQL
//! by `stale_policy_769_common` (why: see its header). "Writes nothing" is no `kiosk_requests` row
//! and no audit line.
//!
//! Already pinned before this ticket, not repeated here (`rates_re4_kiosk_test.rs`):
//! `a_stale_line_under_stop_gets_the_no_rate_answer` (STOP: 400 "no rate", nothing written),
//! `a_stale_line_under_limit_is_quoted_only_up_to_the_limit` (995.10 quoted, 1,027.20 refused
//! under 1,000.00; the exact limit is pinned below).
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.
//!
//! Signatures this test assumes:
//! - [required] `PublishedRates::stale_policy_in(tx, line: &PublishedSide) ->
//!   AppResult<StalePolicy>` (ticket #769 §7), which `PgKioskPorts::check_stale_in` calls for a
//!   stale line; this file pins its effect through the route;
//! - [required] `KioskPorts::check_stale_in` has no default body (#769 §3; pinned by
//!   `kiosk_check_stale_has_no_default_test.rs`);
//! - the kiosk's `quote` payload and answer as `rates_re4_kiosk_test.rs` names them (RE-4b,
//!   merged); `kiosk_requests.published_rate_line_id` and `thb`; the refusal sentence is today's
//!   `KioskError::AmountOrCurrencyInvalid` text.

mod common;
mod compat_common;
mod kiosk_http_common;
mod rpc_common;
mod snapshot_common;
mod stale_policy_769_common;

use kiosk_http_common::{Counter, audit_since};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use stale_policy_769_common::{
    Names, id_of, policy, policy_rows, publish_auto, reference_age, unfindable,
};
use std::str::FromStr;

const MAIN: &str = "MAIN";
const AMOUNT_OR_CURRENCY: &str = "Please enter a valid amount and currency.";
/// USD per 1: BUY 32.1, SELL 32.6 (the counter's dollar).
const USD: (&str, &str, &str, &str) = ("USD", "1", "32.1", "32.6");

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

async fn requests(counter: &Counter) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kiosk_requests")
        .fetch_one(&counter.desk.db.pool)
        .await
        .expect("count kiosk_requests")
}

/// The last audit line's sequence number now.
async fn audit_mark(counter: &Counter) -> i64 {
    audit_since(&counter.desk, 0).await.1
}

/// `(published_rate_line_id, thb)` of one request.
async fn stored(counter: &Counter, id: &str) -> (Option<String>, Decimal) {
    sqlx::query_as(
        "SELECT published_rate_line_id::text, thb FROM kiosk_requests \
         WHERE branch_id = 'MAIN' AND id = $1",
    )
    .bind(id)
    .fetch_one(&counter.desk.db.pool)
    .await
    .unwrap_or_else(|e| panic!("kiosk request {id}: {e}"))
}

/// A quote with the id `id`, answered 200.
async fn quote(counter: &Counter, id: &str, side: &str, amount: Value, what: &str) {
    counter.hands.ids.push(id);
    let answer = counter
        .ask(
            None,
            "quote",
            json!({ "type": side, "currency": "USD", "amount": amount }),
        )
        .await;
    assert_eq!(answer.status, 200, "{what}: {}", answer.text);
    assert_eq!(
        answer.json()["id"],
        json!(id),
        "{what}: the id it was given"
    );
}

/// A quote answered `status`, which writes no request and no audit line.
async fn not_written(counter: &Counter, side: &str, amount: Value, status: u16, what: &str) {
    let before = requests(counter).await;
    let mark = audit_mark(counter).await;
    let answer = counter
        .ask(
            None,
            "quote",
            json!({ "type": side, "currency": "USD", "amount": amount }),
        )
        .await;
    assert_eq!(answer.status, status, "{what}: {:.300}", answer.text);
    if status == 400 {
        answer.assert_refused(400, AMOUNT_OR_CURRENCY, what);
    }
    assert_eq!(
        requests(counter).await,
        before,
        "{what}: no request written"
    );
    let (written, _) = audit_since(&counter.desk, mark).await;
    assert!(
        written.is_empty(),
        "{what}: no audit line written: {written:?}"
    );
}

/// §9 "Stale, snapshot without `policy_version_id`" and "Stale, policy row missing": the quote
/// answers 500 on either side and writes nothing (today it quotes the stale price). The same
/// line made fresh is quoted: no lookup for a line that is not stale (§9 "Line not stale").
#[actix_web::test]
async fn a_stale_line_whose_policy_cannot_be_found_is_a_server_error_and_writes_nothing() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    for (n, (names, label)) in unfindable().into_iter().enumerate() {
        assert_eq!(policy_rows(&pool, &names).await, 0, "the setup: {label}");
        let ids = publish_auto(&pool, MAIN, &names, 30, &[USD]).await;
        not_written(&counter, "BUY", json!(1), 500, &format!("BUY, {label}")).await;
        not_written(&counter, "SELL", json!(100), 500, &format!("SELL, {label}")).await;

        let line = id_of(&ids, "USD");
        reference_age(&pool, &line, 1).await;
        let before = requests(&counter).await;
        let id = format!("00000000-0000-4000-8000-0000000769a{n}");
        quote(&counter, &id, "BUY", json!(100), &format!("fresh, {label}")).await;
        assert_eq!(
            stored(&counter, &id).await,
            (Some(line), d("3210.00")),
            "{label}"
        );
        assert_eq!(requests(&counter).await, before + 1, "{label}");
    }
    counter.finish().await;
}

/// §9 "Stale, `LIMIT` at exactly the limit": 32 × 32.1 = 1,027.20 is quoted under a 1,027.20
/// limit and refused with the "no rate" answer under 1,027.19 (one satang over).
#[actix_web::test]
async fn a_stale_line_under_limit_is_quoted_at_exactly_the_limit_and_refused_one_satang_over() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    let at = policy(&pool, MAIN, Some("LIMIT"), Some("1027.20")).await;
    let ids = publish_auto(&pool, MAIN, &Names::Policy(at), 30, &[USD]).await;
    let id = "00000000-0000-4000-8000-0000000769b1";
    quote(
        &counter,
        id,
        "BUY",
        json!(32),
        "1,027.20 under a 1,027.20 limit",
    )
    .await;
    assert_eq!(
        stored(&counter, id).await,
        (Some(id_of(&ids, "USD")), d("1027.20"))
    );

    let below = policy(&pool, MAIN, Some("LIMIT"), Some("1027.19")).await;
    publish_auto(&pool, MAIN, &Names::Policy(below), 30, &[USD]).await;
    not_written(
        &counter,
        "BUY",
        json!(32),
        400,
        "1,027.20 under a 1,027.19 limit",
    )
    .await;
    assert_eq!(requests(&counter).await, 1);
    counter.finish().await;
}

/// §9 "Stale, `on_stale` NULL": a stale line under a policy with no `on_stale` is quoted at any
/// amount, at that line.
#[actix_web::test]
async fn a_stale_line_under_a_null_policy_is_quoted() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    let none = policy(&pool, MAIN, None, None).await;
    let ids = publish_auto(&pool, MAIN, &Names::Policy(none), 30, &[USD]).await;
    let id = "00000000-0000-4000-8000-0000000769c1";
    quote(&counter, id, "BUY", json!(1000), "stale under NULL").await;
    assert_eq!(
        stored(&counter, id).await,
        (Some(id_of(&ids, "USD")), d("32100.00"))
    );
    counter.finish().await;
}
