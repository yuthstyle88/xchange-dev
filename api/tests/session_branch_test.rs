//! SB-1 (design `docs/design/blueprint/org-roles-branches.md` §14, PR 831): the session branch on the server: sign-in
//! stores the R5 branch (§14.2), `POST /api/auth/session/branch` switches it for this session only and records the
//! user's last choice, a branch outside the user's grants is refused and nothing is written (R3, D-4), the next read
//! answers for the session branch, a revoked grant moves the session on the next read, and login takes no branch (R4).
//!
//! Written before the implementation by the ticket author (claude/83f682). Do not edit, delete or weaken these tests;
//! add your own in another file.
//!
//! Shapes these tests assume (§14.3, §14.4 [required]):
//! - `POST /api/auth/session/branch`, body `{"branch": "<id>"}`, same-origin + `X-Xchange-Request: 1`;
//!   `200` with the `Me` object itself as the body; `400 invalidRequest`; `401 signIn`; `403 outOfScope`.
//! - `GET /api/v3/views/me` → `data.branch` (the effective branch id), `data.branchName`, and `data.branches`
//!   = `[{id, code, name}]` ordered by code, then id.
//! - `user_sessions.branch_id` and `users.last_branch_id` (§14.5).
//!
//! `TEST_DATABASE_URL` must name a database whose name ends in `_test`; each test gets its own schema.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use actix_web::http::Method;
use auth_http_common::{Call, Desk, TestApp, send};
use identity_signin_common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

const OUT_OF_SCOPE: &str = "outOfScope";

/// The organisation's branches beside `MAIN` (code `MAIN`, backfilled by OR-1): codes sort A05 < MAIN < Z09, ids do not.
async fn branches(pool: &PgPool) {
    let org = organization(pool).await;
    for (id, code, name) in [("B2", "Z09", "สาขาสอง"), ("B3", "A05", "สาขาสาม")]
    {
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id, code) VALUES ($1, $2, $3::uuid, $4)",
        )
        .bind(id)
        .bind(name)
        .bind(&org)
        .bind(code)
        .execute(pool)
        .await
        .expect("insert a branch");
    }
}

async fn setup() -> (Desk, FakeIdentity, App, TestApp) {
    let desk = Desk::start().await;
    branches(&desk.pool).await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    let reader = desk_app_without_custody!(desk);
    (desk, identity, app, reader)
}

fn login_body() -> Value {
    json!({ "identifier": "+66812345678", "password": "pw-from-the-browser-01" })
}

/// Signs in as the user bound to `sub` (whose Identity token is `token`); the session cookie value.
async fn sign_in(app: &App, identity: &FakeIdentity, token: &str) -> String {
    identity.login_answers("/auth/login", token, REFRESH_1);
    let reply = app.send(Req::post("/api/auth/login", login_body())).await;
    assert_eq!(reply.status, 200, "sign-in: {}", reply.text);
    reply.session()
}

fn switch(token: &str, branch: &str) -> Req {
    Req::post("/api/auth/session/branch", json!({ "branch": branch })).with_session(token)
}

async fn session_branch(pool: &PgPool, token: &str) -> Option<String> {
    sqlx::query_scalar("SELECT branch_id FROM user_sessions WHERE hash = $1")
        .bind(sha256_hex(token))
        .fetch_one(pool)
        .await
        .expect("the session row")
}

async fn last_branch(pool: &PgPool, user: &str) -> Option<String> {
    sqlx::query_scalar("SELECT last_branch_id FROM users WHERE id = $1::uuid")
        .bind(user)
        .fetch_one(pool)
        .await
        .expect("the user row")
}

/// `GET /api/v3/views/me` with the session cookie; the `data` of a 200.
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

fn all_three() -> Value {
    json!([
        { "id": "B3", "code": "A05", "name": "สาขาสาม" },
        { "id": "MAIN", "code": "MAIN", "name": "XC-B" },
        { "id": "B2", "code": "Z09", "name": "สาขาสอง" },
    ])
}

// ── sign-in stores the R5 branch ──────────────────────────────────────────

/// R5 step 3: several granted branches and no last choice → the first by code (B3 = A05), not `MAIN`.
#[actix_web::test]
async fn a_sign_in_with_several_branches_starts_on_the_first_by_code() {
    let (desk, identity, app, reader) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Mai",
        &[("RATE_MANAGER", Some("B2")), ("RATE_MANAGER", Some("B3"))],
    )
    .await;

    let token = sign_in(&app, &identity, VALID_A).await;

    assert_eq!(
        session_branch(&desk.pool, &token).await.as_deref(),
        Some("B3")
    );
    assert_eq!(
        last_branch(&desk.pool, &user).await,
        None,
        "sign-in is not a choice"
    );
    let data = me(&reader, &token).await;
    assert_eq!(data["branch"], json!("B3"));
    assert_eq!(data["branchName"], json!("สาขาสาม"));
    assert_eq!(
        data["branches"],
        json!([
            { "id": "B3", "code": "A05", "name": "สาขาสาม" },
            { "id": "B2", "code": "Z09", "name": "สาขาสอง" },
        ])
    );
    drop(reader);
    desk.drop_schema().await;
}

/// R5 step 3 for an organisation-wide user: every branch is switchable, and the first by code is B3, not `MAIN`.
#[actix_web::test]
async fn an_organisation_wide_user_starts_on_the_first_branch_by_code_not_main() {
    let (desk, identity, app, reader) = setup().await;
    bound_user(&desk.pool, SUB_A, "ACTIVE", "Admin", &[("ADMIN", None)]).await;

    let token = sign_in(&app, &identity, VALID_A).await;

    assert_eq!(
        session_branch(&desk.pool, &token).await.as_deref(),
        Some("B3")
    );
    let data = me(&reader, &token).await;
    assert_eq!(data["branch"], json!("B3"));
    assert_eq!(data["branches"], all_three());
    drop(reader);
    desk.drop_schema().await;
}

/// R5 step 1: one branch is the branch, and `branches` lists just it.
#[actix_web::test]
async fn a_one_branch_user_starts_on_it_and_has_a_one_element_list() {
    let (desk, identity, app, reader) = setup().await;
    bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Nok",
        &[("TELLER", Some("B2"))],
    )
    .await;

    let token = sign_in(&app, &identity, VALID_A).await;

    assert_eq!(
        session_branch(&desk.pool, &token).await.as_deref(),
        Some("B2")
    );
    let data = me(&reader, &token).await;
    assert_eq!(data["branch"], json!("B2"));
    assert_eq!(data["branchName"], json!("สาขาสอง"));
    assert_eq!(
        data["branches"],
        json!([{ "id": "B2", "code": "Z09", "name": "สาขาสอง" }])
    );
    drop(reader);
    desk.drop_schema().await;
}

/// R4: login takes no branch. A `branch` in the login body changes nothing; the session gets the R5 branch.
#[actix_web::test]
async fn login_takes_no_branch() {
    let (desk, identity, app, reader) = setup().await;
    let user = bound_user(&desk.pool, SUB_A, "ACTIVE", "Admin", &[("ADMIN", None)]).await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let mut body = login_body();
    body["branch"] = json!("B2");

    let reply = app.send(Req::post("/api/auth/login", body)).await;

    assert_eq!(reply.status, 200, "{}", reply.text);
    let token = reply.session();
    assert_eq!(
        session_branch(&desk.pool, &token).await.as_deref(),
        Some("B3")
    );
    assert_eq!(last_branch(&desk.pool, &user).await, None);
    drop(reader);
    desk.drop_schema().await;
}

// ── the switch command ────────────────────────────────────────────────────

/// R2: the command moves this session, records the last choice, answers the new `Me`, and the next read with no
/// `?branch=` answers for the chosen branch.
#[actix_web::test]
async fn switching_to_a_granted_branch_moves_the_session_and_records_the_choice() {
    let (desk, identity, app, reader) = setup().await;
    let user = bound_user(&desk.pool, SUB_A, "ACTIVE", "Admin", &[("ADMIN", None)]).await;
    let token = sign_in(&app, &identity, VALID_A).await;

    let reply = app.send(switch(&token, "B2")).await;

    assert_eq!(reply.status, 200, "{}", reply.text);
    let body = reply.json();
    assert_eq!(body["branch"], json!("B2"), "{body}");
    assert_eq!(body["branchName"], json!("สาขาสอง"), "{body}");
    assert_eq!(body["branches"], all_three(), "{body}");
    assert_eq!(
        session_branch(&desk.pool, &token).await.as_deref(),
        Some("B2")
    );
    assert_eq!(last_branch(&desk.pool, &user).await.as_deref(), Some("B2"));
    let data = me(&reader, &token).await;
    assert_eq!(data["branch"], json!("B2"));
    assert_eq!(data["branchName"], json!("สาขาสอง"));
    drop(reader);
    desk.drop_schema().await;
}

/// The same switch twice gives the same state (idempotent by value, §14.2).
#[actix_web::test]
async fn switching_twice_to_the_same_branch_is_the_same_as_once() {
    let (desk, identity, app, reader) = setup().await;
    let user = bound_user(&desk.pool, SUB_A, "ACTIVE", "Admin", &[("ADMIN", None)]).await;
    let token = sign_in(&app, &identity, VALID_A).await;

    let first = app.send(switch(&token, "MAIN")).await;
    let second = app.send(switch(&token, "MAIN")).await;

    assert_eq!(
        (first.status, second.status),
        (200, 200),
        "{} / {}",
        first.text,
        second.text
    );
    assert_eq!(first.json(), second.json());
    assert_eq!(
        session_branch(&desk.pool, &token).await.as_deref(),
        Some("MAIN")
    );
    assert_eq!(
        last_branch(&desk.pool, &user).await.as_deref(),
        Some("MAIN")
    );
    drop(reader);
    desk.drop_schema().await;
}

/// R3, D-4: a branch outside the user's grants, or one that does not exist, is refused `403 outOfScope` and nothing
/// is written.
#[actix_web::test]
async fn switching_outside_the_grants_is_refused_and_writes_nothing() {
    let (desk, identity, app, reader) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Mai",
        &[("RATE_MANAGER", Some("B2")), ("RATE_MANAGER", Some("B3"))],
    )
    .await;
    let token = sign_in(&app, &identity, VALID_A).await;

    for branch in ["MAIN", "NOPE"] {
        let reply = app.send(switch(&token, branch)).await;
        assert_eq!(reply.status, 403, "{branch}: {}", reply.text);
        assert_eq!(reply.refusal().0, OUT_OF_SCOPE, "{branch}: {}", reply.text);
        assert_eq!(
            session_branch(&desk.pool, &token).await.as_deref(),
            Some("B3"),
            "{branch}"
        );
        assert_eq!(last_branch(&desk.pool, &user).await, None, "{branch}");
    }
    assert_eq!(me(&reader, &token).await["branch"], json!("B3"));
    drop(reader);
    desk.drop_schema().await;
}

/// Another organisation's branch is outside every grant, organisation-wide ones included.
#[actix_web::test]
async fn an_organisation_wide_user_cannot_switch_into_another_organisation() {
    let (desk, identity, app, reader) = setup().await;
    let user = bound_user(&desk.pool, SUB_A, "ACTIVE", "Admin", &[("ADMIN", None)]).await;
    let other: String = sqlx::query_scalar(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES (gen_random_uuid(), 'OTHER', NULL, 'Asia/Bangkok', now()) RETURNING id::text",
    )
    .fetch_one(&desk.pool)
    .await
    .expect("a second organisation");
    sqlx::query("INSERT INTO branches (id, name, organization_id, code) VALUES ('X1', 'อื่น', $1::uuid, 'X1')")
        .bind(&other)
        .execute(&desk.pool)
        .await
        .expect("its branch");
    let token = sign_in(&app, &identity, VALID_A).await;

    let reply = app.send(switch(&token, "X1")).await;

    assert_eq!(reply.status, 403, "{}", reply.text);
    assert_eq!(reply.refusal().0, OUT_OF_SCOPE);
    assert_eq!(
        session_branch(&desk.pool, &token).await.as_deref(),
        Some("B3")
    );
    assert_eq!(last_branch(&desk.pool, &user).await, None);
    assert_eq!(
        me(&reader, &token).await["branches"],
        all_three(),
        "X1 is not listed"
    );
    drop(reader);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn switching_without_a_session_is_refused() {
    let (desk, _identity, app, reader) = setup().await;

    let none = app
        .send(Req::post(
            "/api/auth/session/branch",
            json!({ "branch": "B2" }),
        ))
        .await;
    let unknown = app.send(switch(&"ab".repeat(32), "B2")).await;

    for reply in [none, unknown] {
        assert_eq!(reply.status, 401, "{}", reply.text);
        assert_eq!(reply.refusal().0, "signIn", "{}", reply.text);
    }
    drop(reader);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn a_malformed_or_cross_site_switch_is_refused_and_writes_nothing() {
    let (desk, identity, app, reader) = setup().await;
    let user = bound_user(&desk.pool, SUB_A, "ACTIVE", "Admin", &[("ADMIN", None)]).await;
    let token = sign_in(&app, &identity, VALID_A).await;

    let replies = [
        app.send(Req::post("/api/auth/session/branch", json!({})).with_session(&token))
            .await,
        app.send(
            Req::post("/api/auth/session/branch", json!({ "branch": 7 })).with_session(&token),
        )
        .await,
        app.send(switch(&token, "B2").cross_site()).await,
    ];

    for reply in replies {
        assert_eq!(reply.status, 400, "{}", reply.text);
        assert_eq!(reply.refusal().0, "invalidRequest", "{}", reply.text);
    }
    assert_eq!(
        session_branch(&desk.pool, &token).await.as_deref(),
        Some("B3")
    );
    assert_eq!(last_branch(&desk.pool, &user).await, None);
    drop(reader);
    desk.drop_schema().await;
}

// ── one session, the last choice, and revocation ──────────────────────────

/// R2: the switch moves **this** session only. R5 step 2: the next sign-in starts on the last choice.
#[actix_web::test]
async fn a_switch_moves_only_its_own_session_and_the_next_sign_in_starts_on_it() {
    let (desk, identity, app, reader) = setup().await;
    bound_user(&desk.pool, SUB_A, "ACTIVE", "Admin", &[("ADMIN", None)]).await;
    let first = sign_in(&app, &identity, VALID_A).await;
    let second = sign_in(&app, &identity, VALID_A).await;
    assert_ne!(first, second);

    let reply = app.send(switch(&first, "B2")).await;
    assert_eq!(reply.status, 200, "{}", reply.text);

    assert_eq!(
        session_branch(&desk.pool, &first).await.as_deref(),
        Some("B2")
    );
    assert_eq!(
        session_branch(&desk.pool, &second).await.as_deref(),
        Some("B3"),
        "the other browser keeps its own"
    );
    assert_eq!(me(&reader, &second).await["branch"], json!("B3"));

    let third = sign_in(&app, &identity, VALID_A).await;
    assert_eq!(
        session_branch(&desk.pool, &third).await.as_deref(),
        Some("B2"),
        "the last choice"
    );
    assert_eq!(me(&reader, &third).await["branch"], json!("B2"));
    drop(reader);
    desk.drop_schema().await;
}

/// §14.2 "effective": a grant revoked mid-session moves the session on the next read, without writing.
#[actix_web::test]
async fn a_revoked_grant_moves_the_session_on_the_next_read() {
    let (desk, identity, app, reader) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Mai",
        &[("RATE_MANAGER", Some("B2")), ("RATE_MANAGER", Some("B3"))],
    )
    .await;
    let token = sign_in(&app, &identity, VALID_A).await;
    assert_eq!(app.send(switch(&token, "B2")).await.status, 200);

    sqlx::query(
        "UPDATE user_roles SET valid_to = now() - interval '1 second' \
         WHERE user_id = $1::uuid AND branch_id = 'B2'",
    )
    .bind(&user)
    .execute(&desk.pool)
    .await
    .expect("revoke the B2 row");

    let data = me(&reader, &token).await;
    assert_eq!(data["branch"], json!("B3"), "{data}");
    assert_eq!(
        data["branches"],
        json!([{ "id": "B3", "code": "A05", "name": "สาขาสาม" }])
    );
    assert_eq!(
        session_branch(&desk.pool, &token).await.as_deref(),
        Some("B2"),
        "reading never writes"
    );
    let back = app.send(switch(&token, "B2")).await;
    assert_eq!(back.status, 403, "B2 is no longer granted: {}", back.text);
    drop(reader);
    desk.drop_schema().await;
}

/// §14.5 item 1: a session opened before the migration (`branch_id` NULL) keeps working and reads the R5 branch.
#[actix_web::test]
async fn a_session_with_no_stored_branch_reads_the_r5_branch() {
    let (desk, identity, app, reader) = setup().await;
    let user = bound_user(&desk.pool, SUB_A, "ACTIVE", "Admin", &[("ADMIN", None)]).await;
    let token = sign_in(&app, &identity, VALID_A).await;
    sqlx::query("UPDATE user_sessions SET branch_id = NULL WHERE hash = $1")
        .bind(sha256_hex(&token))
        .execute(&desk.pool)
        .await
        .expect("as the migration leaves it");
    sqlx::query("UPDATE users SET last_branch_id = 'B2' WHERE id = $1::uuid")
        .bind(&user)
        .execute(&desk.pool)
        .await
        .expect("a last choice");

    assert_eq!(me(&reader, &token).await["branch"], json!("B2"));
    drop(reader);
    desk.drop_schema().await;
}
