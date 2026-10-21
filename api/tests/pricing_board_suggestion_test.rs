//! XC-PRICING PX-9 (pricing-tiers.md §4, §5.2, §6): the `pricing_board` view fills `suggestion`
//! (`pricing_board.rs:342` was `None`), staleness, the source error and the recommendations. Prices are §12.3's
//! worked example (USD BoT 32.4500 / 32.9000, adjust −0.5 / +0.4, `pricing_common`) with a TT quote of
//! 32.5 / 33.1; the expected tiers are V.9's formula (`engine.mjs:40-58`), checked by hand in the PR. Needs PX-1,
//! PX-5, PX-6 and PX-8 on the branch. Written before the implementation by the ticket author (claude/mbp-xc-pricing,
//! PX-0). Do not edit, delete or weaken; add tests in a file of your own.

mod pricing_common;

use actix_web::http::Method;
use chrono::Utc;
use pricing_common::*;
use serde_json::{Value, json};
use xc_rate::domain::pricing::market_quote::MarketSource;

async fn board(room: &Room) -> Value {
    view(room, &owner(), "pricing_board", B01).await.ok("pricing_board")["data"].clone()
}

fn row<'a>(data: &'a Value, currency: &str) -> &'a Value {
    data["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["currency"] == json!(currency))
        .unwrap_or_else(|| panic!("no {currency} row"))
}

fn source<'a>(data: &'a Value, name: &str) -> &'a Value {
    data["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["source"] == json!(name))
        .unwrap()
}

#[tokio::test]
async fn an_anchored_row_carries_v9s_four_tier_prices() {
    let room = Room::start().await;
    room.worked_example().await;
    room.tt_batch(&[("USD", "32.5000", "33.1000")], 0).await;
    let data = board(&room).await;
    let usd = row(&data, "USD");
    let tiers: Vec<(i64, rust_decimal::Decimal, rust_decimal::Decimal)> = usd["suggestion"]
        .as_array()
        .expect("a suggestion")
        .iter()
        .map(|t| (t["tierNo"].as_i64().unwrap(), dec_of(&t["buy"]), dec_of(&t["sell"])))
        .collect();
    assert_eq!(
        tiers,
        vec![
            (1, dec("32.53"), dec("33.07")),
            (2, dec("32.54"), dec("33.06")),
            (3, dec("32.56"), dec("33.04")),
            (4, dec("32.59"), dec("33.01")),
        ]
    );
    assert_eq!(usd["flags"], json!([]), "fresh TT, a BoT base, margins reproduce the prices");
    assert_eq!(usd["selectable"], json!(true));
    room.finish().await;
}

#[tokio::test]
async fn a_row_without_a_market_quote_has_no_suggestion() {
    let room = Room::start().await;
    room.worked_example().await;
    let data = board(&room).await;
    assert_eq!(row(&data, "EUR")["suggestion"], Value::Null);
    room.finish().await;
}

#[tokio::test]
async fn an_old_tt_batch_is_stale_everywhere_it_shows() {
    let room = Room::start().await;
    room.worked_example().await;
    room.tt_batch(&[("USD", "32.5000", "33.1000")], 2).await;
    let data = board(&room).await;
    assert_eq!(source(&data, "TT")["stale"], json!(true), "2 h > 30 min");
    assert_eq!(source(&data, "SR")["stale"], json!(true), "no SR batch at all");
    let usd = row(&data, "USD");
    let tt = usd["quotes"].as_array().unwrap().iter().find(|q| q["source"] == json!("TT")).unwrap();
    assert_eq!(tt["stale"], json!(true));
    assert!(usd["flags"].as_array().unwrap().contains(&json!("StaleSource")));
    assert_eq!(usd["selectable"], json!(false), "StaleSource blocks bulk approve");
    room.finish().await;
}

#[tokio::test]
async fn a_fresh_tt_batch_is_not_stale() {
    let room = Room::start().await;
    room.worked_example().await;
    room.tt_batch(&[("USD", "32.5000", "33.1000")], 0).await;
    let data = board(&room).await;
    assert_eq!(source(&data, "TT")["stale"], json!(false));
    assert_eq!(source(&data, "TT")["error"], Value::Null);
    room.finish().await;
}

#[tokio::test]
async fn the_last_fetch_error_is_shown_on_its_source() {
    let room = Room::start().await;
    room.worked_example().await;
    let state = state_of(&room);
    state.market_health.record(MarketSource::SuperRich, Utc::now(), Some("SR: HTTP 503".into()));
    let a = call_with(state, &owner(), Method::GET, &format!("/api/v3/views/pricing_board?branch={B01}"), None, None).await;
    let data = a.ok("pricing_board")["data"].clone();
    assert_eq!(source(&data, "SR")["error"], json!("SR: HTTP 503"));
    assert_eq!(source(&data, "TT")["error"], Value::Null);
    room.finish().await;
}

#[tokio::test]
async fn stale_sources_are_recommendations_with_no_currency() {
    let room = Room::start().await;
    room.worked_example().await;
    let data = board(&room).await;
    let recs = data["recommendations"].as_array().unwrap();
    let tt = recs.iter().find(|r| r["code"] == json!("TtStale")).expect("TtStale");
    assert_eq!((tt["severity"].clone(), tt["currency"].clone(), tt["side"].clone()), (json!("Warn"), Value::Null, Value::Null));
    room.finish().await;
}
