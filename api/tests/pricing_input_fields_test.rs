//! #4902: `pricingApprove` and `pricingConfigure` refuse bad input as 400 `validationFailed` **with the field** the
//! screen can point at, through `v3::input` (design `docs/design/blueprint/bounded-inputs.md` B-3), and their messages
//! carry no Rust type or variant names. No rule changes: what is accepted today stays accepted.
//!
//! Written before the implementation by the ticket author's lane (claude/b873aa20, for claude/7ca80c). Do not edit,
//! delete or weaken these tests; add your own in another file.
//!
//! Behaviour rows pinned (ticket §3):
//! - PA1 approve, tier 2 sell ≤ buy ⇒ 400 `validationFailed`, field `tiers[1].sell`, no `SellNotAboveBuy` in the message.
//! - PA2 approve, blank reason ⇒ 400, field `reason`.
//! - PA3 approve, buy `1.123456789` ⇒ 400, field `tiers[0].buy`.
//! - PC1 configure, minSpread `6` ⇒ 400, field `minSpreadPct`.
//! - PC2 configure, a margin `abc` ⇒ 400, field `margins.<name>` (the ticket leaves `<name>` open; the prefix is pinned).
//! - PX  valid requests ⇒ as today.

mod pricing_common;

use pricing_common::*;
use serde_json::{Value, json};

const APPROVE: &str = "/api/pricing/approve";
const CONFIGURE: &str = "/api/pricing/configure";

/// The in-bound USD prices `pricing_approve_test.rs` approves (T1 +0.4/−0.4 … T4 +0.6/−0.6).
const USD_PRICES: [(&str, &str); 4] = [
    ("32.41755", "32.90"),
    ("32.433775", "32.88355"),
    ("32.45", "32.8671"),
    ("32.48245", "32.8342"),
];

/// The in-guard USD margins `pricing_configure_test.rs` saves.
const USD_MARGINS: [(&str, &str); 4] = [
    ("0.4000", "-0.4000"),
    ("0.4500", "-0.4500"),
    ("0.5000", "-0.5000"),
    ("0.6000", "-0.6000"),
];

fn approve_body(v1: uuid::Uuid, prices: [(&str, &str); 4], reason: &str) -> Value {
    json!({
        "policyVersionId": v1.to_string(),
        "reason": reason,
        "rows": [{
            "currency": "USD",
            "tiers": prices.iter().enumerate()
                .map(|(i, (b, s))| json!({ "tierNo": i + 1, "buy": b, "sell": s }))
                .collect::<Vec<_>>(),
        }],
    })
}

fn configure_body(v1: uuid::Uuid, margins: Option<[(&str, &str); 4]>, min_spread: Value) -> Value {
    let tiers = match margins {
        Some(m) => json!(
            m.iter()
                .enumerate()
                .map(
                    |(i, (b, s))| json!({ "tierNo": i + 1, "buyMarginPct": b, "sellMarginPct": s })
                )
                .collect::<Vec<_>>()
        ),
        None => Value::Null,
    };
    json!({
        "policyVersionId": v1.to_string(),
        "currency": "USD",
        "tiers": tiers,
        "minSpreadPct": min_spread,
        "anchorSource": Value::Null,
        "reason": "#4902 test",
    })
}

fn field(a: &Answer) -> Option<&str> {
    a.body["error"]["field"].as_str()
}

fn message(a: &Answer) -> &str {
    a.body["error"]["message"].as_str().unwrap_or("")
}

async fn refused_on(room: &Room, a: &Answer, expected_field: &str, what: &str) {
    a.refused(400, "validationFailed", what);
    assert_eq!(
        field(a),
        Some(expected_field),
        "{what}: field; body {}",
        a.text
    );
    assert_eq!(room.versions(B01).await.len(), 1, "{what}: no new version");
}

#[tokio::test]
async fn pa1_approve_tier_2_sell_not_above_buy_names_tiers_1_sell() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let mut p = USD_PRICES;
    p[1] = ("32.433775", "32.433775");
    let a = command(
        &room,
        &owner(),
        APPROVE,
        B01,
        approve_body(v1, p, "PA1"),
        None,
    )
    .await;
    refused_on(&room, &a, "tiers[1].sell", "PA1 T2 sell = buy").await;
    assert!(
        !message(&a).contains("SellNotAboveBuy"),
        "PA1: no variant name in the message; body {}",
        a.text
    );
    room.finish().await;
}

#[tokio::test]
async fn pa2_approve_blank_reason_names_reason() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let a = command(
        &room,
        &owner(),
        APPROVE,
        B01,
        approve_body(v1, USD_PRICES, "   "),
        None,
    )
    .await;
    refused_on(&room, &a, "reason", "PA2 blank reason").await;
    room.finish().await;
}

#[tokio::test]
async fn pa3_approve_buy_with_nine_decimals_names_tiers_0_buy() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let mut p = USD_PRICES;
    p[0] = ("1.123456789", "32.90");
    let a = command(
        &room,
        &owner(),
        APPROVE,
        B01,
        approve_body(v1, p, "PA3"),
        None,
    )
    .await;
    refused_on(&room, &a, "tiers[0].buy", "PA3 buy 1.123456789").await;
    room.finish().await;
}

#[tokio::test]
async fn pc1_configure_min_spread_6_names_min_spread_pct() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let a = command(
        &room,
        &owner(),
        CONFIGURE,
        B01,
        configure_body(v1, None, json!("6")),
        None,
    )
    .await;
    refused_on(&room, &a, "minSpreadPct", "PC1 min spread 6").await;
    room.finish().await;
}

#[tokio::test]
async fn pc2_configure_margin_abc_names_a_margins_field() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let mut m = USD_MARGINS;
    m[0] = ("abc", "-0.4000");
    let a = command(
        &room,
        &owner(),
        CONFIGURE,
        B01,
        configure_body(v1, Some(m), Value::Null),
        None,
    )
    .await;
    a.refused(400, "validationFailed", "PC2 margin abc");
    assert!(
        field(&a).is_some_and(|f| f.starts_with("margins.") && f.len() > "margins.".len()),
        "PC2: field `margins.<name>`; body {}",
        a.text
    );
    assert_eq!(room.versions(B01).await.len(), 1, "PC2: no new version");
    room.finish().await;
}

#[tokio::test]
async fn px_valid_requests_are_accepted_as_today() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let a = command(
        &room,
        &owner(),
        APPROVE,
        B01,
        approve_body(v1, USD_PRICES, "PX approve"),
        None,
    )
    .await;
    assert_eq!(
        a.ok("PX approve")["draftState"],
        json!("PUBLISHED"),
        "{}",
        a.text
    );
    room.finish().await;

    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let a = command(
        &room,
        &owner(),
        CONFIGURE,
        B01,
        configure_body(v1, Some(USD_MARGINS), json!("1.5000")),
        None,
    )
    .await;
    assert_eq!(a.ok("PX configure")["version"], json!(2), "{}", a.text);
    room.finish().await;
}
