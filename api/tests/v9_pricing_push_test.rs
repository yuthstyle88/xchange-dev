//! XC-V9-PRICING PV-7, PV-8, PV-9, PV-10, the retry and `pricingBranchPolicySave` (pricing-tiers.md §15.3 step 7,
//! §15.4, §15.8a, §15.11): MAIN is the base; an approve at MAIN pushes to every other active branch that is not `own`
//! for the currency, adjusted in basis points (`H:` §7), each branch in its own transaction, and the answer lists each
//! branch's result. An approve at another branch makes that branch `own`.
//! Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! `H:` is the owner's handoff (`PRICING-DESIGN.md`, kept out of the repo). MAIN's tiers are PV-1's (`H:` §4.4); TT
//! is 33.20/33.90.
//!
//! Readings:
//! - reading: a branch row's `request_id` is `<MAIN row's request_id>:<branch>` (§15.4), read off MAIN's stored row,
//!   so the test does not fix how MAIN's own id is spelt.
//! - reading: PV-9's branch fails in `BranchPricingPolicy::adjust`'s `TierCheck` (bp +300/+300 pushes tier 1 more
//!   than 3 % from the anchor), which `BranchResult` reports as `validationFailed` (`branch_push.rs` doc).
//! - reading: a retried approve (same `Idempotency-Key`, same body) replays through the command seam; whatever it
//!   answers per branch must be `ALREADY_DONE`, or `PUBLISHED` with the same `tierSetId`, and nothing new is written.
//! - reading: `pricingBranchPolicySave` names the branch with the seam's `branch` key, like every v3 command.

mod v9_approve_common;

use serde_json::{Value, json};
use v9_approve_common::*;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xchange::presentation::http::v3::error::V3Error;

const B: &str = "XC-PV-B";
const C: &str = "XC-PV-C";
const D: &str = "XC-PV-D";

/// MAIN's PV-1 tiers at B with buyBp +5 / sellBp −5 (`H:` §7: `buy = floor_2(main × 1.0005)`,
/// `sell = ceil_2(main × 0.9995)`, d = 2 from the base). Tier 1 is `H:` §7's own example; tiers 2–4 are derived from
/// H: §7's formula:
/// - T1 33.23 × 1.0005 = 33.246615 → 33.24; 33.87 × 0.9995 = 33.853065 → 33.86
/// - T2 33.24 × 1.0005 = 33.256620 → 33.25; 33.86 × 0.9995 = 33.843070 → 33.85
/// - T3 33.26 × 1.0005 = 33.276630 → 33.27; 33.84 × 0.9995 = 33.823080 → 33.83
/// - T4 33.29 × 1.0005 = 33.306645 → 33.30; 33.80 × 0.9995 = 33.783100 → 33.79
///
/// Buys never fall and sells never rise, so the monotonic pass changes nothing.
const B_TIERS: [(&str, &str); 4] = [
    ("33.24", "33.86"),
    ("33.25", "33.85"),
    ("33.27", "33.83"),
    ("33.30", "33.79"),
];

/// A second MAIN set (monotonic, tier 1 within 0.1 % of TT).
const S2: [(&str, &str); 4] = [
    ("33.22", "33.88"),
    ("33.23", "33.87"),
    ("33.25", "33.85"),
    ("33.28", "33.81"),
];

/// A room with MAIN and `branches`, each on policy version 1, and TT USD 33.20/33.90.
async fn push_room(branches: &[&str]) -> Room {
    let room = main_room().await;
    for b in branches {
        room.branch(b).await;
    }
    room
}

/// `pricingBranchPolicySave` at `branch` as `who`.
async fn save_policy(room: &Room, who: &Who, branch: &str, body: Value) -> Answer {
    command(room, who, BRANCH_POLICY, branch, body, None).await
}

// ------------------------------------------------------------ PV-7

/// PV-7 (§15.4, `H:` §7): B saves buyBp +5 / sellBp −5 for USD; MAIN approves PV-1's tiers ⇒ B publishes its
/// adjusted tiers (tier 1 = 33.24/33.86) from a `MAIN_PUSH` row based on MAIN's set, and MAIN's answer lists B
/// `PUBLISHED`.
#[tokio::test]
async fn v9_push_main_approve_publishes_the_branch_adjusted_by_its_basis_points() {
    let room = push_room(&[B]).await;
    save_policy(
        &room,
        &owner(),
        B,
        json!({ "currency": "USD", "buyBp": 5, "sellBp": -5 }),
    )
    .await
    .ok("pricingBranchPolicySave");

    let out = approve(
        &room,
        &owner(),
        MAIN,
        vec![row("USD", PV1_TIERS, None)],
        None,
    )
    .await
    .ok("MAIN approves");
    assert_eq!(currency_result(&out, "USD")["state"], json!("PUBLISHED"));
    let main_set = room.only_tier_set(MAIN, "USD").await;
    let b_set = room.only_tier_set(B, "USD").await;

    let b = branch_result(&out, "USD", B);
    assert_eq!(b["state"], json!("PUBLISHED"), "{b}");
    assert!(b["error"].is_null(), "{b}");
    assert_eq!(
        b["tierSetId"],
        json!(b_set.id.to_string()),
        "the answer names B's row"
    );

    assert_eq!(b_set.source, "MAIN_PUSH", "§15.4: a manual push");
    assert_eq!(
        b_set.base_tier_set_id,
        Some(main_set.id),
        "adjusted from MAIN's set"
    );
    assert_eq!(
        b_set.request_id,
        format!("{}:{B}", main_set.request_id),
        "§15.4: request_id = <main request>:<branch>"
    );
    assert_eq!(b_set.prices(), decs(B_TIERS), "the adjusted tiers");
    assert_eq!(
        room.published_prices(B, "USD").await,
        decs(B_TIERS),
        "PV-7: B's published tier 1 = 33.24/33.86 (H: §7)"
    );
    assert_eq!(
        room.published_prices(MAIN, "USD").await,
        decs(PV1_TIERS),
        "MAIN keeps its own tiers"
    );
    room.finish().await;
}

// ------------------------------------------------------------ PV-8

/// PV-8 (§15.4, `H:` §7, §10.7): C is `own` for USD; MAIN's next approve leaves C's publication as it was and lists
/// C `SKIPPED_OWN`.
#[tokio::test]
async fn v9_push_an_own_branch_is_skipped_and_keeps_its_publication() {
    let room = push_room(&[C]).await;
    let first = approve(
        &room,
        &owner(),
        MAIN,
        vec![row("USD", PV1_TIERS, None)],
        None,
    )
    .await
    .ok("MAIN's first approve reaches C");
    assert_eq!(branch_result(&first, "USD", C)["state"], json!("PUBLISHED"));
    let main_id = tier_set_id_of(&currency_result(&first, "USD"));
    let c_publication = room.current_publication(C).await;
    let c_prices = room.published_prices(C, "USD").await;
    assert_eq!(c_prices, decs(PV1_TIERS), "bp 0: C publishes MAIN's tiers");

    room.seed_branch_policy(C, "USD", None, None, true).await;
    let second = approve(
        &room,
        &owner(),
        MAIN,
        vec![row("USD", S2, Some(&main_id.to_string()))],
        None,
    )
    .await
    .ok("MAIN's second approve");
    assert_eq!(currency_result(&second, "USD")["state"], json!("PUBLISHED"));
    assert_eq!(
        branch_result(&second, "USD", C)["state"],
        json!("SKIPPED_OWN"),
        "{second}"
    );
    assert_eq!(room.tier_sets(C, "USD").await.len(), 1, "no new row at C");
    assert_eq!(
        room.current_publication(C).await,
        c_publication,
        "C's publication unchanged"
    );
    assert_eq!(room.published_prices(C, "USD").await, c_prices);
    assert_eq!(room.published_prices(MAIN, "USD").await, decs(S2));
    room.finish().await;
}

// ------------------------------------------------------------ PV-9

/// PV-9 (§15.4, `H:` §5 "สาขาลูกส่งไม่สำเร็จไม่ได้ยกเลิกราคาที่ MAIN สำเร็จแล้ว"): D's push fails; MAIN and B stay
/// published, and the answer lists D `FAILED` with its error code.
///
/// D fails deterministically in its `TierCheck`: buyBp +300 / sellBp +300 (the largest allowed) gives tier 1
/// `floor_2(33.23 × 1.03) = floor_2(34.2269) = 34.22` / `ceil_2(33.87 × 1.03) = ceil_2(34.8861) = 34.89`
/// (derived from H: §7's formula). Against TT 33.20/33.90 that is 34.22 / 33.20 − 1 = +3.07 % on the buy side, beyond
/// 3 × farFromAnchorPct = 3 % (sell +2.92 %). The rest of D's set is well formed (T2 34.23/34.88, T3 34.25/34.86,
/// T4 34.28/34.82), so the anchor check is the only failure.
#[tokio::test]
async fn v9_push_a_failing_branch_does_not_undo_main_or_the_others() {
    let room = push_room(&[B, D]).await;
    room.seed_branch_policy(B, "USD", Some(5), Some(-5), false)
        .await;
    room.seed_branch_policy(D, "USD", Some(300), Some(300), false)
        .await;

    let out = approve(
        &room,
        &owner(),
        MAIN,
        vec![row("USD", PV1_TIERS, None)],
        None,
    )
    .await
    .ok("MAIN approves");
    assert_eq!(currency_result(&out, "USD")["state"], json!("PUBLISHED"));
    let d = branch_result(&out, "USD", D);
    assert_eq!(d["state"], json!("FAILED"), "{d}");
    assert_eq!(d["error"], json!("validationFailed"), "D's error code: {d}");
    assert!(d["tierSetId"].is_null(), "{d}");
    assert_eq!(branch_result(&out, "USD", B)["state"], json!("PUBLISHED"));

    assert!(
        room.tier_sets(D, "USD").await.is_empty(),
        "D's transaction rolled back"
    );
    assert_eq!(room.current_publication(D).await, None, "nothing at D");
    assert_eq!(
        room.published_prices(MAIN, "USD").await,
        decs(PV1_TIERS),
        "MAIN stays"
    );
    assert_eq!(
        room.published_prices(B, "USD").await,
        decs(B_TIERS),
        "B stays"
    );
    room.finish().await;
}

// ------------------------------------------------------------ PV-10

/// PV-10 (§15.3 step 7, `H:` §5 "อนุมัติในสาขาอื่นตั้งสกุลนั้นเป็น own price"): an approve at B publishes B's own
/// `MANUAL` set, sets B's USD row `own = true`, and answers no `branches`; MAIN's next approve skips B.
#[tokio::test]
async fn v9_push_an_approve_at_a_branch_makes_it_own_and_main_skips_it() {
    let room = push_room(&[B]).await;
    let out = approve(&room, &owner(), B, vec![row("USD", S2, None)], None)
        .await
        .ok("B approves");
    let usd = currency_result(&out, "USD");
    assert_eq!(usd["state"], json!("PUBLISHED"), "{out}");
    assert!(
        usd.get("branches").is_none(),
        "§15.8a: `branches` only at MAIN: {usd}"
    );
    let b_set = room.only_tier_set(B, "USD").await;
    assert_eq!(b_set.source, "MANUAL");
    assert_eq!(b_set.base_tier_set_id, None);
    let (_, _, own, _) = room
        .branch_policy(B, "USD")
        .await
        .unwrap_or_else(|| panic!("§15.3 step 7: B's USD row is written"));
    assert!(own, "§15.3 step 7: own = true");
    assert!(
        room.tier_sets(MAIN, "USD").await.is_empty(),
        "nothing at MAIN"
    );

    let main = approve(
        &room,
        &owner(),
        MAIN,
        vec![row("USD", PV1_TIERS, None)],
        None,
    )
    .await
    .ok("MAIN approves next");
    assert_eq!(
        branch_result(&main, "USD", B)["state"],
        json!("SKIPPED_OWN"),
        "{main}"
    );
    assert_eq!(room.tier_sets(B, "USD").await.len(), 1);
    assert_eq!(room.published_prices(B, "USD").await, decs(S2));
    room.finish().await;
}

// ------------------------------------------------------------ retry

/// §15.4 "Retrying with the same request id skips branches already written", `H:` §5: the same `Idempotency-Key`
/// at MAIN twice writes one MAIN set and one B set, publishes nothing new, and B is `ALREADY_DONE` or the same
/// `PUBLISHED` result.
#[tokio::test]
async fn v9_push_the_same_request_twice_writes_main_and_each_branch_once() {
    let room = push_room(&[B]).await;
    room.seed_branch_policy(B, "USD", Some(5), Some(-5), false)
        .await;
    let key = unique("pv-retry");
    let body = json!({
        "policyVersionId": room.current_version(MAIN).await.to_string(),
        "rows": [row("USD", PV1_TIERS, None)],
        "reason": "V.9 retry test",
    });

    let first = command(&room, &owner(), APPROVE, MAIN, body.clone(), Some(&key))
        .await
        .ok("first");
    let main_publication = room.current_publication(MAIN).await;
    let b_publication = room.current_publication(B).await;
    let second = command(&room, &owner(), APPROVE, MAIN, body, Some(&key))
        .await
        .ok("second, same key");

    assert_eq!(room.tier_sets(MAIN, "USD").await.len(), 1, "one MAIN set");
    assert_eq!(room.tier_sets(B, "USD").await.len(), 1, "one B set");
    assert_eq!(room.current_publication(MAIN).await, main_publication);
    assert_eq!(room.current_publication(B).await, b_publication);
    assert_eq!(
        currency_result(&second, "USD")["tierSetId"],
        currency_result(&first, "USD")["tierSetId"],
        "the same MAIN set"
    );
    let b1 = branch_result(&first, "USD", B);
    let b2 = branch_result(&second, "USD", B);
    assert_eq!(b1["state"], json!("PUBLISHED"), "{b1}");
    assert!(
        b2["state"] == json!("ALREADY_DONE")
            || (b2["state"] == json!("PUBLISHED") && b2["tierSetId"] == b1["tierSetId"]),
        "§15.4: B is ALREADY_DONE or unchanged on the retry: {b2}"
    );
    room.finish().await;
}

// ------------------------------------------------------------ pricingBranchPolicySave

/// §15.4 "Who sets the policy": the owner saves a branch default (`'*'`) and a currency row; each answers
/// `{saved: true}` and is stored as sent.
#[tokio::test]
async fn v9_push_branch_policy_save_stores_the_default_and_the_currency_row() {
    let room = push_room(&[B]).await;
    let a = save_policy(
        &room,
        &owner(),
        B,
        json!({ "currency": "*", "buyBp": 10, "sellBp": -10 }),
    )
    .await;
    assert_eq!(a.ok("save *")["saved"], json!(true));
    save_policy(
        &room,
        &owner(),
        B,
        json!({ "currency": "USD", "buyBp": 5, "sellBp": -5, "own": false, "follow": true }),
    )
    .await
    .ok("save USD");
    assert_eq!(
        room.branch_policy(B, "*").await,
        Some((Some(10), Some(-10), false, false))
    );
    assert_eq!(
        room.branch_policy(B, "USD").await,
        Some((Some(5), Some(-5), false, true))
    );
    room.finish().await;
}

/// §15.4 "A bp outside −300..300 is `validationFailed`" (`H:` §7): nothing is written.
#[tokio::test]
async fn v9_push_branch_policy_save_refuses_a_basis_point_outside_300() {
    let room = push_room(&[B]).await;
    for (what, body) in [
        (
            "buyBp 301",
            json!({ "currency": "USD", "buyBp": 301, "sellBp": 0 }),
        ),
        (
            "sellBp −301",
            json!({ "currency": "USD", "buyBp": 0, "sellBp": -301 }),
        ),
        ("'*' buyBp 301", json!({ "currency": "*", "buyBp": 301 })),
    ] {
        save_policy(&room, &owner(), B, body)
            .await
            .refused(400, "validationFailed", what);
    }
    assert_eq!(room.count("pricing_branch_policy").await, 0);
    room.finish().await;
}

/// §15.4: `pricingBranchPolicySave` is an EXECUTIVE frame change (`may_change_the_frame`, V.9's OWNER): a RATE_MANAGER,
/// who may approve, may not change it; nothing is written.
#[tokio::test]
async fn v9_push_branch_policy_save_needs_the_frame_right() {
    let room = push_room(&[B]).await;
    let a = save_policy(
        &room,
        &rate_manager(),
        B,
        json!({ "currency": "USD", "buyBp": 5, "sellBp": -5 }),
    )
    .await;
    let err = PolicyError::RoleNotAllowedHere;
    assert_eq!(a.status.as_u16(), err.status(), "{}", a.text);
    assert_eq!(
        a.code(),
        json!(V3Error::from(err).code),
        "a frame change: EXECUTIVE only (as pricing_suggestion_settings_save_test.rs)"
    );
    assert_eq!(room.count("pricing_branch_policy").await, 0);
    room.finish().await;
}
