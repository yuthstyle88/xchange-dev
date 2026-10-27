//! PV-15b (the kiosk and desk halves) and the kiosk's board-only currencies (pricing-tiers.md §15.7, §15.8, §15.8a,
//! PV-m; `H:` §8, §10.6, §10.8).
//! Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! - The kiosk (self-service) treats a hidden FOLLOW line as `quoteEnabled: false` ("contact staff"), even when the
//!   currency is kiosk-enabled: a stale or blocked FOLLOW price is never shown as ready to use (`H:` §10.6).
//! - A currency with a board price that is not kiosk-enabled is listed `quoteEnabled: false`: seeing a price never
//!   enables a quote (`H:` §8, §10.8).
//! - The desk still quotes the published line; its answer carries `pricingHidden: true` and the reason. That DTO
//!   field is PV-i2's (`dto/v3/quote.rs`, not in the skeleton), so it is read from the JSON.

mod v9_follow_common;

use actix_web::http::Method;
use serde_json::{Value, json};
use v9_follow_common::*;

const BR_B: &str = "BR-B";

fn currency<'a>(data: &'a Value, code: &str) -> &'a Value {
    data["currencies"]
        .as_array()
        .unwrap_or_else(|| panic!("currencies is an array: {data}"))
        .iter()
        .find(|c| c["code"] == json!(code))
        .unwrap_or_else(|| panic!("{code} is listed: {data}"))
}

/// PV-15b (kiosk) and PV-m: at B, USD (kiosk-enabled, B follows, MAIN's latest plan `BLOCKED`) is listed
/// `enabled: true`, `quoteEnabled: false`; EUR (a board price at B, not kiosk-enabled) is listed with its price and
/// `quoteEnabled: false`; GBP (kiosk-enabled, AUTO, visible) keeps `quoteEnabled: true`.
#[tokio::test]
async fn the_kiosk_lists_a_hidden_follow_line_and_a_board_only_currency_as_not_quotable() {
    let room = Room::start().await;
    let now = whole_second_now();
    room.create_branch(BR_B).await;
    room.policy(BR_B).await;
    // B's own AUTO lines before it follows, through the draft path: the push copies them into B's next publication.
    room.bot(&[("EUR", "38.00", "38.60"), ("GBP", "44.00", "44.80")])
        .await;
    room.owner_publishes_auto(BR_B, &["EUR", "GBP"]).await;
    b_follows_and_main_blocks(&room, BR_B, now).await;
    room.kiosk_switch(BR_B, "USD", true).await;
    room.kiosk_switch(BR_B, "GBP", true).await;

    let a = view(&room, &owner(), "kiosk_currencies", BR_B, None).await;
    let data = a.ok("kiosk_currencies")["data"].clone();

    let usd_row = currency(&data, "USD");
    assert_eq!(usd_row["enabled"], json!(true), "{usd_row}");
    assert_eq!(
        usd_row["quoteEnabled"],
        json!(false),
        "§15.7: a hidden FOLLOW line is not quotable at the kiosk: {usd_row}"
    );

    let eur_row = currency(&data, "EUR");
    assert_eq!(eur_row["enabled"], json!(false), "{eur_row}");
    assert!(
        eur_row["buy"].is_string(),
        "a board-only currency is listed with its board price: {eur_row}"
    );
    assert_eq!(
        eur_row["quoteEnabled"],
        json!(false),
        "§15.8: board-only ⇒ contact staff: {eur_row}"
    );

    let gbp_row = currency(&data, "GBP");
    assert_eq!(
        gbp_row["quoteEnabled"],
        json!(true),
        "kiosk-enabled and visible: {gbp_row}"
    );
    room.finish().await;
}

/// PV-15b (desk): B follows, MAIN's latest plan is `BLOCKED` ⇒ a DESK quote of USD at B still quotes the published
/// line (a teller confirms, `H:` gives the desk no hide rule) and the line answers `pricingHidden: true`,
/// `pricingHiddenReason: "BLOCKED"` (§15.7, §15.8a). Fails today at the missing JSON key (PV-i2's DTO field).
#[tokio::test]
async fn a_desk_quote_of_a_hidden_follow_line_answers_pricing_hidden() {
    let room = Room::start().await;
    let now = whole_second_now();
    room.create_branch(BR_B).await;
    room.policy(BR_B).await;
    b_follows_and_main_blocks(&room, BR_B, now).await;

    let body = json!({
        "channel": "DESK",
        "branch": BR_B,
        "lines": [{ "currency": "USD", "shopSide": "BUY", "quantity": "100" }],
    });
    let a = call_in(
        state(&room),
        Some(&owner()),
        Method::POST,
        "/api/quotes",
        Some(body),
        None,
    )
    .await;
    let quote = a.ok("the desk quote");
    let line = &quote["lines"][0];
    assert_eq!(
        line["pricingHidden"],
        json!(true),
        "§15.7: the desk is warned: {line}"
    );
    assert_eq!(line["pricingHiddenReason"], json!("BLOCKED"), "{line}");
    room.finish().await;
}
