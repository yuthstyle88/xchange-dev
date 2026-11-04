//! BC-9 (`board-currencies.md` §6, #6087): `published_rates` honours `branch_currencies.display` (`ASK` and `HIDE`
//! lines are `hidden: true`, `hiddenReason: "DISPLAY"`, on top of the `FOLLOW` rule) and sets `display.more` to the
//! branch's `ASK` rows with no shown line, ordered by `sort_order`.

mod v9_follow_common;

use actix_web::http::StatusCode;
use serde_json::{Value, json};
use v9_follow_common::*;
use xc_rate::domain::pricing::follow_plan::FollowResultStatus;

#[tokio::test]
async fn ask_row_with_a_line_is_hidden_in_more_and_never_a_price() {
    let room = Room::start().await;
    room.publish_manual(
        MAIN,
        &[
            ("USD", "1", "35.10", "35.50"),
            ("NZD", "1", "20.10", "20.50"),
        ],
    )
    .await;

    let data = projection(&room, MAIN).await;
    let nzd = lines_of(&data, "NZD");
    assert_eq!(nzd.len(), 1, "{data}");
    assert_eq!(nzd[0]["hidden"], json!(true), "{data}");
    assert_eq!(nzd[0]["hiddenReason"], json!("DISPLAY"), "{data}");
    assert!(
        data["display"]["more"]
            .as_array()
            .expect("more array")
            .contains(&json!("NZD")),
        "ASK row with a line stays in more: {data}"
    );

    let public = public_rates(&room, MAIN, None).await;
    assert_eq!(public.status, StatusCode::OK, "{}", public.text);
    assert!(
        lines_of(&public.body, "NZD").is_empty(),
        "public board feed strips hidden ASK line: {}",
        public.body
    );
    assert!(
        public.body["display"]["more"]
            .as_array()
            .expect("more array")
            .contains(&json!("NZD")),
        "public board feed lists NZD in more: {}",
        public.body
    );
    room.finish().await;
}

#[tokio::test]
async fn hide_row_with_a_line_is_hidden_and_not_in_more() {
    let room = Room::start().await;
    room.publish_manual(
        MAIN,
        &[
            ("USD", "1", "35.10", "35.50"),
            ("AUD", "1", "22.10", "22.50"),
        ],
    )
    .await;

    let data = projection(&room, MAIN).await;
    let aud = lines_of(&data, "AUD");
    assert_eq!(aud.len(), 1, "{data}");
    assert_eq!(aud[0]["hidden"], json!(true), "{data}");
    assert_eq!(aud[0]["hiddenReason"], json!("DISPLAY"), "{data}");
    assert!(
        !data["display"]["more"]
            .as_array()
            .expect("more array")
            .contains(&json!("AUD")),
        "HIDE row is not in more: {data}"
    );

    let public = public_rates(&room, MAIN, None).await;
    assert_eq!(public.status, StatusCode::OK, "{}", public.text);
    assert!(
        lines_of(&public.body, "AUD").is_empty(),
        "public board feed strips hidden HIDE line: {}",
        public.body
    );
    assert!(
        !public.body["display"]["more"]
            .as_array()
            .expect("more array")
            .contains(&json!("AUD")),
        "public board feed omits AUD from more: {}",
        public.body
    );
    room.finish().await;
}

#[tokio::test]
async fn show_row_with_stale_follow_is_hidden_with_reason_as_today() {
    let room = Room::start().await;
    let now = whole_second_now();
    let batch = follow_usd_at_main(&room, now, 10).await;
    let planned = pricing_loop(&room.pool)
        .plan_once(now)
        .await
        .unwrap_or_else(|e| panic!("plan_once: {}", e.code()));
    assert_eq!(
        planned[0].status,
        FollowResultStatus::Published,
        "{planned:?}"
    );

    room.tt_fetched_at(batch, now - minutes(31)).await;

    let data = projection(&room, MAIN).await;
    let usd = lines_of(&data, "USD");
    assert!(!usd.is_empty(), "{data}");
    for line in usd {
        assert_eq!(line["hidden"], json!(true), "{line}");
        assert_eq!(line["hiddenReason"], json!("STALE"), "{line}");
    }
    room.finish().await;
}

#[tokio::test]
async fn more_is_exactly_the_ask_rows_with_no_shown_line() {
    let room = Room::start().await;
    room.publish_manual(MAIN, &[("USD", "1", "35.10", "35.50")])
        .await;

    let before = projection(&room, MAIN).await;
    assert_eq!(
        before["display"]["more"],
        json!([
            "NZD", "SGD", "MYR", "CNY", "VND", "KRW", "INR", "GBP", "IDR", "TRY", "JOD", "DKK",
            "SEK", "NOK"
        ]),
        "all 14 ASK rows ordered by sort_order when CNY has no shown line: {before}"
    );

    room.publish_manual(
        MAIN,
        &[("USD", "1", "35.10", "35.50"), ("CNY", "1", "4.80", "4.95")],
    )
    .await;

    let after = projection(&room, MAIN).await;
    let cny = lines_of(&after, "CNY");
    assert_eq!(cny.len(), 1, "{after}");
    assert_eq!(cny[0]["hidden"], Value::Bool(false), "{after}");
    assert_eq!(
        after["display"]["more"],
        json!([
            "NZD", "SGD", "MYR", "VND", "KRW", "INR", "GBP", "IDR", "TRY", "JOD", "DKK", "SEK",
            "NOK"
        ]),
        "CNY leaves more once its SHOW row has a shown line: {after}"
    );
    room.finish().await;
}

#[tokio::test]
async fn all_market_off_empties_more() {
    let room = Room::start().await;
    room.publish_manual(MAIN, &[("USD", "1", "35.10", "35.50")])
        .await;
    room.display_row(MAIN, &[1, 2, 3, 4], &["USD"], true, false)
        .await;

    let data = projection(&room, MAIN).await;
    assert_eq!(
        data["display"]["more"],
        json!([]),
        "allMarket off lists no ASK row in more: {data}"
    );
    room.finish().await;
}
