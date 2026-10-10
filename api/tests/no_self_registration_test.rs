//! Ticket #1097 — xchange creates no account by self-registration (owner, 2026-10-10:
//! *"ลบ register ที่ server ออกด้วย"*, *"เพราะ ระบบภายใน"*). Staff accounts are made by the shop owner
//! on the employee screen (`POST /api/employees`), so the server has no `/api/auth/register`.
//! Written before the implementation by the ticket author (claude/89bd61d9). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Uses the Identity sign-in harness (`identity_signin_common`): a real database schema and a
//! fake Identity that records every call. Needs `TEST_DATABASE_URL`; nothing skips.

mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use identity_signin_common::*;
use serde_json::json;

#[actix_web::test]
async fn there_is_no_register_route_and_identity_is_never_asked_to_register() {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    // Identity would accept it: only xchange's own route decides that nobody registers here.
    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": "00000000-0000-4000-8000-0000000c0de1" }).to_string(),
    );
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;

    let reply = app
        .send(Req::post(
            "/api/auth/register",
            json!({ "phone": "+66812345678", "password": "pw-from-the-browser-01" }),
        ))
        .await;

    assert_eq!(
        reply.status, 404,
        "POST /api/auth/register is gone: {}",
        reply.text
    );
    assert!(
        identity.received("/auth/register").is_empty(),
        "nothing is forwarded to Identity's /auth/register"
    );
    assert!(
        identity.calls().is_empty(),
        "nothing is sent to Identity at all: {:?}",
        identity
            .calls()
            .iter()
            .map(|c| c.path.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        reply.sets_no_session(),
        "no session cookie: {:?}",
        reply.cookies
    );
    assert_eq!(all_sessions(&desk.pool).await, 0, "no user_sessions row");
    desk.drop_schema().await;
}
