//! XC-V9-PRICING PV-2, PV-3, PV-4, PV-5 and the stale-page guard (pricing-tiers.md §15.1, §15.2, §15.3, §15.8a,
//! §15.11): `pricingApprove` (`POST /api/pricing/approve`) at MAIN, MANUAL mode. The approved tiers **are** the counter
//! price (V-1): a `pricing_tier_sets` row `source='MANUAL'` with the anchor, a draft whose line carries the 4 tier rows,
//! and 4 published tier lines at exactly those prices. No margins over BoT, no EXECUTIVE wait (B3), at most 5 decimals
//! (B2), no BoT line needed (gap 7), and a later BoT refresh leaves the tiers alone (§15.1).
//! Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! `H:` is the owner's handoff (`PRICING-DESIGN.md`, kept out of the repo). PV-1's tiers are `H:` §4.4's.
//!
//! Readings (§15 leaves them open; each matches `H:`):
//! - reading: a refused tier set answers the whole request `400 validationFailed` (§15.3 step 2, `H:` §5), not a
//!   `200` with the currency `REFUSED`.
//! - reading: the stale-page guard on `rows[].tierSetId` answers `409 staleVersion`, the code the policy-version guard
//!   of the same command already answers (both are documented as "a newer one ⇒ `conflict`").
//! - reading: omitting `tierSetId` says "the page saw none" (the field's doc); with a set on record that is stale too.
//! - reading: the 4 published tier lines are coded `T1`..`T4`, the codes `PolicyTiers` publishes and
//!   `pricing_board.rs:256` reads as tier numbers.
//! - reading: the MAIN row's `request_id` carries the command's `Idempotency-Key` (§15.3 step 3 "keyed on
//!   `requestId`"; `rates.md` §5.6: the key is the command's `requestId`).

mod v9_approve_common;

use std::sync::Arc;

use serde_json::json;
use uuid::Uuid;
use v9_approve_common::*;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};
use xchange::infrastructure::rate_feed::branch_refresh::DraftBranchRefresh;
use xchange::infrastructure::rate_feed::feed_loop::BranchRefresh;
use xchange::presentation::http::v3::commands::make_approval_service;

/// A second USD set, one step wider than PV-1's (monotonic, tier 1 within 0.1 % of TT 33.20/33.90).
const S2: [(&str, &str); 4] = [
    ("33.22", "33.88"),
    ("33.23", "33.87"),
    ("33.25", "33.85"),
    ("33.28", "33.81"),
];
/// A third USD set, one step wider again.
const S3: [(&str, &str); 4] = [
    ("33.21", "33.89"),
    ("33.22", "33.88"),
    ("33.24", "33.86"),
    ("33.27", "33.82"),
];

/// Nothing of an approve reached the database at MAIN for `currency`.
async fn nothing_written(room: &Room, currency: &str, what: &str) {
    assert!(
        room.tier_sets(MAIN, currency).await.is_empty(),
        "{what}: no pricing_tier_sets row"
    );
    assert_eq!(room.count("rate_drafts").await, 0, "{what}: no draft");
    assert_eq!(
        room.count("rate_draft_line_tiers").await,
        0,
        "{what}: no draft tier row"
    );
    assert_eq!(
        room.current_publication(MAIN).await,
        None,
        "{what}: nothing published"
    );
}

fn owner_actor() -> Actor {
    let epoch = chrono::DateTime::<chrono::Utc>::UNIX_EPOCH;
    Actor::new(
        "owner",
        ORG,
        vec![
            ScopedRole::new(Role::Executive, None, epoch, None),
            ScopedRole::new(Role::Admin, None, epoch, None),
        ],
        None,
    )
}

fn drafts(room: &Room) -> Arc<RateDraftService> {
    Arc::new(RateDraftService::new(
        room.pool.clone(),
        make_approval_service(room.pool.clone()),
    ))
}

/// The owner publishes an all-`AUTO` `currency` line at MAIN through today's draft path (as
/// `rate_feed_loop_test.rs`'s `owner_publishes_auto_usd`).
async fn owner_publishes_auto(room: &Room, currency: &str) {
    let who = owner_actor();
    let b = BranchId::new(MAIN);
    let svc = drafts(room);
    let line = DraftLine::new(
        CurrencyCode::parse(currency).expect("a currency"),
        LineSide::Auto,
        LineSide::Auto,
        None,
    );
    let mut tx = room.pool.begin().await.expect("begin");
    let draft = svc
        .create(&mut tx, &b, &who, vec![line])
        .await
        .unwrap_or_else(|e| panic!("create: {}", e.code()));
    svc.submit(&mut tx, &b, &who, &draft.id, &unique("owner-key"))
        .await
        .unwrap_or_else(|e| panic!("submit: {}", e.code()));
    PublishService::new(room.pool.clone())
        .publish(&mut tx, &b, &who, &draft.id, None)
        .await
        .unwrap_or_else(|e| panic!("publish: {}", e.code()));
    tx.commit().await.expect("commit");
}

/// One BoT refresh of MAIN for `batch` (`DraftBranchRefresh`, the rate feed's branch refresh, §11.3).
async fn bot_refresh(room: &Room, batch: Uuid) {
    let refresh = DraftBranchRefresh::new(
        drafts(room),
        Arc::new(PublishService::new(room.pool.clone())),
    );
    let mut tx = room.pool.begin().await.expect("begin");
    refresh
        .refresh(&mut tx, &BranchId::new(MAIN), batch)
        .await
        .unwrap_or_else(|e| panic!("refresh: {}", e.code()));
    tx.commit().await.expect("commit");
}

// ------------------------------------------------------------ PV-2

/// PV-2 (§15.3, `H:` §4.4, §5; B3): approving PV-1's tiers at MAIN publishes them as they are — a `MANUAL` tier set
/// with the TT anchor, a draft line with 4 tier rows, 4 published tier lines at exactly those prices tagged with the
/// set — for a caller with `RatePolicySave` + `RatesDraft` and no frame right, although BoT is 7 % away (the margin
/// model would have sent this to EXECUTIVE).
#[tokio::test]
async fn v9_approve_manual_publishes_the_approved_tiers_as_the_counter_price_with_no_executive_wait()
 {
    let room = Room::start().await;
    room.policy(MAIN, true).await;
    let (tt, _) = room.tt(&[USD_TT]).await;
    // BoT far from the tiers: 33.23 / 31.00 = +7.2 % and 33.87 / 31.50 = +7.5 %, beyond the ±5 % guard. The margin
    // model (`pricing_approve_test.rs`) would leave a RATE_MANAGER's draft PENDING_APPROVAL; V.9 has no BoT guard (B3).
    room.bot(&[("USD", "31.0000", "31.5000")]).await;
    let key = unique("pv2");

    let a = approve(
        &room,
        &rate_manager(),
        MAIN,
        vec![row("USD", PV1_TIERS, None)],
        Some(&key),
    )
    .await;
    let out = a.ok("PV-2 approve");
    let usd = currency_result(&out, "USD");
    assert_eq!(usd["state"], json!("PUBLISHED"), "§15.8a state: {out}");
    assert!(usd["error"].is_null(), "no error: {out}");
    let id = tier_set_id_of(&usd);
    assert_eq!(
        usd["branches"],
        json!([]),
        "§15.8a: `branches` is present at MAIN; no other branch exists here"
    );

    // the tier set (§15.2, §15.3 step 3)
    let set = room.only_tier_set(MAIN, "USD").await;
    assert_eq!(set.id, id, "the answer's tierSetId is the row");
    assert_eq!(set.source, "MANUAL");
    assert_eq!(set.base_tier_set_id, None, "MAIN's own set has no base");
    assert!(
        set.request_id.contains(&key),
        "keyed on the requestId (the Idempotency-Key {key}): {}",
        set.request_id
    );
    assert_eq!(set.prices(), decs(PV1_TIERS), "the approved tiers, as sent");
    let anchor = set
        .anchor
        .clone()
        .unwrap_or_else(|| panic!("§15.3 step 3: the anchor is kept: {set:?}"));
    assert_eq!(
        (dec_of(&anchor["buy"]), dec_of(&anchor["sell"])),
        (dec("33.20"), dec("33.90")),
        "the anchor is the TT line: {anchor}"
    );
    assert_eq!(
        anchor["batch_id"],
        json!(tt.to_string()),
        "the TT batch: {anchor}"
    );
    assert!(
        anchor["source"]
            .as_str()
            .is_some_and(|s| s.to_uppercase().contains("TT")),
        "the anchor's source is TT: {anchor}"
    );

    // the draft (§15.3 steps 4–5)
    let draft = set
        .draft_id
        .unwrap_or_else(|| panic!("the set names its draft: {set:?}"));
    let status: String = sqlx::query_scalar("SELECT status FROM rate_drafts WHERE id = $1")
        .bind(draft)
        .fetch_one(&room.pool)
        .await
        .expect("the draft");
    assert_eq!(status, "PUBLISHED", "B3: in bound, published by policy");
    let draft_tiers: Vec<(i16, rust_decimal::Decimal, rust_decimal::Decimal)> = sqlx::query_as(
        "SELECT t.tier_no, t.buy, t.sell FROM rate_draft_line_tiers t \
         JOIN rate_draft_lines l ON l.id = t.draft_line_id \
         WHERE l.draft_id = $1 AND l.currency_code = 'USD' ORDER BY t.tier_no",
    )
    .bind(draft)
    .fetch_all(&room.pool)
    .await
    .expect("rate_draft_line_tiers");
    assert_eq!(
        draft_tiers
            .iter()
            .map(|(_, b, s)| (*b, *s))
            .collect::<Vec<_>>(),
        decs(PV1_TIERS),
        "§15.3 step 4: 4 tier rows on the USD line"
    );

    // the publication (§15.3 step 6)
    let lines = room.published(MAIN, "USD").await;
    assert_eq!(lines.len(), 4, "4 published tier lines for USD: {lines:?}");
    assert_eq!(
        room.published_prices(MAIN, "USD").await,
        decs(PV1_TIERS),
        "PV-2: published tier 1 = 33.23/33.87 exactly, and tiers 2–4 as approved"
    );
    assert_eq!(
        lines
            .iter()
            .map(|l| l.value_tier.as_str())
            .collect::<Vec<_>>(),
        vec!["T1", "T2", "T3", "T4"]
    );
    for l in &lines {
        assert_eq!(
            (
                &l.snapshot["pricing"]["tier_set_id"],
                &l.snapshot["pricing"]["source"]
            ),
            (&json!(id.to_string()), &json!("MANUAL")),
            "§15.3 step 6: source_snapshot.pricing on {}: {}",
            l.value_tier,
            l.snapshot
        );
    }
    room.finish().await;
}

/// PV-2 (§15.1): after the approve, a BoT refresh of MAIN re-prices the AUTO currency but copies USD's 4 tier prices
/// unchanged; BoT does not move a MANUAL currency.
#[tokio::test]
async fn v9_approve_a_later_bot_refresh_leaves_the_manual_tiers_unchanged() {
    let room = Room::start().await;
    room.policy(MAIN, false).await;
    room.tt(&[USD_TT]).await;
    room.bot(&[("USD", "32.4500", "32.9000"), ("EUR", "38.0000", "38.6000")])
        .await;
    owner_publishes_auto(&room, "EUR").await;
    let eur_before = room.published_prices(MAIN, "EUR").await;
    assert_eq!(eur_before.len(), 1, "one AUTO EUR line: {eur_before:?}");

    approve(
        &room,
        &rate_manager(),
        MAIN,
        vec![row("USD", PV1_TIERS, None)],
        None,
    )
    .await
    .ok("PV-2 approve");
    assert_eq!(room.published_prices(MAIN, "USD").await, decs(PV1_TIERS));
    assert_eq!(
        room.published_prices(MAIN, "EUR").await,
        eur_before,
        "the approve copies the other lines"
    );

    // BoT moves both: USD +4.8 %, EUR +1.3 %.
    let newer = room
        .bot(&[("USD", "34.0000", "34.5000"), ("EUR", "38.5000", "39.1000")])
        .await;
    bot_refresh(&room, newer).await;

    assert_ne!(
        room.published_prices(MAIN, "EUR").await,
        eur_before,
        "the refresh ran: the AUTO currency follows BoT"
    );
    assert_eq!(
        room.published_prices(MAIN, "USD").await,
        decs(PV1_TIERS),
        "§15.1: the refresh copies USD's tier prices, not re-priced from BoT"
    );
    let snapshot_sources: Vec<serde_json::Value> = room
        .published(MAIN, "USD")
        .await
        .iter()
        .map(|l| l.snapshot["pricing"]["source"].clone())
        .collect();
    assert_eq!(
        snapshot_sources,
        vec![json!("MANUAL"); 4],
        "the copied lines are still MANUAL"
    );
    room.finish().await;
}

/// PV-2 (§15.1): when the MANUAL currency is the branch's only line, a BoT refresh has nothing to change and writes
/// no draft; the publication stays.
#[tokio::test]
async fn v9_approve_a_bot_refresh_with_only_manual_lines_writes_no_draft() {
    let room = Room::start().await;
    room.policy(MAIN, false).await;
    room.tt(&[USD_TT]).await;
    room.bot(&[("USD", "32.4500", "32.9000")]).await;
    approve(
        &room,
        &rate_manager(),
        MAIN,
        vec![row("USD", PV1_TIERS, None)],
        None,
    )
    .await
    .ok("PV-2 approve");
    let publication = room.current_publication(MAIN).await;
    let drafts_before = room.drafts_of(MAIN).await;

    let newer = room.bot(&[("USD", "34.0000", "34.5000")]).await;
    bot_refresh(&room, newer).await;

    assert_eq!(
        room.drafts_of(MAIN).await,
        drafts_before,
        "§15.1: a branch whose priced currencies would be its only changes writes no draft"
    );
    assert_eq!(room.current_publication(MAIN).await, publication);
    assert_eq!(room.published_prices(MAIN, "USD").await, decs(PV1_TIERS));
    room.finish().await;
}

// ------------------------------------------------------------ PV-3

/// PV-3 (B2, `H:` §4.3: at most 5 decimals): a manual price with 6 decimals is `validationFailed` and nothing is
/// written. (`main_room` seeds BoT, so the margin model, which takes 8 decimals, would have published it.)
#[tokio::test]
async fn v9_approve_a_price_with_six_decimals_is_refused_and_nothing_is_written() {
    let room = main_room().await;
    let mut tiers = PV1_TIERS;
    tiers[0] = ("33.230001", "33.87");
    let a = approve(&room, &owner(), MAIN, vec![row("USD", tiers, None)], None).await;
    a.refused(400, "validationFailed", "PV-3: 6 decimals");
    nothing_written(&room, "USD", "6 decimals").await;
    room.finish().await;
}

/// PV-3 (B2, `H:` §4.3 "VND 0.00127"): 5 decimals are accepted and published in full, at every tier and both sides.
/// There is no VND BoT line (gap 7).
#[tokio::test]
async fn v9_approve_five_decimals_are_published_in_full() {
    let room = Room::start().await;
    room.policy(MAIN, true).await;
    // VND is unit 1. Tier 1 against the TT anchor: 0.00127 / 0.00126 − 1 = +0.79 % (buy) and
    // 0.00131 / 0.00132 − 1 = −0.76 % (sell), both inside 3 × farFromAnchorPct = 3 %. Buys never fall
    // (0.00127 ≤ 0.00128 ≤ 0.00128 ≤ 0.00129), sells never rise (0.00131 ≥ 0.00131 ≥ 0.00130 ≥ 0.00130), and every
    // sell is above its buy, so TierCheck passes.
    room.tt(&[("VND", "0.00126", "0.00132")]).await;
    const VND: [(&str, &str); 4] = [
        ("0.00127", "0.00131"),
        ("0.00128", "0.00131"),
        ("0.00128", "0.00130"),
        ("0.00129", "0.00130"),
    ];
    let out = approve(&room, &owner(), MAIN, vec![row("VND", VND, None)], None)
        .await
        .ok("PV-3 VND approve");
    assert_eq!(
        currency_result(&out, "VND")["state"],
        json!("PUBLISHED"),
        "{out}"
    );
    assert_eq!(
        room.only_tier_set(MAIN, "VND").await.prices(),
        decs(VND),
        "the set keeps 5 decimals"
    );
    assert_eq!(
        room.published_prices(MAIN, "VND").await,
        decs(VND),
        "published in full: 0.00127, not 0.0013"
    );
    room.finish().await;
}

// ------------------------------------------------------------ PV-4

/// PV-4 (gap 7, §15.3 step 5): a currency with no BoT line and no organisation starting price, but a TT line, is
/// approved and published.
#[tokio::test]
async fn v9_approve_a_currency_with_no_bot_line_and_no_starting_price_publishes() {
    let room = Room::start().await;
    room.policy(MAIN, true).await;
    // GBP tier 1 against TT: 42.05 / 42.00 − 1 = +0.12 %, 42.55 / 42.60 − 1 = −0.12 %.
    room.tt(&[("GBP", "42.00", "42.60")]).await;
    const GBP: [(&str, &str); 4] = [
        ("42.05", "42.55"),
        ("42.06", "42.54"),
        ("42.08", "42.52"),
        ("42.10", "42.50"),
    ];
    let no_base: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM rate_reference_lines WHERE currency_code = 'GBP' AND reference_type IN ('BOT', 'START')) \
              + (SELECT count(*) FROM org_starting_prices WHERE currency_code = 'GBP')",
    )
    .fetch_one(&room.pool)
    .await
    .expect("count GBP bases");
    assert_eq!(
        no_base, 0,
        "the case: GBP has no BoT line and no starting price"
    );

    let out = approve(&room, &owner(), MAIN, vec![row("GBP", GBP, None)], None)
        .await
        .ok("PV-4 approve");
    assert_eq!(
        currency_result(&out, "GBP")["state"],
        json!("PUBLISHED"),
        "{out}"
    );
    assert_eq!(room.published_prices(MAIN, "GBP").await, decs(GBP));
    room.finish().await;
}

// ------------------------------------------------------------ PV-5

/// PV-5 (§15.3 step 2, `H:` §5): tier 1 more than 3 % from the TT anchor, a buy tier below the previous one, and a
/// sell not above its buy are each `validationFailed`, and none leaves a trace — so the corrected set that follows
/// is the first tier set at MAIN (approved with no `tierSetId`) and publishes.
#[tokio::test]
async fn v9_approve_each_tier_check_failure_is_refused_and_leaves_nothing_behind() {
    let room = main_room().await;
    // (a) tier 1 buy 34.25 / 33.20 − 1 = +3.16 % > 3 % (sell 34.90 / 33.90 − 1 = +2.95 %); the tiers are otherwise
    //     monotonic with sell above buy, so only the anchor check fails.
    let far: [(&str, &str); 4] = [
        ("34.25", "34.90"),
        ("34.26", "34.89"),
        ("34.27", "34.88"),
        ("34.28", "34.87"),
    ];
    // (b) buy tier 2 33.22 < tier 1 33.23.
    let mut buy_down = PV1_TIERS;
    buy_down[1] = ("33.22", "33.86");
    // (c) tier 4 sell = buy.
    let mut sell_not_above = PV1_TIERS;
    sell_not_above[3] = ("33.29", "33.29");

    for (what, tiers) in [
        ("tier 1 more than 3 % from TT", far),
        ("buy tier 2 below tier 1", buy_down),
        ("sell not above buy", sell_not_above),
    ] {
        approve(&room, &owner(), MAIN, vec![row("USD", tiers, None)], None)
            .await
            .refused(400, "validationFailed", what);
        nothing_written(&room, "USD", what).await;
    }

    let out = approve(
        &room,
        &owner(),
        MAIN,
        vec![row("USD", PV1_TIERS, None)],
        None,
    )
    .await
    .ok("the corrected set");
    assert_eq!(
        currency_result(&out, "USD")["state"],
        json!("PUBLISHED"),
        "{out}"
    );
    assert_eq!(
        room.only_tier_set(MAIN, "USD").await.prices(),
        decs(PV1_TIERS)
    );
    room.finish().await;
}

// ------------------------------------------------------------ stale-page guard

/// §15.3 "The stale-page guard stays": a row whose `tierSetId` is older than MAIN's latest set for the currency (or
/// absent while one exists) answers 409, and nothing is written; the latest id is accepted.
#[tokio::test]
async fn v9_approve_a_tier_set_id_older_than_the_latest_is_a_conflict_and_writes_nothing() {
    let room = main_room().await;
    let first = approve(
        &room,
        &owner(),
        MAIN,
        vec![row("USD", PV1_TIERS, None)],
        None,
    )
    .await
    .ok("first approve");
    let id1 = tier_set_id_of(&currency_result(&first, "USD"));
    let second = approve(
        &room,
        &owner(),
        MAIN,
        vec![row("USD", S2, Some(&id1.to_string()))],
        None,
    )
    .await
    .ok("built on the latest set");
    let id2 = tier_set_id_of(&currency_result(&second, "USD"));
    assert_ne!(id1, id2);

    let publication = room.current_publication(MAIN).await;
    let drafts = room.count("rate_drafts").await;
    for (what, seen) in [
        ("built on the older set", Some(id1.to_string())),
        ("built on no set while one exists", None),
    ] {
        approve(
            &room,
            &owner(),
            MAIN,
            vec![row("USD", S3, seen.as_deref())],
            None,
        )
        .await
        .refused(409, "staleVersion", what);
        assert_eq!(
            room.tier_sets(MAIN, "USD")
                .await
                .iter()
                .map(|s| s.id)
                .collect::<Vec<_>>(),
            vec![id1, id2],
            "{what}: no new set"
        );
        assert_eq!(
            room.current_publication(MAIN).await,
            publication,
            "{what}: nothing published"
        );
        assert_eq!(room.count("rate_drafts").await, drafts, "{what}: no draft");
    }
    assert_eq!(room.published_prices(MAIN, "USD").await, decs(S2));
    room.finish().await;
}
