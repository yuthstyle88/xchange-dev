//! XC-V9-PRICING PV-m2 (pricing-tiers.md §15.7; `H:` §10.6): the kiosk refuses a quote on a hidden FOLLOW line. PV-m
//! lists such a line `quoteEnabled: false` ("contact staff"); this closes the other door, a customer posting
//! `quote` to `POST /api/kiosk` directly. A visible FOLLOW line still quotes, so the refusal is the hide rule and
//! not "FOLLOW is never quotable".
//! Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING). Do not edit, delete or
//! weaken; add tests in a file of your own.

mod shift_setup_common;
mod v9_follow_common;

use actix_web::http::{Method, StatusCode};
use chrono::Utc;
use serde_json::{Value, json};
use v9_follow_common::*;

const BR_B: &str = "BR-B";

/// A customer's kiosk `quote` at `branch` (no session; the branch is named as the kiosk screen names it).
async fn kiosk_quote(room: &Room, branch: &str, currency: &str) -> Answer {
    let body = json!({
        "action": "quote",
        "payload": { "type": "BUY", "currency": currency, "amount": 100, "branchId": branch },
    });
    call_in(
        state(room),
        None,
        Method::POST,
        "/api/kiosk",
        Some(body),
        None,
    )
    .await
}

/// B's counter open (a kiosk refuses every quote at a closed counter), so a refusal below is the hide rule's.
async fn open_counter(room: &Room, branch: &str) {
    shift_setup_common::open_shift_by_count(
        &room.pool,
        branch,
        json!({ "THB": 100000, "USD": 2000 }),
        Utc::now(),
    )
    .await;
}

async fn rows(room: &Room, branch: &str) -> (i64, i64) {
    (
        room.count(&format!("kiosk_requests WHERE branch_id = '{branch}'"))
            .await,
        room.count(&format!(
            "quotes WHERE branch_id = '{branch}' AND channel = 'KIOSK'"
        ))
        .await,
    )
}

/// PV-m2: B follows USD and MAIN's latest plan is `BLOCKED` ⇒ the published line is hidden (§15.7), so a kiosk
/// quote of USD at B is refused like any line the kiosk cannot quote (400), and writes neither a kiosk request nor
/// a quote.
#[tokio::test]
async fn the_kiosk_refuses_a_quote_on_a_hidden_follow_line() {
    let room = Room::start().await;
    let now = whole_second_now();
    room.create_branch(BR_B).await;
    room.policy(BR_B).await;
    b_follows_and_main_blocks(&room, BR_B, now).await;
    room.kiosk_switch(BR_B, "USD", true).await;
    open_counter(&room, BR_B).await;

    let a = kiosk_quote(&room, BR_B, "USD").await;
    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "§15.7: a hidden FOLLOW line is not quotable at the kiosk: {}",
        a.text
    );
    assert_eq!(
        rows(&room, BR_B).await,
        (0, 0),
        "a refused kiosk quote leaves nothing behind"
    );
    room.finish().await;
}

/// PV-m2 control: B follows USD and MAIN's plan is current (the line is visible) ⇒ the kiosk quotes it.
#[tokio::test]
async fn the_kiosk_still_quotes_a_visible_follow_line() {
    let room = Room::start().await;
    let now = whole_second_now();
    room.create_branch(BR_B).await;
    room.policy(BR_B).await;
    follow_usd_at_main(&room, now, 10).await;
    enrol(&room, BR_B, "USD", 5, -5).await;
    let planned = pricing_loop(&room.pool)
        .plan_once(now)
        .await
        .unwrap_or_else(|e| panic!("plan_once: {}", e.code()));
    assert_eq!(planned.len(), 1, "{planned:?}");
    room.kiosk_switch(BR_B, "USD", true).await;
    open_counter(&room, BR_B).await;

    let a = kiosk_quote(&room, BR_B, "USD").await;
    let body: Value = a.ok("a kiosk quote of a visible FOLLOW line");
    assert!(body["id"].is_string(), "{body}");
    assert_eq!(rows(&room, BR_B).await, (1, 1), "one request, one quote");
    room.finish().await;
}
