//! Ticket #3977: `GET /api/v3/views/me` is authorised by its own policy row (`(EntryPath::Rpc, "me")` ->
//! `Permission::SignedIn`), not by `adminSnapshot`'s `CompatRead`, so a `MARKETING`-only user reads `me` (200 with
//! `MarketingCampaigns`, `MarketingMembers`, `MarketingPerformance`), `TELLER` and `owner` still answer 200, and a
//! `CUSTOMER` session (`403 roleNotAllowedHere`) or no session (`401 signIn`) stays refused as today.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use actix_web::http::Method;
use auth_http_common::*;
use identity_signin_common::*;
use serde_json::{Value, json};
use xc_auth::domain::access::{Permission, Role, RoleGrants};
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xchange::presentation::http::v3::error::V3Error;

async fn setup() -> (Desk, FakeIdentity, App, TestApp) {
    let desk = Desk::start().await;
    desk.branch("XC-B01").await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    let reader = desk_app_without_custody!(desk);
    (desk, identity, app, reader)
}

async fn sign_in(app: &App, identity: &FakeIdentity, token: &str) -> String {
    identity.login_answers("/auth/login", token, REFRESH_1);
    let reply = app
        .send(Req::post(
            "/api/auth/login",
            json!({ "identifier": "+66812345678", "password": "pw-from-the-browser-01" }),
        ))
        .await;
    assert_eq!(reply.status, 200, "sign-in: {}", reply.text);
    reply.session()
}

fn get_me() -> Call {
    Call {
        method: Some(Method::GET),
        path: "/api/v3/views/me".to_owned(),
        proto: Some("https".to_owned()),
        ..Call::default()
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap_or_else(|| panic!("an array of strings: {v}"))
        .iter()
        .map(|s| {
            s.as_str()
                .unwrap_or_else(|| panic!("a string: {s}"))
                .to_owned()
        })
        .collect()
}

#[test]
fn me_policy_row_needs_signed_in_and_marketing_lacks_compat_read() {
    assert_eq!(
        Policy::need(EntryPath::Rpc, &Action::new("me")),
        Some(Need::Permission(Permission::SignedIn))
    );
    assert!(
        !RoleGrants::seed(Role::Marketing).contains(&Permission::CompatRead),
        "MARKETING must not hold CompatRead"
    );
    assert!(
        !RoleGrants::holds("MARKETING", Permission::CompatRead),
        "MARKETING must not hold CompatRead in the active snapshot"
    );
}

#[actix_web::test]
async fn marketing_only_staff_reads_me_with_marketing_permissions() {
    let (desk, identity, app, reader) = setup().await;
    let user_id = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Mkt Staff",
        &[("MARKETING", Some("XC-B01"))],
    )
    .await;
    let token = sign_in(&app, &identity, VALID_A).await;

    let answer = send(
        &reader,
        Call {
            cookie: Some(format!("{SESSION_COOKIE}={token}")),
            ..get_me()
        },
    )
    .await;
    assert_eq!(
        answer.status, 200,
        "MARKETING-only staff reads me (200): {}",
        answer.text
    );
    let data = answer.json()["data"].clone();
    assert_eq!(data["id"], json!(user_id));
    assert_eq!(data["name"], json!("Mkt Staff"));
    assert_eq!(data["branch"], json!("XC-B01"));
    assert_eq!(
        data["roles"],
        json!([{ "role": "MARKETING", "branchId": "XC-B01" }])
    );
    let perms = strings(&data["permissions"]);
    for expected in [
        "MarketingCampaigns",
        "MarketingMembers",
        "MarketingPerformance",
    ] {
        assert!(
            perms.iter().any(|p| p == expected),
            "MARKETING me.permissions must contain {expected}, got {perms:?}"
        );
    }
    assert!(
        !perms.iter().any(|p| p == "CompatRead"),
        "MARKETING me.permissions must not contain CompatRead, got {perms:?}"
    );

    drop(reader);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn teller_and_owner_still_read_me_200_and_customer_or_none_stays_refused() {
    let (desk, identity, app, reader) = setup().await;

    // 1. None (no session) -> 401 signIn
    let none = send(&reader, get_me()).await;
    assert_eq!(none.status, PolicyError::SignIn.status(), "{}", none.text);
    assert_eq!(
        none.json()["error"]["code"],
        json!(V3Error::from(PolicyError::SignIn).code),
        "{}",
        none.text
    );

    // 2. Owner -> 200. OR-7.m w2 (#1027): the owner signs in through Identity (EXECUTIVE + ADMIN),
    // no longer with the v2 owner cookie, so `id` is the owner's `users.id`, not v2's "owner".
    let owner = desk.owner_session("Owner Somchai").await;
    let owner_id: String = sqlx::query_scalar("SELECT id::text FROM users WHERE name = $1")
        .bind("Owner Somchai")
        .fetch_one(&desk.pool)
        .await
        .expect("the owner's users row");
    let owner_ans = send(&reader, get_me().session(&owner)).await;
    assert_eq!(owner_ans.status, 200, "owner reads me: {}", owner_ans.text);
    assert_eq!(owner_ans.json()["data"]["id"], json!(owner_id));

    // 3. TELLER -> 200
    let teller_id = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Teller Nok",
        &[("TELLER", Some("XC-B01"))],
    )
    .await;
    let teller_token = sign_in(&app, &identity, VALID_A).await;
    let teller_ans = send(
        &reader,
        Call {
            cookie: Some(format!("{SESSION_COOKIE}={teller_token}")),
            ..get_me()
        },
    )
    .await;
    assert_eq!(
        teller_ans.status, 200,
        "TELLER reads me: {}",
        teller_ans.text
    );
    assert_eq!(teller_ans.json()["data"]["id"], json!(teller_id));

    // 4. CUSTOMER session -> 403 roleNotAllowedHere (unchanged from today)
    bound_user(
        &desk.pool,
        SUB_B,
        "ACTIVE",
        "Customer Som",
        &[("CUSTOMER", None)],
    )
    .await;
    let customer_token = sign_in(&app, &identity, VALID_B).await;
    let customer_ans = send(
        &reader,
        Call {
            cookie: Some(format!("{SESSION_COOKIE}={customer_token}")),
            ..get_me()
        },
    )
    .await;
    assert_eq!(
        customer_ans.status,
        PolicyError::RoleNotAllowedHere.status(),
        "customer session stays refused 403: {}",
        customer_ans.text
    );
    assert_eq!(
        customer_ans.json()["error"]["code"],
        json!(V3Error::from(PolicyError::RoleNotAllowedHere).code),
        "customer session answers roleNotAllowedHere: {}",
        customer_ans.text
    );

    drop(reader);
    desk.drop_schema().await;
}
