//! PV-11, PV-12, PV-13, PV-14 and `pricingModeSave` (pricing-tiers.md §15.5, §15.4, §15.7; `H:` §6, §7): FOLLOW at
//! MAIN, driven through `MarketPricingLoop::plan_once(now)` built from the real services
//! (`RateDraftService` → `TierSetPublisher` → `PublishService`, `BranchPush` over the Pg stores).
//! Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! - `pricingModeSave {currency, mode}` (`POST /api/pricing/mode`) is MAIN only and an EXECUTIVE frame change:
//!   another branch ⇒ `validationFailed`; a RATE_MANAGER (has `ratePolicySave`, not the frame) ⇒
//!   `roleNotAllowedHere`; each save bumps `pricing_modes.revision`.
//! - PV-11: TT fetched 31 minutes before `now` ⇒ `BLOCKED` `STALE_SOURCE`, nothing published, and an earlier FOLLOW
//!   line is `hidden`, `STALE` (§15.7).
//! - PV-12: a new TT batch that moves tier 1 buy by 1.2 % ⇒ `BLOCKED` with `LARGE_MOVE`, the publication unchanged.
//! - PV-13: a fresh TT 33.20 / 33.90 (d = 2) ⇒ published once with PV-1's tiers, one `FOLLOW` tier set with
//!   `request_id = follow:MAIN:USD:<batch>`; the same batch planned again publishes nothing.
//! - PV-14: branch B enrolled (`follow = true`, +5 / −5 bp) follows MAIN with `H:` §7's tiers, `source = 'FOLLOW'`,
//!   `base_tier_set_id` = MAIN's set; branch E (not enrolled) and branch F (only a `'*'` row with `follow = true`)
//!   are left alone.
//!
//! Time is passed in: every market batch carries an explicit `fetched_at` and a matching `market_feed_status` row,
//! and `plan_once` takes `now`. The BoT base 33 / 34 is seeded so the suggestion has margins (no base ⇒
//! `NoAnchor`, `pricing_suggestion_test.rs:51-63`).

mod v9_follow_common;

use serde_json::json;
use v9_follow_common::*;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_rate::domain::pricing::follow_plan::{FollowReason, FollowResult, FollowResultStatus};
use xchange::presentation::http::v3::error::V3Error;

const BR_B: &str = "BR-B";
const BR_E: &str = "BR-E";
const BR_F: &str = "BR-F";

async fn plan(room: &Room, now: chrono::DateTime<chrono::Utc>) -> Vec<FollowResult> {
    pricing_loop(&room.pool)
        .plan_once(now)
        .await
        .unwrap_or_else(|e| panic!("plan_once: {}", e.code()))
}

fn only_usd(results: &[FollowResult]) -> &FollowResult {
    assert_eq!(
        results.len(),
        1,
        "plan_once plans every FOLLOW currency and nothing else: {results:?}"
    );
    assert_eq!(results[0].currency, usd(), "{results:?}");
    &results[0]
}

// ------------------------------------------------------------ pricingModeSave

/// PV-f: `pricingModeSave` at MAIN as the owner writes `pricing_modes` and bumps `revision` on every save (§15.5).
#[tokio::test]
async fn pricing_mode_save_at_main_writes_the_mode_and_bumps_the_revision() {
    let room = Room::start().await;
    let a = command(
        &room,
        &owner(),
        MODE,
        MAIN,
        json!({ "currency": "USD", "mode": "FOLLOW" }),
    )
    .await;
    assert_eq!(a.ok("FOLLOW"), json!({ "saved": true }), "§15.8a");
    let first: (String, i64) = sqlx::query_as(
        "SELECT mode, revision FROM pricing_modes WHERE trim(currency_code) = 'USD'",
    )
    .fetch_one(&room.pool)
    .await
    .expect("a pricing_modes row for USD");
    assert_eq!(first.0, "FOLLOW");

    command(
        &room,
        &owner(),
        MODE,
        MAIN,
        json!({ "currency": "USD", "mode": "MANUAL" }),
    )
    .await
    .ok("MANUAL");
    let second: (String, i64) = sqlx::query_as(
        "SELECT mode, revision FROM pricing_modes WHERE trim(currency_code) = 'USD'",
    )
    .fetch_one(&room.pool)
    .await
    .expect("still one row");
    assert_eq!(second.0, "MANUAL");
    assert!(
        second.1 > first.1,
        "every save bumps the revision: {first:?} then {second:?}"
    );
    room.finish().await;
}

/// PV-f: FOLLOW lives at MAIN only (`H:` §6) — `pricingModeSave` naming another branch is `validationFailed`.
#[tokio::test]
async fn pricing_mode_save_at_another_branch_is_validation_failed() {
    let room = Room::start().await;
    room.create_branch(BR_B).await;
    let a = command(
        &room,
        &owner(),
        MODE,
        BR_B,
        json!({ "currency": "USD", "mode": "FOLLOW" }),
    )
    .await;
    a.refused(400, "validationFailed", "FOLLOW away from MAIN");
    assert_eq!(room.count("pricing_modes").await, 0, "nothing written");
    room.finish().await;
}

/// PV-f: `pricingModeSave` is an EXECUTIVE frame change (V.9's OWNER): a RATE_MANAGER, who holds `ratePolicySave`,
/// is refused `roleNotAllowedHere` by the command, and a TELLER, who does not, is refused `roleNotAllowedHere` by
/// the seam.
#[tokio::test]
async fn pricing_mode_save_without_the_frame_right_is_refused() {
    let room = Room::start().await;
    let a = command(
        &room,
        &rate_manager(),
        MODE,
        MAIN,
        json!({ "currency": "USD", "mode": "FOLLOW" }),
    )
    .await;
    let err = PolicyError::RoleNotAllowedHere;
    assert_eq!(a.status.as_u16(), err.status(), "{}", a.text);
    assert_eq!(
        a.code(),
        json!(V3Error::from(err).code),
        "RATE_MANAGER ⇒ roleNotAllowedHere"
    );
    let t = command(
        &room,
        &teller(),
        MODE,
        MAIN,
        json!({ "currency": "USD", "mode": "FOLLOW" }),
    )
    .await;
    assert_eq!(
        t.status.as_u16(),
        PolicyError::RoleNotAllowedHere.status(),
        "TELLER: {}",
        t.text
    );
    assert_eq!(
        t.code(),
        json!(V3Error::from(PolicyError::RoleNotAllowedHere).code),
        "TELLER: {}",
        t.text
    );
    assert_eq!(room.count("pricing_modes").await, 0, "nothing written");
    room.finish().await;
}

// ------------------------------------------------------------ PV-11

/// PV-11: FOLLOW USD at MAIN with TT fetched 31 minutes before `now` ⇒ `BLOCKED` `STALE_SOURCE` (`H:` §6: at most
/// 30 minutes old), recorded in `pricing_follow_results`; no tier set, no publication.
#[tokio::test]
async fn a_tt_fetched_31_minutes_ago_blocks_follow_with_stale_source_and_publishes_nothing() {
    let room = Room::start().await;
    let now = whole_second_now();
    follow_usd_at_main(&room, now, 31).await;

    let results = plan(&room, now).await;
    let usd_result = only_usd(&results);
    assert_eq!(usd_result.status, FollowResultStatus::Blocked);
    assert_eq!(usd_result.reasons, vec![FollowReason::StaleSource]);
    assert_eq!(usd_result.planned_at, now);

    let row = room.follow_result("USD").await.expect("a result row");
    assert_eq!(row.status, "BLOCKED");
    assert_eq!(row.reasons, vec!["STALE_SOURCE".to_string()]);
    assert_eq!(row.tier_set_id, None);
    assert_eq!(room.count("pricing_tier_sets").await, 0, "no tier set");
    assert_eq!(room.publications(MAIN).await, 0, "nothing published");
    room.finish().await;
}

/// PV-11 (with an earlier FOLLOW publication, §15.7 `STALE`): published while TT was fresh; then TT's last fetch is
/// 31 minutes before `now` ⇒ the plan is `BLOCKED` `STALE_SOURCE`, the publication is unchanged, and the public
/// projection's USD line is `hidden: true`, `hiddenReason: "STALE"`, `pricingSource: "FOLLOW"`.
#[tokio::test]
async fn a_published_follow_line_whose_tt_turns_31_minutes_old_is_blocked_and_hidden_stale() {
    let room = Room::start().await;
    let now = whole_second_now();
    let batch = follow_usd_at_main(&room, now, 5).await;
    let first = plan(&room, now).await;
    assert_eq!(only_usd(&first).status, FollowResultStatus::Published);
    let publication = room.current_publication(MAIN).await.expect("published");
    let tiers = room.published_tiers(MAIN, "USD").await;

    room.tt_fetched_at(batch, now - minutes(31)).await;
    let second = plan(&room, now).await;
    let usd_result = only_usd(&second);
    assert_eq!(usd_result.status, FollowResultStatus::Blocked);
    assert_eq!(usd_result.reasons, vec![FollowReason::StaleSource]);
    let row = room.follow_result("USD").await.expect("a result row");
    assert_eq!(
        (row.status.as_str(), row.reasons.clone()),
        ("BLOCKED", vec!["STALE_SOURCE".to_string()])
    );

    assert_eq!(
        room.current_publication(MAIN).await,
        Some(publication),
        "nothing published"
    );
    assert_eq!(room.published_tiers(MAIN, "USD").await, tiers);
    assert_eq!(
        room.tier_sets(MAIN, "USD").await.len(),
        1,
        "no new tier set"
    );

    let data = projection(&room, MAIN).await;
    let usd_lines = lines_of(&data, "USD");
    assert!(!usd_lines.is_empty(), "the published USD line: {data}");
    for line in usd_lines {
        assert_eq!(line["pricingSource"], json!("FOLLOW"), "{line}");
        assert_eq!(line["hidden"], json!(true), "§15.7 fail closed: {line}");
        assert_eq!(line["hiddenReason"], json!("STALE"), "{line}");
    }

    // reading: §15.7 "the public rates feed drops a hidden line" and the DTO carries `hidden`; either way the feed
    // never answers USD as a visible price.
    let public = public_rates(&room, MAIN, None).await;
    let body = public.ok("the public feed");
    for line in lines_of(&body, "USD") {
        assert_eq!(
            line["hidden"],
            json!(true),
            "the public feed shows no stale FOLLOW price: {line}"
        );
    }
    room.finish().await;
}

// ------------------------------------------------------------ PV-12

/// PV-12: after a first fresh FOLLOW publication (tier 1 33.23 / 33.87), a new fresh TT batch 33.60 / 34.30 moves
/// tier 1 to 33.63 / 34.27 — buy +1.20 %, sell +1.18 % — past the 1 % default (`H:` §6: both sides, against both the
/// approved tier and the ledger) ⇒ `BLOCKED` with `LARGE_MOVE`, and the publication is unchanged.
/// Derived from H: §4.2's formula (PV-1's rounding): 33.60 × 1.001 = 33.6336 → 33.63; 34.30 × 0.999 = 34.2657 → 34.27;
/// 33.63 ÷ 33.23 − 1 = 1.2037 %; 34.27 ÷ 33.87 − 1 = 1.1810 %.
#[tokio::test]
async fn a_new_tt_moving_tier_one_by_1_2_percent_is_blocked_large_move_and_publishes_nothing() {
    let room = Room::start().await;
    let now = whole_second_now();
    follow_usd_at_main(&room, now, 10).await;
    assert_eq!(
        only_usd(&plan(&room, now).await).status,
        FollowResultStatus::Published
    );
    let publication = room.current_publication(MAIN).await.expect("published");
    assert_eq!(
        room.published_tiers(MAIN, "USD").await,
        pairs(&PV1_TIERS),
        "the first publication is PV-1's"
    );

    room.tt(now - minutes(1), &[("USD", "33.60", "34.30")])
        .await;
    let results = plan(&room, now).await;
    let usd_result = only_usd(&results);
    assert_eq!(usd_result.status, FollowResultStatus::Blocked);
    assert!(
        usd_result.reasons.contains(&FollowReason::LargeMove),
        "{usd_result:?}"
    );
    assert!(
        !usd_result.reasons.contains(&FollowReason::StaleSource),
        "the new batch is fresh: {usd_result:?}"
    );
    let row = room.follow_result("USD").await.expect("a result row");
    assert_eq!(row.status, "BLOCKED");
    assert!(row.reasons.contains(&"LARGE_MOVE".to_string()), "{row:?}");

    assert_eq!(room.current_publication(MAIN).await, Some(publication));
    assert_eq!(room.published_tiers(MAIN, "USD").await, pairs(&PV1_TIERS));
    assert_eq!(
        room.tier_sets(MAIN, "USD").await.len(),
        1,
        "no new tier set"
    );
    room.finish().await;
}

// ------------------------------------------------------------ PV-13

/// PV-13: FOLLOW USD with a fresh TT 33.20 / 33.90 (d = 2) ⇒ one publication of PV-1's tiers (33.23 / 33.87 ·
/// 33.24 / 33.86 · 33.26 / 33.84 · 33.29 / 33.80) as 4 tier lines; one `pricing_tier_sets` row `source = 'FOLLOW'`,
/// `request_id = follow:MAIN:USD:<batch id>`; `pricing_follow_results` `PUBLISHED` with that set and batch.
/// `plan_once` again on the same batch publishes nothing new.
#[tokio::test]
async fn a_fresh_tt_publishes_pv1s_tiers_once_and_the_same_batch_publishes_nothing_more() {
    let room = Room::start().await;
    let now = whole_second_now();
    let batch = follow_usd_at_main(&room, now, 2).await;

    let results = plan(&room, now).await;
    let usd_result = only_usd(&results);
    assert_eq!(usd_result.status, FollowResultStatus::Published);
    assert_eq!(usd_result.reasons, Vec::<FollowReason>::new());
    assert_eq!(usd_result.anchor_batch_id, Some(batch));

    let sets = room.tier_sets(MAIN, "USD").await;
    assert_eq!(sets.len(), 1, "one tier set: {sets:?}");
    let set = &sets[0];
    assert_eq!(set.source, "FOLLOW");
    assert_eq!(set.request_id, format!("follow:MAIN:USD:{batch}"));
    assert_eq!(set.base, None, "MAIN's own set has no base");
    assert_eq!(set.tiers, pairs(&PV1_TIERS), "H: §4.4");
    assert_eq!(usd_result.tier_set_id, Some(set.id));

    let row = room.follow_result("USD").await.expect("a result row");
    assert_eq!(row.status, "PUBLISHED");
    assert_eq!(row.tier_set_id, Some(set.id));
    assert_eq!(row.anchor_batch_id, Some(batch));

    assert_eq!(
        room.published_tiers(MAIN, "USD").await,
        pairs(&PV1_TIERS),
        "§15.3 step 6: 4 tier lines from the tier rows"
    );
    assert_eq!(room.publications(MAIN).await, 1);
    let source_snapshot: serde_json::Value = sqlx::query_scalar(
        "SELECT l.source_snapshot FROM published_rate_lines l \
         JOIN branches b ON b.current_publication_id = l.publication_id \
         WHERE b.id = 'MAIN' AND trim(l.currency_code) = 'USD' AND l.tier_lower_exclusive_thb IS NULL",
    )
    .fetch_one(&room.pool)
    .await
    .expect("tier 1's line");
    assert_eq!(
        source_snapshot["pricing"],
        json!({ "tier_set_id": set.id.to_string(), "source": "FOLLOW" }),
        "§15.3 step 6"
    );

    let again = plan(&room, now).await;
    assert_eq!(only_usd(&again).status, FollowResultStatus::Published);
    assert_eq!(
        room.publications(MAIN).await,
        1,
        "the same batch never publishes twice"
    );
    assert_eq!(room.tier_sets(MAIN, "USD").await.len(), 1, "one tier set");
    room.finish().await;
}

// ------------------------------------------------------------ PV-14

/// PV-14: branch B enrolled for USD (`follow = true`, buyBp +5, sellBp −5, through `pricingBranchPolicySave`)
/// follows MAIN's FOLLOW publication with `H:` §7's tiers (tier 1 33.24 / 33.86); its row is `source = 'FOLLOW'`,
/// `base_tier_set_id` = MAIN's set, `request_id = <MAIN's request>:BR-B`. Branch E (no row) and branch F (only a
/// `'*'` row with `follow = true`: a branch default never enrols, `H:` §7) are left alone, though each has a policy
/// and could publish.
#[tokio::test]
async fn an_enrolled_branch_follows_with_its_bp_and_unenrolled_branches_are_left_alone() {
    let room = Room::start().await;
    for b in [BR_B, BR_E, BR_F] {
        room.create_branch(b).await;
        room.policy(b).await;
    }
    let now = whole_second_now();
    follow_usd_at_main(&room, now, 3).await;
    enrol(&room, BR_B, "USD", 5, -5).await;
    room.exec(
        "INSERT INTO pricing_branch_policy (branch_id, currency_code, buy_bp, sell_bp, own, follow, updated_by, updated_at) \
         VALUES ('BR-F', '*', NULL, NULL, false, true, 'owner-1', now())",
    )
    .await;

    assert_eq!(
        only_usd(&plan(&room, now).await).status,
        FollowResultStatus::Published
    );
    let main_sets = room.tier_sets(MAIN, "USD").await;
    assert_eq!(main_sets.len(), 1);
    let main_set = &main_sets[0];

    let b_sets = room.tier_sets(BR_B, "USD").await;
    assert_eq!(b_sets.len(), 1, "B follows once: {b_sets:?}");
    let b_set = &b_sets[0];
    assert_eq!(
        b_set.source, "FOLLOW",
        "§15.5: a followed branch's row is FOLLOW"
    );
    assert_eq!(b_set.base, Some(main_set.id));
    assert_eq!(b_set.request_id, format!("{}:{BR_B}", main_set.request_id));
    assert_eq!(b_set.tiers, pairs(&PV7_TIERS), "H: §7");
    assert_eq!(
        room.published_tiers(BR_B, "USD").await,
        pairs(&PV7_TIERS),
        "B's tier 1 = 33.24 / 33.86 (H: §7)"
    );

    for b in [BR_E, BR_F] {
        assert_eq!(
            room.tier_sets(b, "USD").await,
            vec![],
            "{b} is not enrolled"
        );
        assert_eq!(room.current_publication(b).await, None, "{b} unchanged");
    }
    room.finish().await;
}
