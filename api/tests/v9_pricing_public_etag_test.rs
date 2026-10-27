//! PV-15d (pricing-tiers.md §15.7 "Other changes"): the public route's ETag hashes the effective display settings, so
//! every `rateDisplaySave` moves it, not only the first one away from V.9's default. Added by the sub-controller
//! claude/451f0116 after the PV-i review (PR 5188) found that no provided test saw a second change. Do not edit,
//! delete or weaken; add tests in a file of your own.

mod v9_follow_common;

use actix_web::http::StatusCode;
use serde_json::json;
use v9_follow_common::*;

/// PV-15d: two successive `featured` changes give two different ETags, and the tag from the first never answers 304
/// after the second.
#[tokio::test]
async fn every_display_change_moves_the_public_etag() {
    let room = Room::start().await;
    room.publish_manual(
        MAIN,
        &[
            ("USD", "1", "35.10", "35.50"),
            ("EUR", "1", "38.10", "38.70"),
        ],
    )
    .await;
    let mut tags = Vec::new();
    for featured in [json!(["EUR", "USD"]), json!(["USD", "EUR"])] {
        command(
            &room,
            &owner(),
            DISPLAY,
            MAIN,
            json!({ "tiers": [1, 2, 3, 4], "featured": featured, "highlight": true, "allMarket": true }),
        )
        .await
        .ok("rateDisplaySave");
        let answer = public_rates(&room, MAIN, None).await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.text);
        tags.push(answer.etag.clone().expect("an ETag"));
    }
    assert_ne!(tags[0], tags[1], "the second display change moves the tag");
    let stale = public_rates(&room, MAIN, Some(&tags[0])).await;
    assert_eq!(
        stale.status,
        StatusCode::OK,
        "the first change's tag is stale: {}",
        stale.text
    );
    room.finish().await;
}
