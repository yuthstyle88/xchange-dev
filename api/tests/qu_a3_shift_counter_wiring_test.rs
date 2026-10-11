//! #1128 (QU-A3) — the shift actions themselves staff and release the queue counter: route-level
//! tests through `/api/rpc` `adminOpenShift` / `adminCloseShift`, so an implementation that never
//! installs the hook at the composition root, or never passes the session user to it, fails here.
//! Written before the implementation by claude/dell-claude-6 on the lane controller
//! claude/ad05bf8d's rulings 1–3 (#1128). Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! The rulings these pin:
//! 1. A Blueprint-session open gives the shift's teller an ACTIVE counter, and the close releases
//!    it. A legacy password caller (no session user) gets no counter row.
//! 2. The session user reaches `shift_action` as an extra argument of `CompatRpc::dispatch`, from
//!    its only caller (`presentation/http/rpc.rs`). `RpcCaller` gains no field.
//! 3. The counter is the shift's teller (queue-tickets.md Q7, `shifts.teller_id`), not whoever
//!    clicked. On this route an open always names the caller as the teller, so "an admin acts
//!    for a teller" is the close: a manager closing a teller's shift releases the teller's
//!    counter.
//!
//! The service- and hook-level tests are `xc-shift/tests/shift_lifecycle_hook_test.rs` and
//! `xc-queue/tests/queue_counter_hook_test.rs`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::{Desk, OWNER_PASSWORD};
use identity_signin_common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// What the shift is opened with, and so what a close with no difference counts.
fn till() -> Value {
    json!({ "THB": 100000, "USD": 1000 })
}

fn request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// A fresh schema, a fake Identity, and the app wired to it.
async fn setup() -> (Desk, FakeIdentity, App) {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    (desk, identity, app)
}

/// Signs a bound user in through Identity (the BFF) and gives back the session token.
async fn blueprint_session(
    app: &App,
    identity: &FakeIdentity,
    access_token: &str,
    refresh_token: &str,
    identifier: &str,
) -> String {
    identity.login_answers("/auth/login", access_token, refresh_token);
    app.send(Req::post(
        "/api/auth/login",
        json!({ "identifier": identifier, "password": "pw-from-the-browser-01" }),
    ))
    .await
    .session()
}

/// Signs the owner in with v2's password (`/api/auth`): a legacy caller, no session user.
async fn legacy_owner_cookie(app: &App) -> String {
    let reply = app
        .send(Req::post(
            "/api/auth",
            json!({ "action": "login", "payload": { "username": "boss", "password": OWNER_PASSWORD } }),
        ))
        .await;
    assert_eq!(reply.status, 200, "the owner signs in: {}", reply.text);
    match reply.cookies.get(OWNER_COOKIE) {
        Some(c) if !c.value.is_empty() => format!("{OWNER_COOKIE}={}", c.value),
        other => panic!("the reply sets {OWNER_COOKIE}: {other:?} / {}", reply.text),
    }
}

fn session_cookie(token: &str) -> String {
    format!("{SESSION_COOKIE}={token}")
}

fn rpc(action: &str, payload: Value, cookie: &str) -> Req {
    let mut req = Req::post("/api/rpc", json!({ "action": action, "payload": payload }));
    req.cookie = Some(cookie.to_owned());
    req
}

async fn open_shift(app: &App, cookie: &str) {
    let reply = app
        .send(rpc(
            "adminOpenShift",
            json!({ "branchId": "MAIN", "requestId": request_id(), "opening": till() }),
            cookie,
        ))
        .await;
    assert_eq!(reply.status, 200, "adminOpenShift: {}", reply.text);
}

async fn close_shift(app: &App, cookie: &str, shift_id: &str) {
    let reply = app
        .send(rpc(
            "adminCloseShift",
            json!({ "branchId": "MAIN", "requestId": request_id(), "shiftId": shift_id,
                    "counted": till(), "expected": till(), "note": "" }),
            cookie,
        ))
        .await;
    assert_eq!(reply.status, 200, "adminCloseShift: {}", reply.text);
}

/// The OPEN shift at MAIN: (id, its teller's `users.id`).
async fn open_shift_at_main(pool: &PgPool) -> (String, String) {
    sqlx::query_as(
        "SELECT id, teller_id::text FROM shifts WHERE branch_id = 'MAIN' AND status = 'OPEN'",
    )
    .fetch_one(pool)
    .await
    .expect("one OPEN shift at MAIN")
}

/// `teller status` of every queue counter at MAIN, by teller.
async fn counters(pool: &PgPool) -> Vec<String> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT teller_id::text || ' ' || status FROM queue_counters WHERE branch_id = 'MAIN' \
         ORDER BY teller_id",
    )
    .fetch_all(pool)
    .await
    .expect("read the queue counters");
    rows.into_iter().map(|(r,)| r).collect()
}

/// Ruling 1: a Blueprint-session open staffs the shift teller's counter; the close releases it.
#[actix_web::test]
async fn a_blueprint_session_open_staffs_the_shift_tellers_counter_and_the_close_releases_it() {
    let (desk, identity, app) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Manager",
        &[("BRANCH_MANAGER", Some("MAIN"))],
    )
    .await;
    let cookie = session_cookie(&blueprint_session(&app, &identity, VALID_A, REFRESH_1, "m").await);

    open_shift(&app, &cookie).await;
    let (shift_id, teller) = open_shift_at_main(&desk.pool).await;
    assert_eq!(teller, user, "the shift's teller is the session user");
    assert_eq!(
        counters(&desk.pool).await,
        vec![format!("{user} ACTIVE")],
        "adminOpenShift through a Blueprint session must give the shift's teller an ACTIVE counter"
    );

    close_shift(&app, &cookie, &shift_id).await;
    assert_eq!(
        counters(&desk.pool).await,
        Vec::<String>::new(),
        "adminCloseShift through a Blueprint session must release that counter"
    );
    desk.drop_schema().await;
}

/// Ruling 1: a legacy password caller has no session user — no counter row, open or close.
#[actix_web::test]
async fn a_legacy_password_open_and_close_leave_every_counter_untouched() {
    let (desk, _identity, app) = setup().await;
    let cookie = legacy_owner_cookie(&app).await;

    open_shift(&app, &cookie).await;
    let (shift_id, _teller) = open_shift_at_main(&desk.pool).await;
    assert_eq!(
        counters(&desk.pool).await,
        Vec::<String>::new(),
        "a legacy open staffs no counter"
    );

    close_shift(&app, &cookie, &shift_id).await;
    assert_eq!(counters(&desk.pool).await, Vec::<String>::new());
    desk.drop_schema().await;
}

/// Ruling 3: a manager closing another user's shift releases the shift teller's counter, not
/// the manager's; the manager never gets one.
#[actix_web::test]
async fn an_admin_closing_a_tellers_shift_releases_the_tellers_counter() {
    let (desk, identity, app) = setup().await;
    let teller = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Opener",
        &[("BRANCH_MANAGER", Some("MAIN"))],
    )
    .await;
    let admin = bound_user(
        &desk.pool,
        SUB_B,
        "ACTIVE",
        "Closer",
        &[("BRANCH_MANAGER", Some("MAIN"))],
    )
    .await;
    let teller_cookie =
        session_cookie(&blueprint_session(&app, &identity, VALID_A, REFRESH_1, "t").await);
    let admin_cookie =
        session_cookie(&blueprint_session(&app, &identity, VALID_B, REFRESH_2, "a").await);

    open_shift(&app, &teller_cookie).await;
    let (shift_id, shift_teller) = open_shift_at_main(&desk.pool).await;
    assert_eq!(shift_teller, teller);
    assert_eq!(
        counters(&desk.pool).await,
        vec![format!("{teller} ACTIVE")],
        "the opener's counter is staffed"
    );

    close_shift(&app, &admin_cookie, &shift_id).await;
    let after = counters(&desk.pool).await;
    assert!(
        !after.iter().any(|c| c.starts_with(&teller)),
        "closing the shift releases its teller's counter even when another user closes it: {after:?}"
    );
    assert!(
        !after.iter().any(|c| c.starts_with(&admin)),
        "the user who closed it gets no counter: {after:?}"
    );
    desk.drop_schema().await;
}
