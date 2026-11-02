//! R5 "Sign-in stores its pick" (design `docs/design/blueprint/org-roles-branches.md` §14.2, amended 2026-11-02;
//! lane controller claude/608dff's ruling B on #564 issuecomment-6572815765): a sign-in with no last choice stores the
//! branch it picked as `users.last_branch_id`, and never overwrites a choice already set. So creating a branch never
//! moves an organisation-wide owner, not even across a sign-out and a new sign-in.
//!
//! Root cause pinned here: a new branch's code is its id (`xc-rate/src/infrastructure/branch/branch_repository.rs:181`),
//! `B…` sorts before `MAIN`, and with `last_branch_id` NULL every sign-in fell to R5 step 3 (first by code)
//! (#564 issuecomment-6572524037). The first test is claude/01TxaRJG's probe, with the sign-out they added.
//!
//! Written before the implementation by the ticket author claude/b091f5e0 (the first test by claude/01TxaRJG). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use actix_web::http::Method;
use auth_http_common::{Call, Desk, TestApp, send};
use identity_signin_common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

const NEW_BRANCH_REQUEST: &str = "00000000-0000-4000-8000-00000000b501";

async fn me(reader: &TestApp, token: &str) -> Value {
    let answer = send(
        reader,
        Call {
            method: Some(Method::GET),
            path: "/api/v3/views/me".to_owned(),
            proto: Some("https".to_owned()),
            cookie: Some(format!("{SESSION_COOKIE}={token}")),
            ..Call::default()
        },
    )
    .await;
    assert_eq!(answer.status, 200, "me: {}", answer.text);
    answer.json()["data"].clone()
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

async fn sign_out(app: &App, token: &str) {
    let out = app
        .send(Req::post("/api/auth/logout", json!({})).with_session(token))
        .await;
    assert_eq!(out.status, 200, "logout: {}", out.text);
}

async fn create_branch(app: &App, token: &str) -> String {
    let body = json!({ "action": "adminCreateBranch",
        "payload": { "name": "TEST2", "requestId": NEW_BRANCH_REQUEST } });
    let reply = app
        .send(Req::post("/api/rpc", body).with_session(token))
        .await;
    assert_eq!(reply.status, 200, "create: {}", reply.text);
    format!("B{NEW_BRANCH_REQUEST}")
}

async fn last_branch(pool: &PgPool, user: &str) -> Option<String> {
    sqlx::query_scalar("SELECT last_branch_id FROM users WHERE id = $1::uuid")
        .bind(user)
        .fetch_one(pool)
        .await
        .expect("the user's last branch")
}

/// claude/01TxaRJG's probe, with the sign-out: the owner starts on MAIN, creates TEST2 (code `B…`, before `MAIN`),
/// signs out (the session row is deleted) and signs in again, and is still on MAIN.
#[actix_web::test]
async fn creating_a_branch_then_signing_out_and_in_leaves_the_owner_on_main() {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    identity.answer("/auth/logout", 204, "");
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    let reader = desk_app_without_custody!(desk);
    let owner = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Owner",
        &[("EXECUTIVE", None), ("ADMIN", None)],
    )
    .await;

    let first = sign_in(&app, &identity, VALID_A).await;
    assert_eq!(
        me(&reader, &first).await["branch"],
        json!("MAIN"),
        "before the create"
    );
    assert_eq!(
        last_branch(&desk.pool, &owner).await.as_deref(),
        Some("MAIN"),
        "§14.2: the first sign-in stored its pick"
    );

    let created = create_branch(&app, &first).await;
    assert!(
        created.as_str() < "MAIN",
        "the new branch's code sorts before MAIN: {created}"
    );
    assert_eq!(
        me(&reader, &first).await["branch"],
        json!("MAIN"),
        "same session after the create"
    );

    sign_out(&app, &first).await;
    let second = sign_in(&app, &identity, VALID_A).await;
    assert_eq!(
        me(&reader, &second).await["branch"],
        json!("MAIN"),
        "a new sign-in after the create and a sign-out"
    );
    drop(reader);
    desk.drop_schema().await;
}

/// A real choice (the switch command, R2) is never overwritten by a later sign-in: the owner switches to TEST2, signs
/// out and in, and starts on TEST2.
#[actix_web::test]
async fn a_switch_is_never_overwritten_by_a_later_sign_in() {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    identity.answer("/auth/logout", 204, "");
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    let reader = desk_app_without_custody!(desk);
    let owner = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Owner",
        &[("EXECUTIVE", None), ("ADMIN", None)],
    )
    .await;

    let first = sign_in(&app, &identity, VALID_A).await;
    let created = create_branch(&app, &first).await;
    let switched = app
        .send(
            Req::post("/api/auth/session/branch", json!({ "branch": created }))
                .with_session(&first),
        )
        .await;
    assert_eq!(switched.status, 200, "switch: {}", switched.text);

    sign_out(&app, &first).await;
    let second = sign_in(&app, &identity, VALID_A).await;
    assert_eq!(me(&reader, &second).await["branch"], json!(created));
    assert_eq!(
        last_branch(&desk.pool, &owner).await,
        Some(created),
        "the sign-in left the chosen branch as it was"
    );
    drop(reader);
    desk.drop_schema().await;
}

/// A last choice outside the user's branches (here: a branch the user was never granted) is not overwritten by the
/// sign-in; R5 step 3 picks as today, and the stored value stays (ruling B: store only into an empty slot).
#[actix_web::test]
async fn a_set_choice_outside_the_users_branches_is_left_as_it_is() {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    let reader = desk_app_without_custody!(desk);
    let teller = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Nok",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    let owner_identity = FakeIdentity::start();
    let (owner_app, owner_token) = app_for_owner(&desk.pool, &owner_identity).await;
    let created = create_branch(&owner_app, &owner_token).await;
    sqlx::query("UPDATE users SET last_branch_id = $2 WHERE id = $1::uuid")
        .bind(&teller)
        .bind(&created)
        .execute(&desk.pool)
        .await
        .expect("a stale choice the teller has no grant for");

    let token = sign_in(&app, &identity, VALID_A).await;
    assert_eq!(
        me(&reader, &token).await["branch"],
        json!("MAIN"),
        "R5 step 1: the only branch"
    );
    assert_eq!(
        last_branch(&desk.pool, &teller).await,
        Some(created),
        "§14.2: a set choice is never overwritten by a sign-in"
    );
    drop(reader);
    desk.drop_schema().await;
}

/// An owner signed in through their own fake Identity (subject B), for creating a branch: `(app, session token)`.
async fn app_for_owner(pool: &PgPool, identity: &FakeIdentity) -> (App, String) {
    bound_user(
        pool,
        SUB_B,
        "ACTIVE",
        "Owner",
        &[("EXECUTIVE", None), ("ADMIN", None)],
    )
    .await;
    let app = app(state_with(pool, &identity_env(&identity.url()))).await;
    let token = sign_in(&app, identity, VALID_B).await;
    (app, token)
}
