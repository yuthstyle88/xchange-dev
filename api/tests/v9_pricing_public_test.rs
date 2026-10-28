//! PV-15, PV-15b (the public half), PV-15c and PV-15d (pricing-tiers.md §15.7, §15.8; `H:` §6, §8, §10.6): what the
//! public projection says a board may show, and the public route's ETag.
//! Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! - PV-15: a MANUAL line (approved through `pricingApprove`) with an old anchor is `hidden: false`,
//!   `pricingSource: "MANUAL"` — MANUAL is never hidden (B5).
//! - PV-15b: branch B follows; MAIN's latest plan is `BLOCKED` ⇒ B's (and MAIN's) USD line is `hidden`, `BLOCKED`.
//!   The kiosk and desk halves are in `v9_pricing_kiosk_test.rs`.
//! - PV-15c: MAIN re-plans other tiers but the publish fails ⇒ MAIN's and B's lines are `hidden`, `MISMATCH`. The
//!   failure is made deterministic with a seeded condition, see the test.
//! - PV-15d: `rateDisplaySave` changing `featured`, or a new Super Rich currency list, changes the public feed's
//!   ETag: a request with the old tag gets `200`, never `304` (§15.7 "Other changes").
//!
//! The projection is read through `published_rates_data`, the one reader the `published_rates` view and the public
//! route serialise (`public_rates_etag_test.rs` pins that they are the same body).

mod v9_follow_common;

use actix_web::http::StatusCode;
use serde_json::{Value, json};
use v9_follow_common::*;
use xc_rate::domain::pricing::follow_plan::FollowResultStatus;

const BR_B: &str = "BR-B";

fn approve_body(policy: uuid::Uuid, code: &str, tiers: &[(&str, &str)]) -> Value {
    json!({
        "policyVersionId": policy.to_string(),
        "reason": "PV-15",
        "rows": [{
            "currency": code,
            "tiers": tiers.iter().enumerate()
                .map(|(i, (b, s))| json!({ "tierNo": i + 1, "buy": b, "sell": s }))
                .collect::<Vec<_>>(),
        }],
    })
}

fn assert_hidden(data: &Value, code: &str, reason: &str, what: &str) {
    let lines = lines_of(data, code);
    assert!(!lines.is_empty(), "{what}: a {code} line: {data}");
    for line in lines {
        assert_eq!(line["pricingSource"], json!("FOLLOW"), "{what}: {line}");
        assert_eq!(line["hidden"], json!(true), "{what}: {line}");
        assert_eq!(line["hiddenReason"], json!(reason), "{what}: {line}");
    }
}

// ------------------------------------------------------------ PV-15

/// PV-15: USD approved MANUAL at MAIN against a TT anchor fetched 2 hours ago ⇒ published, and every USD line of
/// the projection is `pricingSource: "MANUAL"`, `hidden: false`, `hiddenReason: null` (B5: MANUAL is never hidden,
/// whatever its anchor's age).
#[tokio::test]
async fn a_manual_line_with_an_old_anchor_is_never_hidden() {
    let room = Room::start().await;
    let policy = room.policy(MAIN).await;
    room.bot(&[("USD", USD_BOT.0, USD_BOT.1)]).await;
    room.tt(
        whole_second_now() - minutes(120),
        &[("USD", PV1_TT.0, PV1_TT.1)],
    )
    .await;

    let a = command(
        &room,
        &owner(),
        APPROVE,
        MAIN,
        approve_body(policy, "USD", &PV1_TIERS),
    )
    .await;
    let out = a.ok("approve");
    assert_eq!(
        out["currencies"][0]["state"],
        json!("PUBLISHED"),
        "§15.8a: {}",
        a.text
    );
    assert_eq!(
        room.published_tiers(MAIN, "USD").await.first().copied(),
        Some((dec("33.23"), dec("33.87"))),
        "PV-2: tier 1 is the approved price"
    );

    let data = projection(&room, MAIN).await;
    let lines = lines_of(&data, "USD");
    assert!(!lines.is_empty(), "{data}");
    for line in lines {
        assert_eq!(line["pricingSource"], json!("MANUAL"), "{line}");
        assert_eq!(line["hidden"], json!(false), "B5: {line}");
        assert_eq!(line["hiddenReason"], Value::Null, "{line}");
    }
    room.finish().await;
}

// ------------------------------------------------------------ PV-15b

/// PV-15b (public half): B follows MAIN; MAIN's latest plan is `BLOCKED` (`LARGE_MOVE`, TT still current) ⇒ B's
/// FOLLOW line is `hidden: true`, `hiddenReason: "BLOCKED"`; so is MAIN's own line.
#[tokio::test]
async fn a_followed_line_is_hidden_blocked_when_mains_latest_plan_is_blocked() {
    let room = Room::start().await;
    let now = whole_second_now();
    room.create_branch(BR_B).await;
    room.policy(BR_B).await;
    b_follows_and_main_blocks(&room, BR_B, now).await;

    assert_hidden(&projection(&room, BR_B).await, "USD", "BLOCKED", "B");
    assert_hidden(&projection(&room, MAIN).await, "USD", "BLOCKED", "MAIN");
    room.finish().await;
}

// ------------------------------------------------------------ PV-15c

/// PV-15c: MAIN and B publish a FOLLOW USD; then MAIN re-plans other tiers from a fresh TT 33.25 / 33.95 (tier 1
/// 33.28 / 33.92, +0.15 %: within 1 %, so a `CANDIDATE`), but the publish fails ⇒ `pricing_follow_results` is
/// `CANDIDATE`, MAIN's publication is unchanged, and both MAIN's and B's USD lines are `hidden`, `MISMATCH` (MAIN's
/// latest result is not `PUBLISHED` with the line's set, §15.7).
///
/// reading: the deterministic failure is `TierSetPublisher::publish_in`'s own documented refusal — "any other line of
/// the copied publication keeps today's evaluation; if one of them sends the draft to `PENDING_APPROVAL`, the answer
/// is `ApprovalRequired`". MAIN's publication also holds an AUTO EUR line; after the first FOLLOW publication MAIN's
/// policy guard for EUR is narrowed to [+1 %, +2 %] while its adjust stays 0 %, so the copied EUR line is out of bound
/// (`Evaluation::of_tiered`: distance 0 % is outside [1, 2]) and the re-plan's draft cannot publish. This holds because
/// the FOLLOW publisher submits as a SYSTEM actor that is not EXECUTIVE, as the BoT feed does
/// (`branch_refresh.rs:76-85`); `RateDraftService::submit` self-approves an out-of-bound draft only for an EXECUTIVE
/// submitter (`rate_draft_service.rs:645`). Measured on the skeleton (scratch run, 2026-10-27): an AUTO EUR draft
/// submitted at MAIN by a non-EXECUTIVE SYSTEM actor is `Approved` before the narrowing and `PendingApproval` after.
#[tokio::test]
async fn a_replan_whose_publish_fails_hides_main_and_the_followed_branch_as_mismatch() {
    let room = Room::start().await;
    let now = whole_second_now();
    room.create_branch(BR_B).await;
    room.policy(BR_B).await;
    room.policy(MAIN).await;
    room.bot(&[("USD", USD_BOT.0, USD_BOT.1), ("EUR", "38.00", "38.60")])
        .await;
    room.owner_publishes_auto(MAIN, &["EUR"]).await;
    follow_on(&room, "USD").await;
    enrol(&room, BR_B, "USD", 5, -5).await;
    room.tt(now - minutes(10), &[("USD", PV1_TT.0, PV1_TT.1)])
        .await;
    let first = pricing_loop(&room.pool)
        .plan_once(now)
        .await
        .unwrap_or_else(|e| panic!("plan_once: {}", e.code()));
    assert_eq!(first[0].status, FollowResultStatus::Published, "{first:?}");
    let main_publication = room.current_publication(MAIN).await.expect("published");
    let b_publication = room.current_publication(BR_B).await.expect("B follows");

    room.exec(
        "UPDATE rate_policy_versions SET guards = guards || jsonb_build_object('EUR', \
             '{\"buy\": {\"min_pct\": \"1.0000\", \"max_pct\": \"2.0000\"}, \
               \"sell\": {\"min_pct\": \"1.0000\", \"max_pct\": \"2.0000\"}}'::jsonb) \
         WHERE branch_id = 'MAIN' AND valid_to IS NULL",
    )
    .await;
    room.tt(now - minutes(1), &[("USD", "33.25", "33.95")])
        .await;
    let second = pricing_loop(&room.pool)
        .plan_once(now)
        .await
        .unwrap_or_else(|e| panic!("plan_once: {}", e.code()));
    assert_eq!(
        second[0].status,
        FollowResultStatus::Candidate,
        "a failed publish records CANDIDATE: {second:?}"
    );
    let row = room.follow_result("USD").await.expect("a result row");
    assert_eq!(row.status, "CANDIDATE");
    assert_eq!(room.current_publication(MAIN).await, Some(main_publication));
    assert_eq!(room.current_publication(BR_B).await, Some(b_publication));

    assert_hidden(&projection(&room, MAIN).await, "USD", "MISMATCH", "MAIN");
    assert_hidden(&projection(&room, BR_B).await, "USD", "MISMATCH", "B");
    room.finish().await;
}

// ------------------------------------------------------------ PV-15d

/// PV-15d: `rateDisplaySave` at MAIN changing `featured` ⇒ the public feed's ETag changes; the old tag gets `200`
/// with the new tag and the new `display.featured`, never `304`.
#[tokio::test]
async fn rate_display_save_changing_featured_changes_the_public_etag() {
    let room = Room::start().await;
    room.publish_manual(
        MAIN,
        &[
            ("USD", "1", "35.10", "35.50"),
            ("EUR", "1", "38.10", "38.70"),
        ],
    )
    .await;
    let first = public_rates(&room, MAIN, None).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let old = first.etag.clone().expect("an ETag");
    let same = public_rates(&room, MAIN, Some(&old)).await;
    assert_eq!(same.status, StatusCode::NOT_MODIFIED, "nothing changed yet");

    command(
        &room,
        &owner(),
        DISPLAY,
        MAIN,
        json!({ "tiers": [1, 2, 3, 4], "featured": ["EUR", "USD"], "highlight": true, "allMarket": true }),
    )
    .await
    .ok("rateDisplaySave");

    let after = public_rates(&room, MAIN, Some(&old)).await;
    assert_eq!(
        after.status,
        StatusCode::OK,
        "a display change never answers 304: {}",
        after.text
    );
    let new = after.etag.clone().expect("an ETag");
    assert_ne!(new, old, "the tag moves with the display settings");
    assert_eq!(after.body["display"]["featured"], json!(["EUR", "USD"]));
    room.finish().await;
}

/// PV-15d: a new Super Rich batch with another currency list (KHR added) changes the public feed's ETag: the old tag
/// gets `200`, and with `allMarket` (the default) KHR, which MAIN has no price for, is in `display.more`.
#[tokio::test]
async fn a_new_super_rich_currency_list_changes_the_public_etag() {
    let room = Room::start().await;
    let now = whole_second_now();
    room.publish_manual(MAIN, &[("USD", "1", "35.10", "35.50")])
        .await;
    room.sr(now - minutes(3), &[("USD", "33.30", "33.40")])
        .await;
    let first = public_rates(&room, MAIN, None).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let old = first.etag.clone().expect("an ETag");

    room.sr(
        now - minutes(1),
        &[("USD", "33.30", "33.40"), ("KHR", "0.0080", "0.0090")],
    )
    .await;
    let after = public_rates(&room, MAIN, Some(&old)).await;
    assert_eq!(
        after.status,
        StatusCode::OK,
        "a new SR list never answers 304: {}",
        after.text
    );
    assert_ne!(after.etag.clone().expect("an ETag"), old);
    assert_eq!(after.body["display"]["more"], json!(["KHR"]));
    room.finish().await;
}
