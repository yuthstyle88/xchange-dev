//! XC-V9-PRICING PV-6 (pricing-tiers.md §15.6, B4; `H:` §8): with PV-2 published at MAIN, a desk quote of USD worth
//! 300,000 THB — tier 4's band — is priced at **tier 1**, and its line points at tier 1's published line. The planner
//! preview, which reads the same `PgPublishedRates::line_for`, answers the same. The kiosk is not covered here.
//! Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! `H:` is the owner's handoff (`PRICING-DESIGN.md`, kept out of the repo). `H:` §8: the server quote takes
//! `rates[currency].buy/sell` and does not call `tierFor()` to change the price with the amount.
//!
//! The amounts: 9,028 USD × 33.23 = 300,000.44 THB (shop BUY) and 9,028 × 33.87 = 305,778.36 THB (shop SELL), both
//! above 200,000, so V.9's bands (§15.3 step 6) put them in tier 4; B4 prices them at tier 1 all the same. Both
//! products are exact at 2 places, so no rounding rule is involved.

mod v9_approve_common;

use std::str::FromStr;

use actix_web::http::Method;
use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use v9_approve_common::*;
use xc_app::application::planner::planner_preview::PlannerPreview;
use xc_app::domain::planner::planner_draft::{DraftAmount, DraftPlan};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, CustomerSide, ForeignAmount, Money, Rate};

const QUANTITY: &str = "9028";

/// MAIN with PV-1's tiers approved and published. Answers tier 1's published line id.
async fn pv2_published() -> (Room, uuid::Uuid) {
    let room = main_room().await;
    approve(
        &room,
        &owner(),
        MAIN,
        vec![row("USD", PV1_TIERS, None)],
        None,
    )
    .await
    .ok("PV-2 approve");
    let lines = room.published(MAIN, "USD").await;
    assert_eq!(lines.len(), 4, "PV-2: 4 tier lines: {lines:?}");
    let t1 = lines
        .iter()
        .find(|l| l.lower_exclusive_thb.is_none())
        .unwrap_or_else(|| panic!("tier 1 has no lower bound: {lines:?}"));
    assert_eq!(
        (t1.buy, t1.sell),
        (Some(dec("33.23")), Some(dec("33.87"))),
        "tier 1 is PV-1's"
    );
    let id = t1.id;
    (room, id)
}

/// `POST /api/quotes` (DESK) at MAIN as a FRONT teller, one USD line of `QUANTITY` on `side`.
async fn desk_quote(room: &Room, side: &str) -> Value {
    let body = json!({
        "channel": "DESK",
        "branch": MAIN,
        "lines": [{ "currency": "USD", "shopSide": side, "quantity": QUANTITY }],
    });
    call(
        room,
        &front(MAIN),
        Method::POST,
        "/api/quotes",
        Some(body),
        None,
    )
    .await
    .ok("the desk quote")
}

/// PV-6 (B4, `H:` §8): a desk quote of USD worth 300,000 THB at MAIN is priced at tier 1, both sides, and its line
/// (answered and stored) is tier 1's published line.
#[tokio::test]
async fn v9_quote_a_desk_quote_in_tier_4s_band_is_priced_at_tier_1() {
    let (room, t1) = pv2_published().await;
    for (side, rate, thb) in [
        ("BUY", "33.23", "300000.44"),
        ("SELL", "33.87", "305778.36"),
    ] {
        let q = desk_quote(&room, side).await;
        let l = &q["lines"][0];
        assert_eq!(
            dec_of(&l["baseRate"]),
            dec(rate),
            "{side}: tier 1's rate, not tier 4's: {q}"
        );
        assert_eq!(dec_of(&l["finalRate"]), dec(rate), "{side}: {q}");
        assert_eq!(l["thbAmount"], json!(thb), "{side}: {q}");
        assert_eq!(
            l["publishedRateLineId"],
            json!(t1.to_string()),
            "{side}: §15.6: the quote line points at tier 1"
        );
        let stored: String = sqlx::query_scalar(
            "SELECT published_rate_line_id::text FROM quote_lines WHERE quote_id = $1::uuid",
        )
        .bind(q["id"].as_str().expect("the quote id"))
        .fetch_one(&room.pool)
        .await
        .expect("the stored quote line");
        assert_eq!(stored, t1.to_string(), "{side}: the stored line");
    }
    room.finish().await;
}

/// PV-6 (§15.6: "That one change covers … the planner (`planner_preview.rs:72-80`)"): the planner preview of the
/// same amounts answers tier 1's rate and THB too.
#[tokio::test]
async fn v9_quote_the_planner_preview_is_priced_at_tier_1_too() {
    let (room, _) = pv2_published().await;
    // CUSTOMER_SELL is the shop's BUY; CUSTOMER_BUY is the shop's SELL (P4).
    for (side, rate, thb) in [
        (CustomerSide::CustomerSell, "33.23", "300000.44"),
        (CustomerSide::CustomerBuy, "33.87", "305778.36"),
    ] {
        let plan = DraftPlan {
            branch_id: Some(BranchId::new(MAIN)),
            currency: CurrencyCode::parse("USD").expect("USD"),
            customer_side: side,
            amount: DraftAmount::Quantity(ForeignAmount::from_decimal(
                Decimal::from_str(QUANTITY).expect("a decimal"),
            )),
        };
        let mut tx = room.pool.begin().await.expect("begin");
        let estimate = PlannerPreview::new(room.pool.clone())
            .estimate(&mut tx, &plan, Utc::now())
            .await
            .unwrap_or_else(|e| panic!("{side:?}: the preview estimates: {}", e.code()));
        tx.commit().await.expect("commit");
        assert_eq!(
            estimate.rate,
            Rate::parse(rate).expect("a rate"),
            "{side:?}: tier 1's rate"
        );
        assert_eq!(
            estimate.thb,
            Money::parse(thb).expect("THB"),
            "{side:?}: THB at tier 1"
        );
    }
    room.finish().await;
}
