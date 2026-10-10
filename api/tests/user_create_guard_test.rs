//! #1157 review (claude/709ec749, PR 1194): two guards on `POST /api/users` (`userCreate`) that the
//! provided tests in `user_create_test.rs` do not cover. Written by the implementer (claude/08f7f46f).
//!
//! 1. A write from another site is refused with `originRejected` before Identity is called, as on
//!    every other write route.
//! 2. The roles a creator gives must be inside the creator's own ADMIN scope: an organisation-wide
//!    role needs ADMIN organisation-wide; a branch role needs ADMIN organisation-wide or on that same
//!    branch. Anything else is `outOfScope`, nothing is written and Identity is not called.

mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use identity_signin_common::*;
use serde_json::{Value, json};

async fn setup() -> (Desk, FakeIdentity, App) {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    (desk, identity, app)
}

/// Signs in a bound user holding `roles` and returns the session token.
async fn session_with(
    desk: &Desk,
    identity: &FakeIdentity,
    app: &App,
    roles: &[(&str, Option<&str>)],
) -> String {
    let _ = bound_user(&desk.pool, SUB_A, "ACTIVE", "Creator", roles).await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    app.send(Req::post(
        "/api/auth/login",
        json!({ "identifier": "+66811111111", "password": "pw-creator-01" }),
    ))
    .await
    .session()
}

async fn branch(desk: &Desk, id: &str) {
    sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1) ON CONFLICT (id) DO NOTHING")
        .bind(id)
        .execute(&desk.pool)
        .await
        .expect("insert a branch");
}

fn create(name: &str, roles: Value) -> Req {
    Req::post(
        "/api/users",
        json!({
            "name": name,
            "phone": "+66855555555",
            "password": "temp-password-55",
            "roles": roles
        }),
    )
}

async fn users_named(desk: &Desk, name: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM users WHERE name = $1")
        .bind(name)
        .fetch_one(&desk.pool)
        .await
        .expect("count users")
}

#[actix_web::test]
async fn a_cross_site_create_is_refused_before_identity() {
    let (desk, identity, app) = setup().await;
    let token = session_with(
        &desk,
        &identity,
        &app,
        &[("ADMIN", None), ("EXECUTIVE", None)],
    )
    .await;
    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": SUB_B }).to_string(),
    );

    let reply = app
        .send(
            create("CrossSite", json!([{ "role": "TELLER", "branch": "MAIN" }]))
                .with_session(&token)
                .cross_site(),
        )
        .await;

    assert_eq!(reply.status, 400, "{}", reply.text);
    assert_eq!(reply.refusal().0, "originRejected");
    assert!(
        identity.received("/auth/register").is_empty(),
        "Identity was not called"
    );
    assert_eq!(users_named(&desk, "CrossSite").await, 0, "no user row");
    desk.drop_schema().await;
}

#[actix_web::test]
async fn a_branch_admin_cannot_create_an_organisation_wide_role() {
    let (desk, identity, app) = setup().await;
    let token = session_with(&desk, &identity, &app, &[("ADMIN", Some("MAIN"))]).await;
    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": SUB_B }).to_string(),
    );

    let reply = app
        .send(
            create(
                "Escalated",
                json!([{ "role": "EXECUTIVE", "branch": null }, { "role": "ADMIN", "branch": null }]),
            )
            .with_session(&token),
        )
        .await;

    assert_eq!(reply.status, 403, "{}", reply.text);
    assert_eq!(reply.refusal().0, "outOfScope");
    assert!(
        identity.received("/auth/register").is_empty(),
        "Identity was not called"
    );
    assert_eq!(users_named(&desk, "Escalated").await, 0, "no user row");
    desk.drop_schema().await;
}

#[actix_web::test]
async fn a_branch_admin_cannot_create_a_role_at_another_branch() {
    let (desk, identity, app) = setup().await;
    branch(&desk, "B2").await;
    let token = session_with(&desk, &identity, &app, &[("ADMIN", Some("MAIN"))]).await;
    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": SUB_B }).to_string(),
    );

    let reply = app
        .send(
            create("OtherBranch", json!([{ "role": "TELLER", "branch": "B2" }]))
                .with_session(&token),
        )
        .await;

    assert_eq!(reply.status, 403, "{}", reply.text);
    assert_eq!(reply.refusal().0, "outOfScope");
    assert!(
        identity.received("/auth/register").is_empty(),
        "Identity was not called"
    );
    assert_eq!(users_named(&desk, "OtherBranch").await, 0, "no user row");
    desk.drop_schema().await;
}

#[actix_web::test]
async fn a_branch_admin_creates_a_role_at_their_own_branch() {
    let (desk, identity, app) = setup().await;
    let token = session_with(&desk, &identity, &app, &[("ADMIN", Some("MAIN"))]).await;
    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": SUB_B }).to_string(),
    );

    let reply = app
        .send(
            create("OwnBranch", json!([{ "role": "TELLER", "branch": "MAIN" }]))
                .with_session(&token),
        )
        .await;

    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(reply.json()["status"], "ACTIVE");
    assert_eq!(identity.received("/auth/register").len(), 1);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn an_organisation_wide_admin_creates_an_organisation_wide_role() {
    let (desk, identity, app) = setup().await;
    let token = session_with(
        &desk,
        &identity,
        &app,
        &[("ADMIN", None), ("EXECUTIVE", None)],
    )
    .await;
    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": SUB_B }).to_string(),
    );

    let reply = app
        .send(create("OrgWide", json!([{ "role": "ADMIN", "branch": null }])).with_session(&token))
        .await;

    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(identity.received("/auth/register").len(), 1);
    desk.drop_schema().await;
}
