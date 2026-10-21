//! Up-front tests for #1112: when the feed read fails, `GET /api/v3/system/broker/exchanges` answers
//! `500 {"error": "internal error"}` and never hands the database's error text to the caller.
//! Written by the ticket author (claude/5d45b017, on the xchange lane controller's order) before
//! the implementation. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The lane controller's ruling on #1112 (2026-10-10): `tracing::error!` the error and return the
//! fixed body `{"error": "internal error"}`, not `V3Error`, because Broker's wire (design
//! `broker-seam.md` §6.4) and #974's `assert_error_body` need `error` to be a string.
//!
//! This binary sets `XC_BROKER_FEED_TOKEN` once, before any server is built (the token is read
//! once at boot), as `broker_exchanges_route_test.rs` does. Needs `TEST_DATABASE_URL`; nothing
//! here skips.

mod broker_feed_common;
mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use std::sync::Once;

use broker_feed_common::{get, send};
use rpc_common::Desk;
use serde_json::{Value, json};

const TOKEN: &str = "fd3-feed-token-0123456789abcdef";

fn configure() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: called once, before any server of this binary reads the variable.
        unsafe { std::env::set_var("XC_BROKER_FEED_TOKEN", TOKEN) };
    });
}

/// With the feed table hidden, the read fails inside sqlx with an error naming the relation.
/// The answer is a 500 with exactly the fixed body; none of the database's words reach Broker.
#[actix_web::test]
async fn a_failed_feed_read_is_a_fixed_500_without_the_database_error() {
    configure();
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    sqlx::query("ALTER TABLE broker_exchange_feed RENAME TO broker_exchange_feed_gone")
        .execute(&desk.db.pool)
        .await
        .expect("hide the table");

    let answer = send(&app, get("", Some(TOKEN))).await;

    sqlx::query("ALTER TABLE broker_exchange_feed_gone RENAME TO broker_exchange_feed")
        .execute(&desk.db.pool)
        .await
        .expect("restore the table");

    assert_eq!(answer.status, 500, "{:.300}", answer.text);
    let lower = answer.text.to_lowercase();
    for leaked in [
        "broker_exchange_feed",
        "relation",
        "does not exist",
        "database",
        "sqlx",
        "error returned from",
    ] {
        assert!(
            !lower.contains(leaked),
            "the 500 body hands the caller the database error ({leaked:?}): {}",
            answer.text
        );
    }
    let body: Value = serde_json::from_str(&answer.text)
        .unwrap_or_else(|_| panic!("a JSON body: {:.300}", answer.text));
    assert_eq!(body, json!({ "error": "internal error" }));
    desk.finish().await;
}
