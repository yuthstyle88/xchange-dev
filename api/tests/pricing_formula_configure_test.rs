//! pricing-formula wave 3 (docs/design/blueprint/pricing-formula.md §4, §6a; B14, B18): `pricingConfigure` runs the
//! §4 check, and the guard is computed from the price the formula gives, not from `adjust_pct + margin`.
//!
//! Written before the implementation. Do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.
//!
//! Contract pinned here: `pricingConfigure` builds the version it would write (the currency's new override among
//! the others) and runs `SpreadCheck::run(Branch)` in place of today's `eff = adjust + margin` guard loop
//! (`pricing_configure.rs:258-300`); a failure is `400 validationFailed`, the message one `SpreadFailure::describe()`
//! per line, and the field names the first failure's input as the screen does (`PricingPolicy.tsx:67-68`):
//! `OutsideGuard` on tier n ⇒ `margins.buy-n` / `margins.sell-n` by its side, any other failure on tier n ⇒
//! `margins.buy-n`, a failure on the single line ⇒ `tiers`. A pass answers `notChecked`.

mod common;
mod pricing_common;
mod pricing_formula_api;

use pricing_common::{Room, command, owner};
use pricing_formula_api::*;
use serde_json::{Value, json};
use uuid::Uuid;

async fn room() -> (Room, Uuid) {
    let room = Room::start().await;
    bot_1028(&room).await;
    let v = policy(&room, "MAIN", "0", "0", guards("-1.0000", "1.0000"), true).await;
    (room, v)
}

fn tiers(margins: [(&str, &str); 4]) -> Value {
    Value::Array(
        margins
            .iter()
            .enumerate()
            .map(|(i, (b, s))| json!({ "tierNo": i + 1, "buyMarginPct": b, "sellMarginPct": s }))
            .collect(),
    )
}

async fn configure(room: &Room, version: Uuid, margins: [(&str, &str); 4]) -> pricing_common::Answer {
    command(
        room,
        &owner(),
        CONFIGURE,
        "MAIN",
        json!({
            "policyVersionId": version.to_string(),
            "currency": "USD",
            "tiers": tiers(margins),
            "minSpreadPct": null,
            "anchorSource": null,
            "reason": "configure",
        }),
        None,
    )
    .await
}

/// Tier 4 buys 1.2 % under the reference: outside a ±1 % guard, without inverting.
const TIER4_BUY_1_2: [(&str, &str); 4] = [("0", "0"), ("0.1000", "-0.1000"), ("0.2000", "-0.2000"), ("-1.2000", "-0.3000")];

#[actix_web::test]
async fn b18_under_the_default_a_margin_past_the_guard_is_refused_by_price() {
    let (room, v) = room().await;
    let a = configure(&room, v, TIER4_BUY_1_2).await;
    a.refused(400, "validationFailed", "B18 default");
    let (field, message) = error_of(&a);
    assert_eq!(field, json!("margins.buy-4"));
    assert!(message.contains("MAIN USD tier 4 buy 33.2444216 outside guard [-1, 1] (-1.2 %)"), "{message}");
    room.finish().await;
}

#[actix_web::test]
async fn b18_a_formula_that_ignores_the_margin_is_judged_on_its_price() {
    let (room, v) = room().await;
    // ref_mid ± 0.05 is 0.33 % from the reference whatever the margins: today's adjust + margin check would refuse
    // the 1.2 % margin, the price-based one does not
    org_formula(&room, 1, "ref_mid - 0.05", "ref_mid + 0.05", true).await;
    let body = configure(&room, v, TIER4_BUY_1_2).await.ok("B18 formula");
    assert_eq!(body["notChecked"], json!([]));
    room.finish().await;
}

#[actix_web::test]
async fn b14_a_tier_that_inverts_is_refused_and_named() {
    let (room, v) = room().await;
    // tier 3: buy +0.5 %, sell −0.5 % inverts the 0.95 % USD spread
    let a = configure(&room, v, [("0", "0"), ("0.1000", "-0.1000"), ("0.5000", "-0.5000"), ("0.5000", "-0.5000")]).await;
    a.refused(400, "validationFailed", "B14");
    let (field, message) = error_of(&a);
    assert_eq!(field, json!("margins.buy-3"));
    assert!(message.contains("MAIN USD tier 3 buy 33.816441 > sell 33.7972645"), "{message}");
    room.finish().await;
}
