//! Up-front test for FD-3 (#974): with `XC_BROKER_FEED_TOKEN` unset, the Broker feed route
//! refuses every request with `401` and never panics. Its own binary, because the token is read
//! once at boot. Written by the ticket author before the implementation. Do not edit, delete or
//! weaken.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod broker_feed_common;
mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use broker_feed_common::{completed, exchange, get, send};
use rpc_common::Desk;
use serde_json::Value;

#[actix_web::test]
async fn every_request_is_refused() {
    // SAFETY: this binary's only test; it runs before the server reads the variable.
    unsafe { std::env::remove_var("XC_BROKER_FEED_TOKEN") };
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let a = exchange(&desk, 1).await;
    completed(&desk, &a, "2026-10-06T01:00:00.000000Z").await;
    for token in [None, Some(""), Some("   "), Some("anything")] {
        let answer = send(&app, get("", token)).await;
        assert_eq!(answer.status, 401, "{token:?}: {:.300}", answer.text);
        let body: Value = serde_json::from_str(&answer.text).expect("a JSON body");
        assert!(body["error"].is_string(), "{body}");
        assert!(!answer.text.contains(&a), "no feed data leaks");
    }
    desk.finish().await;
}
