//! XC-PRICING PX-7 (pricing-tiers.md §5.2): `pricingApprove` refuses a tier set V.9's `validateTiers` refuses
//! (`engine.mjs:68-84`): sell above buy, equal-or-better, tier 1 within 3 % of the market anchor. Prices are §12.3's
//! worked example (`pricing_common`). Written before the implementation by the ticket author (claude/mbp-xc-pricing,
//! PX-0). Do not edit, delete or weaken; add tests in a file of your own.

mod pricing_common;

use pricing_common::*;
use serde_json::{Value, json};

const ROUTE: &str = "/api/pricing/approve";

/// The in-bound USD prices `pricing_approve_test.rs` approves (T1 +0.4/−0.4 … T4 +0.6/−0.6).
const USD_PRICES: [(&str, &str); 4] = [
    ("32.41755", "32.90"),
    ("32.433775", "32.88355"),
    ("32.45", "32.8671"),
    ("32.48245", "32.8342"),
];

fn body(v1: uuid::Uuid, prices: [(&str, &str); 4]) -> Value {
    json!({
        "policyVersionId": v1.to_string(),
        "reason": "PX-7 test",
        "rows": [{
            "currency": "USD",
            "tiers": prices.iter().enumerate()
                .map(|(i, (b, s))| json!({ "tierNo": i + 1, "buy": b, "sell": s }))
                .collect::<Vec<_>>(),
        }],
    })
}

async fn refused_on_tiers(room: &Room, v1: uuid::Uuid, prices: [(&str, &str); 4], what: &str) {
    let a = command(room, &owner(), ROUTE, B01, body(v1, prices), None).await;
    a.refused(400, "validationFailed", what);
    assert_eq!(room.versions(B01).await.len(), 1, "{what}: no new version");
}

#[tokio::test]
async fn sell_not_above_buy_is_refused() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let mut p = USD_PRICES;
    p[0] = ("32.41755", "32.41755");
    refused_on_tiers(&room, v1, p, "T1 sell = buy").await;
    room.finish().await;
}

#[tokio::test]
async fn a_worse_higher_tier_is_refused() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let mut p = USD_PRICES;
    p[3] = ("32.40", "32.8342");
    refused_on_tiers(&room, v1, p, "T4 buys below T3").await;
    room.finish().await;
}

#[tokio::test]
async fn tier_one_far_from_the_market_is_refused() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    // TT 34.00 / 34.50: T1 buy 32.41755 is 4.7 % below, beyond 3 × 1 %.
    room.tt_batch(&[("USD", "34.0000", "34.5000")], 0).await;
    refused_on_tiers(&room, v1, USD_PRICES, "T1 far from TT").await;
    room.finish().await;
}

#[tokio::test]
async fn the_suggested_shape_still_approves() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    room.tt_batch(&[("USD", "32.4000", "32.9500")], 0).await;
    let a = command(&room, &owner(), ROUTE, B01, body(v1, USD_PRICES), None).await;
    assert_eq!(
        a.ok("approve")["draftState"],
        json!("PUBLISHED"),
        "{}",
        a.text
    );
    room.finish().await;
}
