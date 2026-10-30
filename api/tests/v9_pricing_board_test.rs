//! PV-20, the §15.8a board fields and `marketRatesRefresh` (pricing-tiers.md §15.5 gap 6, §15.8a, §15.9; `H:` §6,
//! §8).
//! Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! - PV-20: `pricing_board` is versioned through `own_version` (B6): read twice with nothing changed, the second
//!   answer (`?since=<version>`, `view.rs:626-660`) is `{changed:false}`; each of a `pricing_tier_sets` insert, a
//!   `pricingModeSave`, a `pricingBranchPolicySave` and a `rateDisplaySave` moves the version; and TT turning stale —
//!   30 minutes passing with no write — moves it too.
//! - §15.8a: a row's `mode`, `tierSet`, `followResult`, `branchPolicy`; the top level's `branchDefault`, `display`,
//!   `isMain`.
//! - `marketRatesRefresh` (`POST /api/pricing/market-refresh`): with fake TT and SR sources injected through
//!   `AppState::with_market_sources`, it answers each source's fetch and, at MAIN, one result per FOLLOW currency; a
//!   caller without `MarketRatesRefresh` is refused and nothing is fetched.

mod v9_follow_common;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use actix_web::http::Method;
use chrono::Utc;
use serde_json::{Value, json};
use v9_follow_common::*;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_rate::domain::pricing::market_quote::MarketSource;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xchange::presentation::http::v3::error::V3Error;

const BR_B: &str = "BR-B";
const ROUTE_BOARD: &str = "pricing_board";

/// `(version, changed)` of a `pricing_board` read at `branch`, with `since` when given.
async fn board_version(room: &Room, branch: &str, since: Option<i64>) -> (i64, bool) {
    let a = view(room, &owner(), ROUTE_BOARD, branch, since).await;
    let body = a.ok("pricing_board");
    (
        body["version"]
            .as_i64()
            .unwrap_or_else(|| panic!("a version: {body}")),
        body["changed"]
            .as_bool()
            .unwrap_or_else(|| panic!("changed: {body}")),
    )
}

async fn board(room: &Room, branch: &str) -> Value {
    let a = view(room, &owner(), ROUTE_BOARD, branch, None).await;
    a.ok("pricing_board")["data"].clone()
}

fn row<'a>(data: &'a Value, code: &str) -> &'a Value {
    data["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("rows is an array: {data}"))
        .iter()
        .find(|r| r["currency"] == json!(code))
        .unwrap_or_else(|| panic!("a row for {code}: {}", data["rows"]))
}

/// A write moved the version: a read since `before` is `changed: true` with another version, and a read since that
/// new version is `changed: false`. Answers the new version.
async fn moved(room: &Room, before: i64, what: &str) -> i64 {
    let (after, changed) = board_version(room, MAIN, Some(before)).await;
    assert!(changed, "{what} changes the board");
    assert_ne!(after, before, "{what} moves the version");
    let (again, changed_again) = board_version(room, MAIN, Some(after)).await;
    assert!(!changed_again, "{what}: then nothing changed");
    assert_eq!(again, after);
    after
}

// ------------------------------------------------------------ PV-20

/// PV-20 (B6): the second of two reads with nothing changed is `{changed:false}`; then each pricing write — a
/// `pricing_tier_sets` insert, `pricingModeSave`, `pricingBranchPolicySave`, `rateDisplaySave` — moves the version
/// (§15.9's hash inputs).
#[tokio::test]
async fn pricing_board_is_versioned_and_each_pricing_write_moves_the_version() {
    let room = Room::start().await;
    let now = whole_second_now();
    room.policy(MAIN).await;
    room.bot(&[("USD", USD_BOT.0, USD_BOT.1)]).await;
    room.tt(now - minutes(2), &[("USD", PV1_TT.0, PV1_TT.1)])
        .await;

    let (v0, first_changed) = board_version(&room, MAIN, None).await;
    assert!(first_changed, "a first read carries the data");
    let (same, changed) = board_version(&room, MAIN, Some(v0)).await;
    assert!(
        !changed,
        "§15.9: an unchanged read answers {{changed:false}}"
    );
    assert_eq!(same, v0);

    room.exec(
        "INSERT INTO pricing_tier_sets (id, request_id, branch_id, currency_code, source, tiers, approved_by, approved_at) \
         VALUES (gen_random_uuid(), 'pv20-manual', 'MAIN', 'USD', 'MANUAL', \
             '[{\"tier\":1,\"buy\":\"33.23\",\"sell\":\"33.87\"},{\"tier\":2,\"buy\":\"33.24\",\"sell\":\"33.86\"},\
               {\"tier\":3,\"buy\":\"33.26\",\"sell\":\"33.84\"},{\"tier\":4,\"buy\":\"33.29\",\"sell\":\"33.80\"}]'::jsonb, \
             'owner-1', now())",
    )
    .await;
    let v1 = moved(&room, v0, "a pricing_tier_sets insert").await;

    follow_on(&room, "USD").await;
    let v2 = moved(&room, v1, "pricingModeSave").await;

    command(
        &room,
        &owner(),
        BRANCH_POLICY,
        MAIN,
        json!({ "currency": "*", "buyBp": 5, "sellBp": -5 }),
    )
    .await
    .ok("pricingBranchPolicySave");
    let v3 = moved(&room, v2, "pricingBranchPolicySave").await;

    command(
        &room,
        &owner(),
        DISPLAY,
        MAIN,
        json!({ "tiers": [1, 2], "featured": ["USD"], "highlight": true, "allMarket": false }),
    )
    .await
    .ok("rateDisplaySave");
    moved(&room, v3, "rateDisplaySave").await;
    room.finish().await;
}

/// PV-20 (B6): TT's stale boolean is part of the version (§15.9), so TT turning stale — 30 minutes passing with no
/// write — moves it. TT is seeded 30 minutes minus 6 seconds before the first read: the first two reads see it fresh
/// (`{changed:false}`), the read after the 30-minute mark sees a new version.
///
/// reading: the views read the wall clock (no clock seam reaches `own_version`), so "now passing 30 minutes" is the
/// real clock crossing the mark: this test waits for it, at most ~7 s, instead of writing anything.
#[tokio::test]
async fn tt_turning_stale_with_no_write_moves_the_board_version() {
    let room = Room::start().await;
    room.policy(MAIN).await;
    room.bot(&[("USD", USD_BOT.0, USD_BOT.1)]).await;
    let t0 = Utc::now();
    let fetched_at = t0 - minutes(30) + chrono::Duration::seconds(6);
    room.tt(fetched_at, &[("USD", PV1_TT.0, PV1_TT.1)]).await;
    let flips_at = fetched_at + minutes(30);

    let (v0, _) = board_version(&room, MAIN, None).await;
    let (same, changed) = board_version(&room, MAIN, Some(v0)).await;
    assert!(
        Utc::now() < flips_at,
        "the two fresh reads took longer than 6 s; the harness is too slow for this test"
    );
    assert!(!changed, "§15.9: nothing changed yet");
    assert_eq!(same, v0);

    while Utc::now() <= flips_at + chrono::Duration::milliseconds(500) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let (after, changed) = board_version(&room, MAIN, Some(v0)).await;
    assert!(changed, "TT turned stale with no write: a new version");
    assert_ne!(after, v0);
    room.finish().await;
}

// ------------------------------------------------------------ §15.8a

/// §15.8a at MAIN and at a followed branch: MAIN publishes FOLLOW USD; B has a `'*'` row +2 / −2 and a USD row
/// +5 / −5 with `follow`. MAIN's board: `isMain: true`, USD `mode: "FOLLOW"`, `tierSet.source: "FOLLOW"` with PV-1's
/// 4 tiers, `followResult.status: "PUBLISHED"`, `branchPolicy.from: "default"`, `branchDefault` 0 / 0. B's board:
/// `isMain: false`, `branchDefault` 2 / −2, USD `mode: "FOLLOW"` (it mirrors MAIN, §15.1) with B's own FOLLOW set and
/// MAIN's result, `branchPolicy` `{5, −5, own false, follow true, from "currency"}`; EUR (a BoT line only) reads the
/// `'*'` row: `{2, −2, from "*"}`.
#[tokio::test]
async fn the_board_shows_mode_tier_set_follow_result_branch_policy_and_is_main() {
    let room = Room::start().await;
    let now = whole_second_now();
    room.create_branch(BR_B).await;
    room.policy(BR_B).await;
    room.bot(&[("EUR", "38.00", "38.60")]).await;
    follow_usd_at_main(&room, now, 3).await;
    enrol(&room, BR_B, "USD", 5, -5).await;
    command(
        &room,
        &owner(),
        BRANCH_POLICY,
        BR_B,
        json!({ "currency": "*", "buyBp": 2, "sellBp": -2 }),
    )
    .await
    .ok("the '*' row");
    pricing_loop(&room.pool)
        .plan_once(now)
        .await
        .unwrap_or_else(|e| panic!("plan_once: {}", e.code()));

    let main = board(&room, MAIN).await;
    assert_eq!(main["isMain"], json!(true));
    assert_eq!(main["branchDefault"], json!({ "buyBp": 0, "sellBp": 0 }));
    assert_eq!(main["display"]["from"], json!("default"));
    let usd_main = row(&main, "USD");
    assert_eq!(usd_main["mode"], json!("FOLLOW"), "{usd_main}");
    assert_eq!(usd_main["tierSet"]["source"], json!("FOLLOW"), "{usd_main}");
    let tiers: Vec<_> = usd_main["tierSet"]["tiers"]
        .as_array()
        .unwrap_or_else(|| panic!("tierSet.tiers: {usd_main}"))
        .iter()
        .map(|t| (dec_of(&t["buy"]), dec_of(&t["sell"])))
        .collect();
    assert_eq!(tiers, pairs(&PV1_TIERS));
    assert_eq!(usd_main["followResult"]["status"], json!("PUBLISHED"));
    assert_eq!(usd_main["followResult"]["reasons"], json!([]));
    assert_eq!(usd_main["branchPolicy"]["from"], json!("default"));

    let b = board(&room, BR_B).await;
    assert_eq!(b["isMain"], json!(false));
    assert_eq!(b["branchDefault"], json!({ "buyBp": 2, "sellBp": -2 }));
    let usd_b = row(&b, "USD");
    assert_eq!(usd_b["mode"], json!("FOLLOW"), "{usd_b}");
    assert_eq!(usd_b["tierSet"]["source"], json!("FOLLOW"), "{usd_b}");
    assert_eq!(
        dec_of(&usd_b["tierSet"]["tiers"][0]["buy"]),
        dec("33.24"),
        "B's own set (H: §7): {usd_b}"
    );
    assert_eq!(
        usd_b["followResult"]["status"],
        json!("PUBLISHED"),
        "MAIN's"
    );
    assert_eq!(
        usd_b["branchPolicy"],
        json!({ "buyBp": 5, "sellBp": -5, "own": false, "follow": true, "from": "currency" })
    );
    let eur_b = row(&b, "EUR");
    assert_eq!(
        eur_b["branchPolicy"],
        json!({ "buyBp": 2, "sellBp": -2, "own": false, "follow": false, "from": "*" })
    );
    assert_eq!(eur_b["mode"], json!("AUTO"), "{eur_b}");
    assert_eq!(eur_b["tierSet"], Value::Null);
    room.finish().await;
}

// ------------------------------------------------------------ marketRatesRefresh (gap 6)

fn fake_sources() -> (MarketSources, Arc<FakeMarket>) {
    let tt = Arc::new(FakeMarket::new("TT", &[("USD", PV1_TT.0, PV1_TT.1)]));
    let sr = Arc::new(FakeMarket::new("SR", &[("USD", "33.33", "33.40")]));
    let sources: MarketSources = vec![
        (MarketSource::Tt, tt.clone() as Arc<dyn RateSource>),
        (MarketSource::SuperRich, sr as Arc<dyn RateSource>),
    ];
    (sources, tt)
}

async fn refresh(room: &Room, who: &Who, branch: &str) -> (Answer, Arc<FakeMarket>) {
    let (sources, tt) = fake_sources();
    let a = call_in(
        state(room).with_market_sources(sources),
        Some(who),
        Method::POST,
        MARKET_REFRESH,
        Some(json!({ "branch": branch })),
        None,
    )
    .await;
    (a, tt)
}

/// gap 6 (PV-g): `marketRatesRefresh` at MAIN fetches TT then SR through `AppState::market_sources` and answers
/// `sources: [{source:"TT", ok:true}, {source:"SR", ok:true}]`; with FOLLOW on for USD, it plans USD at once and
/// answers its result (a fresh fetch of PV-1's quote ⇒ `PUBLISHED`).
#[tokio::test]
async fn market_rates_refresh_fetches_each_source_and_plans_follow_at_main() {
    let room = Room::start().await;
    room.policy(MAIN).await;
    room.bot(&[("USD", USD_BOT.0, USD_BOT.1)]).await;
    follow_on(&room, "USD").await;

    let (a, tt) = refresh(&room, &owner(), MAIN).await;
    let out = a.ok("marketRatesRefresh");
    assert_eq!(tt.calls.load(Ordering::SeqCst), 1, "TT fetched once");
    let sources = out["sources"]
        .as_array()
        .unwrap_or_else(|| panic!("sources: {}", a.text));
    let named: Vec<(Value, Value)> = sources
        .iter()
        .map(|s| (s["source"].clone(), s["ok"].clone()))
        .collect();
    assert_eq!(
        named,
        vec![(json!("TT"), json!(true)), (json!("SR"), json!(true))],
        "{}",
        a.text
    );
    assert_eq!(
        room.count("rate_reference_batches WHERE provider = 'TT'")
            .await,
        1
    );
    assert_eq!(
        room.count("rate_reference_batches WHERE provider = 'SR'")
            .await,
        1
    );

    // reading: H: §6 needs `fetched_at = last_attempt_at` for a current source; a fetch the command just made is
    // current, so the plan it runs right after publishes.
    let results = out["followResults"]
        .as_array()
        .unwrap_or_else(|| panic!("followResults: {}", a.text));
    assert_eq!(results.len(), 1, "one per FOLLOW currency: {}", a.text);
    assert_eq!(results[0]["currency"], json!("USD"));
    assert_eq!(results[0]["status"], json!("PUBLISHED"), "{}", a.text);
    assert_eq!(
        room.follow_result("USD").await.map(|r| r.status),
        Some("PUBLISHED".to_string())
    );
    room.finish().await;
}

/// gap 6 (PV-g), restated for #5558 N-3 (claude/553782, #5558 issuecomment-6515987193): `marketRatesRefresh` is an
/// organisation-level row, so naming branch B runs it at the org-wide holder's session branch (MAIN here) and the
/// FOLLOW plan is made; a TELLER (no `MarketRatesRefresh`) is refused `roleNotAllowedHere` by the seam and nothing
/// is fetched. The `MAIN` gate itself is deleted by a follow-up ticket.
#[tokio::test]
async fn market_rates_refresh_naming_another_branch_plans_at_the_session_branch() {
    let room = Room::start().await;
    room.create_branch(BR_B).await;
    room.policy(BR_B).await;
    room.policy(MAIN).await;
    room.bot(&[("USD", USD_BOT.0, USD_BOT.1)]).await;

    let (refused, tt) = refresh(&room, &teller(), MAIN).await;
    let err = PolicyError::RoleNotAllowedHere;
    assert_eq!(refused.status.as_u16(), err.status(), "{}", refused.text);
    assert_eq!(
        refused.code(),
        json!(V3Error::from(err).code),
        "{}",
        refused.text
    );
    assert_eq!(tt.calls.load(Ordering::SeqCst), 0, "nothing fetched");
    assert_eq!(room.count("market_feed_status").await, 0);
    assert_eq!(
        room.count("rate_reference_batches WHERE provider IN ('TT', 'SR')")
            .await,
        0
    );

    follow_on(&room, "USD").await;

    let (at_b, _) = refresh(&room, &owner(), BR_B).await;
    let out = at_b.ok("marketRatesRefresh naming B");
    assert_eq!(
        out["followResults"].as_array().map(Vec::len),
        Some(1),
        "{}",
        at_b.text
    );
    assert!(
        room.follow_result("USD").await.is_some(),
        "planned at the session branch"
    );
    room.finish().await;
}
