//! XC-PRICING PX-11 (pricing-tiers.md §5.2, §6): `pricingSuggestionSettingsSave` (`POST
//! /api/pricing/suggestion-settings`), V.9's `configure` (`src/pricing.mjs:200-211`): a branch's `better` % for one
//! currency or `"*"`, an EXECUTIVE-only frame change, no policy version and no draft. Needs PX-10 on the branch.
//! Written before the implementation by the ticket author (claude/mbp-xc-pricing, PX-0). Do not edit, delete or
//! weaken; add tests in a file of your own.

mod pricing_common;

use pricing_common::*;
use serde_json::{Value, json};
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xchange::presentation::http::v3::error::V3Error;

const ROUTE: &str = "/api/pricing/suggestion-settings";

fn body(currency: &str, better: Option<[&str; 4]>) -> Value {
    json!({ "currency": currency, "betterPct": better })
}

async fn stored(room: &Room, scope: &str) -> Option<Vec<rust_decimal::Decimal>> {
    sqlx::query_scalar::<_, Vec<rust_decimal::Decimal>>(
        "SELECT better_pct FROM pricing_suggestion_settings WHERE branch_id = $1 AND scope = $2",
    )
    .bind(B01)
    .bind(scope)
    .fetch_optional(&room.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn the_owner_saves_a_currency_and_the_branch_default() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let a = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body("USD", Some(["0", "0.1", "0.2", "0.3"])),
        None,
    )
    .await;
    assert_eq!(a.ok("save USD")["saved"], json!(true));
    assert_eq!(
        stored(&room, "USD").await,
        Some(vec![dec("0"), dec("0.1"), dec("0.2"), dec("0.3")])
    );
    command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body("*", Some(["0.05", "0.1", "0.1", "0.2"])),
        None,
    )
    .await
    .ok("save *");
    assert!(stored(&room, "*").await.is_some());
    assert_eq!(
        room.current_version(B01).await,
        v1,
        "no policy version is written"
    );
    assert_eq!(room.count("rate_drafts").await, 0, "no draft");
    room.finish().await;
}

#[tokio::test]
async fn null_removes_the_row() {
    let room = Room::start().await;
    room.worked_example().await;
    command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body("USD", Some(["0", "0.1", "0.2", "0.3"])),
        None,
    )
    .await
    .ok("save");
    command(&room, &owner(), ROUTE, B01, body("USD", None), None)
        .await
        .ok("remove");
    assert_eq!(stored(&room, "USD").await, None);
    room.finish().await;
}

#[tokio::test]
async fn v9s_checks_refuse_and_write_nothing() {
    let room = Room::start().await;
    room.worked_example().await;
    for (what, b) in [
        ("above 2 %", body("USD", Some(["0", "0.1", "0.2", "2.5"]))),
        (
            "a higher tier worse",
            body("USD", Some(["0.2", "0.1", "0.3", "0.4"])),
        ),
        (
            "three values",
            json!({ "currency": "USD", "betterPct": ["0", "0.1", "0.2"] }),
        ),
        ("not a number", body("USD", Some(["0", "x", "0.2", "0.3"]))),
        (
            "unknown currency",
            body("ZZZ", Some(["0", "0.1", "0.2", "0.3"])),
        ),
    ] {
        command(&room, &owner(), ROUTE, B01, b, None)
            .await
            .refused(400, "validationFailed", what);
    }
    assert_eq!(room.count("pricing_suggestion_settings").await, 0);
    room.finish().await;
}

#[tokio::test]
async fn a_rate_manager_may_not_change_it() {
    let room = Room::start().await;
    room.worked_example().await;
    let a = command(
        &room,
        &rate_manager(),
        ROUTE,
        B01,
        body("USD", Some(["0", "0.1", "0.2", "0.3"])),
        None,
    )
    .await;
    let err = PolicyError::RoleNotAllowedHere;
    assert_eq!(a.status.as_u16(), err.status(), "{}", a.text);
    assert_eq!(
        a.code(),
        json!(V3Error::from(err).code),
        "a frame change: EXECUTIVE only (V.9 pricing.mjs:201 owner only)"
    );
    assert_eq!(room.count("pricing_suggestion_settings").await, 0);
    room.finish().await;
}

#[tokio::test]
async fn a_caller_without_rate_policy_save_is_refused() {
    let room = Room::start().await;
    room.worked_example().await;
    let a = command(
        &room,
        &teller(),
        ROUTE,
        B01,
        body("USD", Some(["0", "0.1", "0.2", "0.3"])),
        None,
    )
    .await;
    let err = PolicyError::RoleNotAllowedHere;
    assert_eq!(a.status.as_u16(), err.status(), "{}", a.text);
    assert_eq!(a.code(), json!(V3Error::from(err).code), "{}", a.text);
    assert_eq!(room.count("pricing_suggestion_settings").await, 0);
    room.finish().await;
}
