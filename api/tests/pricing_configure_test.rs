//! PT-5 (pricing-tiers.md §3, §3.1, §6, §11): `pricingConfigure` (`POST /api/pricing/configure`) saves one currency's
//! override margins, minimum spread and anchor as a new policy version through `PolicyVersionWriter` (no draft).
//! A frame change: EXECUTIVE only. Written before the implementation by the ticket author (claude/647e1967, PT-0).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Choices pinned here (the implementer follows them):
//! - A `policyVersionId` that is not the branch's current version ⇒ 409 `staleVersion` (v3's lost-update code,
//!   rates.md §5.6), nothing written.
//! - Bad input (not four tiers 1..=4, a margin that is not a scale-4 percent, min spread outside 0..=5, an unknown
//!   anchor) ⇒ 400 `validationFailed`, nothing written.
//! - Each override margin is checked against **that currency's** guard of the current version
//!   (`adjust + margin` inside `[min_pct, max_pct]`, both sides, every tier) ⇒ 400 `validationFailed`.
//! - The new version keeps every other field of the current one (adjusts, guards, stale settings, value tiers, the
//!   branch anchor) and every other currency's override.
//! - `tiers`, `minSpreadPct` and `anchorSource` all `null` ⇒ the currency's override is removed.
//!
//! Runs once PT-1 (writer), PT-2 and PT-3 are on `feat/pricing-tiers`.

mod pricing_common;

use pricing_common::*;
use serde_json::{Value, json};
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xchange::presentation::http::v3::error::V3Error;

const ROUTE: &str = "/api/pricing/configure";

fn tiers(margins: [(&str, &str); 4]) -> Value {
    json!(
        margins
            .iter()
            .enumerate()
            .map(|(i, (b, s))| json!({ "tierNo": i + 1, "buyMarginPct": b, "sellMarginPct": s }))
            .collect::<Vec<_>>()
    )
}

const USD: [(&str, &str); 4] = [
    ("0.4000", "-0.4000"),
    ("0.4500", "-0.4500"),
    ("0.5000", "-0.5000"),
    ("0.6000", "-0.6000"),
];

fn body(v1: uuid::Uuid, currency: &str, t: Value, min_spread: Value, anchor: Value) -> Value {
    json!({
        "policyVersionId": v1.to_string(),
        "currency": currency,
        "tiers": t,
        "minSpreadPct": min_spread,
        "anchorSource": anchor,
        "reason": "PT-5 test",
    })
}

async fn nothing_written(room: &Room, what: &str) {
    assert_eq!(room.versions(B01).await.len(), 1, "{what}: no new version");
    assert_eq!(
        room.count("rate_policy_currency_tiers").await,
        0,
        "{what}: no override row"
    );
    assert_eq!(
        room.count("rate_policy_currency_settings").await,
        0,
        "{what}: no settings row"
    );
}

#[tokio::test]
async fn the_owner_saves_a_currency_override_as_a_new_version_without_a_draft() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;

    let a = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, "USD", tiers(USD), json!("1.5000"), json!("MARKET_TT")),
        None,
    )
    .await;
    let out = a.ok("configure");
    assert_eq!(out["version"], json!(2), "{}", a.text);

    let v2 = room.current_version(B01).await;
    assert_eq!(out["id"], json!(v2.to_string()));
    let rows = room.override_rows(v2).await;
    assert_eq!(rows.len(), 4);
    for ((currency, no, buy, sell), (eb, es)) in rows.iter().zip(USD.iter()) {
        assert_eq!(currency, "USD");
        assert_eq!((*buy, *sell), (dec(eb), dec(es)), "tier {no}");
    }
    assert_eq!(
        room.settings_rows(v2).await,
        vec![(
            "USD".to_string(),
            Some(dec("1.5")),
            Some("MARKET_TT".to_string())
        )]
    );
    assert_eq!(
        room.count("rate_drafts").await,
        0,
        "§6: configure writes no draft"
    );
    room.finish().await;
}

#[tokio::test]
async fn the_new_version_keeps_the_rest_of_the_policy_and_the_other_currencies() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    room.override_margins(
        v1,
        "EUR",
        [
            ("0.1000", "0.1000"),
            ("0.2000", "0.2000"),
            ("0.3000", "0.3000"),
            ("0.4000", "0.4000"),
        ],
    )
    .await;
    room.settings(v1, "EUR", Some("2.0000"), None).await;

    command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, "USD", tiers(USD), Value::Null, Value::Null),
        None,
    )
    .await
    .ok("configure");
    let v2 = room.current_version(B01).await;

    let same: (bool,) = sqlx::query_as(
        "SELECT a.buy_adjust_pct = b.buy_adjust_pct AND a.sell_adjust_pct = b.sell_adjust_pct AND a.guards = b.guards \
                AND a.value_tiers_on = b.value_tiers_on AND a.anchor_source = b.anchor_source \
                AND b.source_version = a.version \
         FROM rate_policy_versions a, rate_policy_versions b WHERE a.id = $1 AND b.id = $2",
    )
    .bind(v1)
    .bind(v2)
    .fetch_one(&room.pool)
    .await
    .expect("compare versions");
    assert!(same.0, "v2 carries v1's frame and names v1 as its source");
    assert_eq!(
        room.count("rate_policy_value_tiers").await,
        8,
        "v2 has the branch's four value tiers too"
    );

    let eur: Vec<_> = room
        .override_rows(v2)
        .await
        .into_iter()
        .filter(|r| r.0 == "EUR")
        .collect();
    assert_eq!(eur.len(), 4, "EUR's override is carried");
    assert_eq!(
        room.settings_rows(v2).await,
        vec![("EUR".to_string(), Some(dec("2")), None)],
        "USD has no setting; EUR's is carried"
    );
    room.finish().await;
}

#[tokio::test]
async fn three_tiers_are_refused_and_nothing_is_written() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let mut t = tiers(USD);
    t.as_array_mut().unwrap().pop();
    let a = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, "USD", t, Value::Null, Value::Null),
        None,
    )
    .await;
    a.refused(400, "validationFailed", "§11: override with 3 rows");
    nothing_written(&room, "3 tiers").await;
    room.finish().await;
}

#[tokio::test]
async fn a_margin_outside_the_currencys_guard_is_refused() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    // T4 buy: −0.5 + 6.0 = 5.5, outside USD's [−5, 5].
    let t = tiers([
        ("0.0000", "0.0000"),
        ("0.1000", "-0.1000"),
        ("0.2000", "-0.2000"),
        ("6.0000", "-0.3000"),
    ]);
    let a = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, "USD", t, Value::Null, Value::Null),
        None,
    )
    .await;
    a.refused(
        400,
        "validationFailed",
        "§11: override margin outside the version's guard",
    );
    nothing_written(&room, "outside the guard").await;
    room.finish().await;
}

#[tokio::test]
async fn a_minimum_spread_outside_zero_to_five_or_an_unknown_anchor_is_refused() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let a = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, "USD", Value::Null, json!("5.5000"), Value::Null),
        None,
    )
    .await;
    a.refused(400, "validationFailed", "min spread 5.5");
    let a = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, "USD", Value::Null, Value::Null, json!("MARKET_XX")),
        None,
    )
    .await;
    a.refused(400, "validationFailed", "anchor MARKET_XX");
    nothing_written(&room, "bad settings").await;
    room.finish().await;
}

#[tokio::test]
async fn a_rate_manager_is_refused_as_outside_the_role() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let a = command(
        &room,
        &rate_manager(),
        ROUTE,
        B01,
        body(v1, "USD", tiers(USD), Value::Null, Value::Null),
        None,
    )
    .await;
    let err = PolicyError::RoleNotAllowedHere;
    assert_eq!(a.status.as_u16(), err.status(), "{}", a.text);
    assert_eq!(
        a.code(),
        json!(V3Error::from(err).code),
        "§11: RATE_MANAGER configures margins ⇒ roleNotAllowedHere"
    );
    let a = command(
        &room,
        &rate_manager(),
        ROUTE,
        B01,
        body(v1, "USD", Value::Null, Value::Null, json!("MARKET_TT")),
        None,
    )
    .await;
    assert_eq!(
        a.code(),
        json!(V3Error::from(PolicyError::RoleNotAllowedHere).code),
        "the anchor is frame too (§3.1)"
    );
    nothing_written(&room, "RATE_MANAGER").await;
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
        body(v1, "USD", tiers(USD), Value::Null, Value::Null),
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
async fn a_stale_policy_version_is_refused() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, "EUR", Value::Null, json!("1.0000"), Value::Null),
        None,
    )
    .await
    .ok("first");
    let a = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, "USD", tiers(USD), Value::Null, Value::Null),
        None,
    )
    .await;
    a.refused(409, "staleVersion", "built on v1 after v2 was written");
    assert_eq!(room.versions(B01).await.len(), 2, "no third version");
    room.finish().await;
}

#[tokio::test]
async fn all_three_null_removes_the_currencys_override() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    room.override_margins(v1, "USD", USD).await;
    room.settings(v1, "USD", Some("1.0000"), Some("MARKET_TT"))
        .await;

    command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, "USD", Value::Null, Value::Null, Value::Null),
        None,
    )
    .await
    .ok("remove");
    let v2 = room.current_version(B01).await;
    assert!(
        room.override_rows(v2).await.is_empty(),
        "USD back to the branch's margins"
    );
    assert!(
        room.settings_rows(v2).await.is_empty(),
        "USD back to the branch's anchor"
    );
    assert_eq!(
        room.override_rows(v1).await.len(),
        4,
        "v1's rows are immutable"
    );
    room.finish().await;
}
