//! PT-6 (pricing-tiers.md §2, §5.2, §6, §11; owner P-3 "Use xchange's flow"): `pricingApprove`
//! (`POST /api/pricing/approve`). For 1..n currencies: one new policy version with their margins (through
//! `PolicyVersionWriter`), then one rate draft with one AUTO line per currency, submitted; in bound ⇒ published by
//! policy, out of bound ⇒ waits in A06 for EXECUTIVE. Written before the implementation by the ticket author
//! (claude/647e1967, PT-0). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Choices pinned here (the implementer follows them):
//! - Each approved tier price becomes the margin that produces it over the draft path's base (§2, §5.2):
//!   `margin = (price ÷ base − 1) × 100 − adjust_pct`, rounded to scale 4, per side, where base is the currency's
//!   `ReferenceBase::newest_in` line at its unit size. A currency with no base ⇒ 400 `validationFailed`.
//! - `[decided by BE controller 2026-10-20]` (claude/a90ab16c, reading A, recorded on #3827): approve is **not** a frame change. Any caller with
//!   `RatePolicySave` + `RatesDraft` may approve (V.9's `pricing.tiers:approve` is FULL, its `pricing.policy:write` is
//!   owner-only), the margins are written without the save-time guard refusal, and the guard decides through the
//!   draft: in bound ⇒ `PUBLISHED`, out of bound ⇒ `PENDING_APPROVAL` (§11's two approve rows).
//! - A `policyVersionId` that is not the branch's current version ⇒ 409 `staleVersion`, nothing written.
//! - The `Idempotency-Key` header is the command's `requestId` (`rates.md` §5.6): the same key twice ⇒ one version,
//!   one draft, the same answer.
//!
//! Prices are §12.3's worked example (USD BoT 32.4500 / 32.9000, adjust −0.5 / +0.4, see `pricing_common`).
//! Runs once PT-1, PT-2, PT-3 and PT-4 are on `feat/pricing-tiers`.

mod pricing_common;

use pricing_common::*;
use serde_json::{Value, json};
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xchange::presentation::http::v3::error::V3Error;

const ROUTE: &str = "/api/pricing/approve";

/// USD prices whose margins are T1 +0.4/−0.4, T2 +0.45/−0.45, T3 +0.5/−0.5, T4 +0.6/−0.6.
const USD_PRICES: [(&str, &str); 4] = [
    ("32.41755", "32.90"),
    ("32.433775", "32.88355"),
    ("32.45", "32.8671"),
    ("32.48245", "32.8342"),
];
const USD_MARGINS: [(&str, &str); 4] = [
    ("0.4", "-0.4"),
    ("0.45", "-0.45"),
    ("0.5", "-0.5"),
    ("0.6", "-0.6"),
];

/// EUR prices at the branch's own margins (T1 0/0 … T4 +0.3/−0.3).
const EUR_PRICES: [(&str, &str); 4] = [
    ("37.81", "38.7544"),
    ("37.848", "38.7158"),
    ("37.886", "38.6772"),
    ("37.924", "38.6386"),
];

fn row(currency: &str, prices: [(&str, &str); 4]) -> Value {
    json!({
        "currency": currency,
        "tiers": prices.iter().enumerate()
            .map(|(i, (b, s))| json!({ "tierNo": i + 1, "buy": b, "sell": s }))
            .collect::<Vec<_>>(),
    })
}

fn body(v1: uuid::Uuid, rows: Vec<Value>) -> Value {
    json!({ "policyVersionId": v1.to_string(), "rows": rows, "reason": "PT-6 test" })
}

async fn nothing_written(room: &Room, what: &str) {
    assert_eq!(room.versions(B01).await.len(), 1, "{what}: no new version");
    assert_eq!(
        room.count("rate_policy_currency_tiers").await,
        0,
        "{what}: no override row"
    );
    assert_eq!(room.count("rate_drafts").await, 0, "{what}: no draft");
}

#[tokio::test]
async fn approve_in_bound_writes_a_version_with_the_margins_and_publishes_the_prices() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;

    let a = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, vec![row("USD", USD_PRICES)]),
        None,
    )
    .await;
    let out = a.ok("approve");
    let v2 = room.current_version(B01).await;
    assert_eq!(out["policyVersionId"], json!(v2.to_string()), "{}", a.text);
    assert_eq!(out["version"], json!(2));
    assert_eq!(
        out["draftState"],
        json!("PUBLISHED"),
        "§11: approve, in bound ⇒ published"
    );

    let margins: Vec<_> = room
        .override_rows(v2)
        .await
        .into_iter()
        .map(|(_, _, b, s)| (b, s))
        .collect();
    assert_eq!(
        margins,
        USD_MARGINS
            .iter()
            .map(|(b, s)| (dec(b), dec(s)))
            .collect::<Vec<_>>()
    );

    let status: String = sqlx::query_scalar("SELECT status FROM rate_drafts WHERE id::text = $1")
        .bind(out["draftId"].as_str().expect("draftId"))
        .fetch_one(&room.pool)
        .await
        .expect("the draft");
    assert_eq!(status, "PUBLISHED");
    assert_eq!(
        room.current_rates(B01, "USD").await,
        vec![
            (
                "T1".to_string(),
                "32.41755000".to_string(),
                "32.90000000".to_string()
            ),
            (
                "T2".to_string(),
                "32.43377500".to_string(),
                "32.88355000".to_string()
            ),
            (
                "T3".to_string(),
                "32.45000000".to_string(),
                "32.86710000".to_string()
            ),
            (
                "T4".to_string(),
                "32.48245000".to_string(),
                "32.83420000".to_string()
            ),
        ],
        "the published prices are the approved ones"
    );
    room.finish().await;
}

#[tokio::test]
async fn bulk_approve_writes_one_version_and_one_draft_for_every_currency() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let a = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, vec![row("USD", USD_PRICES), row("EUR", EUR_PRICES)]),
        None,
    )
    .await;
    a.ok("bulk approve");
    assert_eq!(room.versions(B01).await.len(), 2, "one new version");
    assert_eq!(room.count("rate_drafts").await, 1, "one draft");
    let lines: i64 = sqlx::query_scalar("SELECT count(*) FROM rate_draft_lines")
        .fetch_one(&room.pool)
        .await
        .expect("draft lines");
    assert_eq!(lines, 2, "one AUTO line per currency");
    let v2 = room.current_version(B01).await;
    let eur: Vec<_> = room
        .override_rows(v2)
        .await
        .into_iter()
        .filter(|r| r.0 == "EUR")
        .map(|r| (r.2, r.3))
        .collect();
    assert_eq!(
        eur,
        vec![
            (dec("0"), dec("0")),
            (dec("0.1"), dec("-0.1")),
            (dec("0.2"), dec("-0.2")),
            (dec("0.3"), dec("-0.3"))
        ]
    );
    assert_eq!(
        room.current_rates(B01, "EUR").await[0],
        (
            "T1".to_string(),
            "37.81000000".to_string(),
            "38.75440000".to_string()
        )
    );
    room.finish().await;
}

#[tokio::test]
async fn approve_out_of_bound_waits_for_executive() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    // T4 sell 34.6766 = 32.9 × 1.054 ⇒ margin 5.0, adjust + margin = 5.4 > USD's 5 (buy stays below sell).
    // A RATE_MANAGER approves: an EXECUTIVE proposer self-approves an out-of-bound draft (rates.md §4.5).
    let mut prices = USD_PRICES;
    prices[3] = ("32.48245", "34.6766");
    let a = command(
        &room,
        &rate_manager(),
        ROUTE,
        B01,
        body(v1, vec![row("USD", prices)]),
        None,
    )
    .await;
    let out = a.ok("approve out of bound");
    assert_eq!(
        out["draftState"],
        json!("PENDING_APPROVAL"),
        "§11: approve, out of bound ⇒ waits in A06"
    );
    assert_eq!(room.versions(B01).await.len(), 2, "the version is written");
    assert!(
        room.current_rates(B01, "USD").await.is_empty(),
        "nothing published"
    );
    room.finish().await;
}

#[tokio::test]
async fn a_rate_manager_may_approve_in_bound() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let out = command(
        &room,
        &rate_manager(),
        ROUTE,
        B01,
        body(v1, vec![row("USD", USD_PRICES)]),
        None,
    )
    .await
    .ok("RATE_MANAGER approves");
    assert_eq!(out["draftState"], json!("PUBLISHED"));
    room.finish().await;
}

#[tokio::test]
async fn a_stale_policy_version_is_refused_and_nothing_is_written() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let a = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(uuid::Uuid::new_v4(), vec![row("USD", USD_PRICES)]),
        None,
    )
    .await;
    a.refused(
        409,
        "staleVersion",
        "§11: approve with a stale policy_version_id",
    );
    nothing_written(&room, "stale").await;

    command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, vec![row("EUR", EUR_PRICES)]),
        None,
    )
    .await
    .ok("first approve");
    let a = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, vec![row("USD", USD_PRICES)]),
        None,
    )
    .await;
    a.refused(409, "staleVersion", "built on v1 after v2");
    assert_eq!(room.versions(B01).await.len(), 2);
    assert_eq!(room.count("rate_drafts").await, 1);
    room.finish().await;
}

#[tokio::test]
async fn the_same_request_twice_writes_once() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let key = unique("approve");
    let first = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, vec![row("USD", USD_PRICES)]),
        Some(&key),
    )
    .await
    .ok("first");
    let second = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, vec![row("USD", USD_PRICES)]),
        Some(&key),
    )
    .await
    .ok("second");
    assert_eq!(
        first, second,
        "§11: approve twice with one requestId ⇒ the same answer"
    );
    assert_eq!(room.versions(B01).await.len(), 2, "one version");
    assert_eq!(room.count("rate_drafts").await, 1, "one draft");
    room.finish().await;
}

#[tokio::test]
async fn a_caller_without_rate_policy_save_is_forbidden() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let a = command(
        &room,
        &teller(),
        ROUTE,
        B01,
        body(v1, vec![row("USD", USD_PRICES)]),
        None,
    )
    .await;
    // The permission layer refuses a role outside the command's row (`ratePolicySave`'s, inherited) as it does
    // for `ratePolicySave` itself: `roleNotAllowedHere` with its status.
    let err = PolicyError::RoleNotAllowedHere;
    assert_eq!(
        a.status.as_u16(),
        err.status(),
        "§11: user without RatePolicySave; body {}",
        a.text
    );
    assert_eq!(a.code(), json!(V3Error::from(err).code), "{}", a.text);
    nothing_written(&room, "TELLER").await;
    room.finish().await;
}

#[tokio::test]
async fn bad_rows_are_refused_and_nothing_is_written() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;

    let a = command(&room, &owner(), ROUTE, B01, body(v1, vec![]), None).await;
    a.refused(400, "validationFailed", "no rows");

    let mut three = row("USD", USD_PRICES);
    three["tiers"].as_array_mut().unwrap().pop();
    let a = command(&room, &owner(), ROUTE, B01, body(v1, vec![three]), None).await;
    a.refused(400, "validationFailed", "3 tiers");

    // GBP has no BoT line and no START line: no base, so no margin (§5.2 "no base ⇒ no margin").
    let a = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, vec![row("USD", USD_PRICES), row("GBP", USD_PRICES)]),
        None,
    )
    .await;
    a.refused(400, "validationFailed", "a currency with no base");

    nothing_written(&room, "bad rows").await;
    room.finish().await;
}
