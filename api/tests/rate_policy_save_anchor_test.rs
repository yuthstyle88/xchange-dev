//! PT-16 (pricing-tiers.md §3, §3.1, §11): `ratePolicySave` (`POST /api/rate-policies`) takes an optional
//! `anchorSource` under the frame rule, and its save-time guard check covers the per-currency override rows the new
//! version carries. Written before the implementation by the ticket author (claude/647e1967). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Choices pinned here (the implementer follows them):
//! - `anchorSource` absent or `null` ⇒ the version keeps the current anchor (`PolicyVersionWriter` carries it).
//!   `BOT` · `MARKET_TT` · `MARKET_SR`; anything else ⇒ 400 `validationFailed`.
//! - Changing the anchor is a frame change: without a valid EXECUTIVE row ⇒ `roleNotAllowedHere`, nothing
//!   written. Sending the current anchor unchanged is not a change.
//! - Every override row the new version carries is checked against the new version's guard of **its own currency**
//!   (`adjust + margin` inside `[min_pct, max_pct]`, both sides, every tier), the same check the branch tiers get
//!   (`rate_policy_save.rs`'s guard loop). A carried row outside ⇒ 400 `validationFailed`, nothing written.

mod pricing_common;

use pricing_common::*;
use serde_json::{Value, json};
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xchange::presentation::http::v3::error::V3Error;

const ROUTE: &str = "/api/rate-policies";

fn guard(min_buy: &str, max_buy: &str) -> Value {
    json!({ "buy": { "min_pct": min_buy, "max_pct": max_buy },
            "sell": { "min_pct": "-5.0000", "max_pct": "5.0000" } })
}

/// `pricing_common`'s worked-example frame (USD and EUR guards ±5) with `buy`/`sell` adjusts and `extra` fields.
fn save_body(buy: &str, sell: &str, guards: Value, extra: Value) -> Value {
    let mut b = json!({
        "buyAdjustPct": buy,
        "sellAdjustPct": sell,
        "guards": guards,
        "maxReferenceAgeHours": null,
        "onStale": null,
        "staleLimitThb": null,
        "reason": "PT-16 test",
    });
    for (k, v) in extra.as_object().expect("an object") {
        b[k] = v.clone();
    }
    b
}

fn frame_guards() -> Value {
    let g = guard("-5.0000", "5.0000");
    json!({ "USD": g.clone(), "EUR": g })
}

async fn anchor_of(room: &Room, version: uuid::Uuid) -> String {
    sqlx::query_scalar("SELECT anchor_source FROM rate_policy_versions WHERE id = $1")
        .bind(version)
        .fetch_one(&room.pool)
        .await
        .expect("anchor_source")
}

/// USD override on v1 whose T4 buy margin is +5.0 (adjust −0.5 ⇒ 4.5): inside ±5, outside a guard of max 4.
const USD_WIDE: [(&str, &str); 4] = [
    ("0.0000", "0.0000"),
    ("0.1000", "-0.1000"),
    ("0.2000", "-0.2000"),
    ("5.0000", "-0.3000"),
];

#[tokio::test]
async fn the_owner_sets_the_branch_anchor() {
    let room = Room::start().await;
    room.worked_example().await;
    let body = save_body(
        "-0.5",
        "0.4",
        frame_guards(),
        json!({ "anchorSource": "MARKET_TT" }),
    );
    command(&room, &owner(), ROUTE, B01, body, None)
        .await
        .ok("set anchor");
    let v2 = room.current_version(B01).await;
    assert_eq!(anchor_of(&room, v2).await, "MARKET_TT");
    room.finish().await;
}

#[tokio::test]
async fn an_adjust_only_save_keeps_the_anchor() {
    let room = Room::start().await;
    room.worked_example().await;
    let body = save_body(
        "-0.5",
        "0.4",
        frame_guards(),
        json!({ "anchorSource": "MARKET_TT" }),
    );
    command(&room, &owner(), ROUTE, B01, body, None)
        .await
        .ok("set anchor");
    let body = save_body("-0.4", "0.4", frame_guards(), json!({}));
    command(&room, &rate_manager(), ROUTE, B01, body, None)
        .await
        .ok("adjust only");
    let v3 = room.current_version(B01).await;
    assert_eq!(room.versions(B01).await.len(), 3);
    assert_eq!(
        anchor_of(&room, v3).await,
        "MARKET_TT",
        "§11: adjust-only save on a branch anchored MARKET_TT keeps it"
    );
    room.finish().await;
}

#[tokio::test]
async fn a_rate_manager_cannot_change_the_anchor_but_may_send_it_unchanged() {
    let room = Room::start().await;
    room.worked_example().await;
    let body = save_body(
        "-0.5",
        "0.4",
        frame_guards(),
        json!({ "anchorSource": "MARKET_TT" }),
    );
    let a = command(&room, &rate_manager(), ROUTE, B01, body, None).await;
    let err = PolicyError::RoleNotAllowedHere;
    assert_eq!(a.status.as_u16(), err.status(), "{}", a.text);
    assert_eq!(
        a.code(),
        json!(V3Error::from(err).code),
        "§11: RATE_MANAGER changes anchorSource ⇒ roleNotAllowedHere"
    );
    assert_eq!(room.versions(B01).await.len(), 1, "nothing written");

    let body = save_body(
        "-0.4",
        "0.4",
        frame_guards(),
        json!({ "anchorSource": "BOT" }),
    );
    command(&room, &rate_manager(), ROUTE, B01, body, None)
        .await
        .ok("the current anchor, unchanged");
    room.finish().await;
}

#[tokio::test]
async fn an_unknown_anchor_is_refused() {
    let room = Room::start().await;
    room.worked_example().await;
    let body = save_body(
        "-0.5",
        "0.4",
        frame_guards(),
        json!({ "anchorSource": "MARKET_XX" }),
    );
    let a = command(&room, &owner(), ROUTE, B01, body, None).await;
    a.refused(400, "validationFailed", "anchorSource MARKET_XX");
    assert_eq!(room.versions(B01).await.len(), 1);
    room.finish().await;
}

#[tokio::test]
async fn a_guard_tightened_so_a_carried_override_breaks_it_is_refused() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    room.override_margins(v1, "USD", USD_WIDE).await;
    // USD buy guard [−1, 4]: the branch's tiers (−0.5 … −0.2) fit, the USD override's T4 (4.5) does not.
    let guards = json!({ "USD": guard("-1.0000", "4.0000"), "EUR": guard("-5.0000", "5.0000") });
    let a = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        save_body("-0.5", "0.4", guards, json!({})),
        None,
    )
    .await;
    a.refused(
        400,
        "validationFailed",
        "§11: a guard tightened so a carried override breaks it",
    );
    assert_eq!(room.versions(B01).await.len(), 1, "nothing written");
    room.finish().await;
}

#[tokio::test]
async fn an_adjust_that_pushes_a_carried_override_outside_its_guard_is_refused() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    room.override_margins(v1, "USD", USD_WIDE).await;
    // buy adjust +0.6: the branch's tiers give 0.6 … 0.9, the USD override's T4 gives 5.6 > 5.
    let a = command(
        &room,
        &rate_manager(),
        ROUTE,
        B01,
        save_body("0.6", "0.4", frame_guards(), json!({})),
        None,
    )
    .await;
    a.refused(
        400,
        "validationFailed",
        "carried override outside the guard",
    );
    assert_eq!(room.versions(B01).await.len(), 1, "nothing written");
    room.finish().await;
}

#[tokio::test]
async fn a_carried_override_inside_the_guard_is_kept() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    room.override_margins(v1, "USD", USD_WIDE).await;
    command(
        &room,
        &rate_manager(),
        ROUTE,
        B01,
        save_body("-0.6", "0.4", frame_guards(), json!({})),
        None,
    )
    .await
    .ok("adjust only, override still inside");
    let v2 = room.current_version(B01).await;
    assert_eq!(
        room.override_rows(v2).await.len(),
        4,
        "§11: ratePolicySave changing only adjust_pct carries every currency's overrides"
    );
    room.finish().await;
}
