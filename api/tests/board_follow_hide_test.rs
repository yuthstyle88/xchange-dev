//! Board currencies BC-7 (`docs/design/board-currencies.md` §4, §9):
//! `MarketPricingLoop::plan_once` skips a `FOLLOW` currency whose `MAIN` `ALL` row is `HIDE`.

mod v9_follow_common;

use v9_follow_common::*;
use xc_rate::domain::pricing::follow_plan::FollowResultStatus;

#[tokio::test]
async fn plan_with_a_hide_follow_currency_is_not_planned() {
    let room = Room::start().await;
    let now = whole_second_now();
    room.policy(MAIN).await;
    room.bot(&[("USD", USD_BOT.0, USD_BOT.1), ("AED", "9.1000", "9.2000")])
        .await;
    room.tt(
        now - minutes(10),
        &[("USD", PV1_TT.0, PV1_TT.1), ("AED", "9.12", "9.18")],
    )
    .await;
    follow_on(&room, "USD").await;
    follow_on(&room, "AED").await;
    room.kiosk_switch(MAIN, "AED", false).await;

    let results = pricing_loop(&room.pool)
        .plan_once(now)
        .await
        .unwrap_or_else(|e| panic!("plan_once: {}", e.code()));

    assert_eq!(results.len(), 1, "only USD planned: {results:?}");
    assert_eq!(results[0].currency, usd());
    assert_eq!(results[0].status, FollowResultStatus::Published);
    assert_eq!(room.follow_result("AED").await, None);
    assert!(room.published_tiers(MAIN, "AED").await.is_empty());
    room.finish().await;
}
