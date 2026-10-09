//! #763 (login follow-up of #762): an `inviteRefused` refusal names its cause by code.
//!
//! Before this ticket `UserService::claim` (`api/crates/xc-auth/src/application/user/user_service.rs:141-159`
//! @ abb7ea1) raised four `AppError::Domain(<Thai text>)` and `session_service.rs:77` mapped all four to
//! `(400, "inviteRefused", msg)`, so the message was the only thing that told the four apart on the wire.
//! The web must never branch on that message (W10), so the server names the cause:
//!
//!   [required] `InviteRefusal::{InviteInvalid, AlreadyLinked, UserNotFound, UserDisabled}`, serde camelCase,
//!   sent as `{"error": {"code": "inviteRefused", "reason": "<value>", "message": ...}}`;
//!   `code` stays `inviteRefused` (HTTP 400) for all four.
//!
//! It also pins that `POST /api/auth/register` answers a taken phone with the code `alreadyRegistered`, which the
//! invite step maps to its own sentence (lane controller's ruling on #763, 2026-10-10).
//!
//! Each of the four refusal paths is driven through `POST /api/auth/invite/claim` with OR-3's harness
//! (`identity_signin_common`: Identity faked over real HTTP, fixed Ed25519 tokens).
//!
//! Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! `TEST_DATABASE_URL` must name a database whose name ends in `_test`; each test gets its own schema.

mod auth_http_common;
mod common;
mod identity_signin_common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use auth_http_common::Desk;
use chrono::{Duration, Utc};
use identity_signin_common::*;
use serde_json::json;
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::application::user::UserService;
use xc_auth::domain::access::{Role, ScopedRole};
use xc_auth::infrastructure::user::PgUserRepository;
use xc_kernel::ids::BranchId;

async fn setup() -> (Desk, FakeIdentity, App) {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    (desk, identity, app)
}

/// An invite for a new TELLER at MAIN (OR-2's `UserService::invite`): (users.id, code).
async fn invite(pool: &PgPool) -> (String, String) {
    let org = organization(pool).await;
    let now = Utc::now();
    let mut tx = pool.begin().await.expect("begin");
    let (user, code) = UserService::new(Arc::new(PgUserRepository::new()))
        .invite(
            &mut tx,
            &org,
            "Invited Teller",
            vec![ScopedRole::new(
                Role::Teller,
                Some(BranchId::new("MAIN")),
                now - Duration::days(1),
                None,
            )],
            "test",
            now,
            now + Duration::days(7),
        )
        .await
        .expect("invite");
    tx.commit().await.expect("commit");
    (user.id, code)
}

fn claim(code: &str) -> Req {
    Req::post(
        "/api/auth/invite/claim",
        json!({ "identifier": "+66812345678", "password": "pw-from-the-browser-01", "inviteCode": code }),
    )
}

/// The refusal is `400 inviteRefused` with this `reason`, a non-empty message, and no session.
fn refused_for(reply: &Reply, reason: &str) {
    assert_eq!(reply.status, 400, "{reason}: {}", reply.text);
    let body = reply.json();
    assert_eq!(
        body["error"]["code"],
        json!("inviteRefused"),
        "{reason}: code stays inviteRefused: {}",
        reply.text
    );
    assert_eq!(
        body["error"]["reason"],
        json!(reason),
        "the refusal names its cause in error.reason: {}",
        reply.text
    );
    let message = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        !message.is_empty(),
        "{reason}: the message is still sent: {}",
        reply.text
    );
    assert!(reply.sets_no_session(), "{reason}: {}", reply.text);
}

// ── inviteInvalid: the code is unknown, already spent, or expired ─────────

#[actix_web::test]
async fn an_unknown_code_is_invite_invalid() {
    let (desk, identity, app) = setup().await;
    invite(&desk.pool).await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let reply = app.send(claim("NO-SUCH-CODE-763")).await;
    refused_for(&reply, "inviteInvalid");
    assert_eq!(all_sessions(&desk.pool).await, 0);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn a_spent_code_is_invite_invalid() {
    let (desk, identity, app) = setup().await;
    let (_user, code) = invite(&desk.pool).await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let first = app.send(claim(&code)).await;
    assert_eq!(first.status, 200, "{}", first.text);

    identity.login_answers("/auth/login", VALID_B, REFRESH_2);
    let second = app.send(claim(&code)).await;
    refused_for(&second, "inviteInvalid");
    assert_eq!(all_sessions(&desk.pool).await, 1);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn an_expired_code_is_invite_invalid() {
    let (desk, identity, app) = setup().await;
    let (user, code) = invite(&desk.pool).await;
    sqlx::query(
        "UPDATE user_invites SET expires_at = now() - interval '1 hour' WHERE user_id = $1::uuid",
    )
    .bind(&user)
    .execute(&desk.pool)
    .await
    .expect("expire the invite");
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let reply = app.send(claim(&code)).await;
    refused_for(&reply, "inviteInvalid");
    assert_eq!(all_sessions(&desk.pool).await, 0);
    desk.drop_schema().await;
}

// ── alreadyLinked: the Identity subject is already bound to a user ────────

#[actix_web::test]
async fn a_subject_already_bound_is_already_linked() {
    let (desk, identity, app) = setup().await;
    let (_first_user, first_code) = invite(&desk.pool).await;
    let (_second_user, second_code) = invite(&desk.pool).await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    assert_eq!(app.send(claim(&first_code)).await.status, 200);

    let reply = app.send(claim(&second_code)).await;
    refused_for(&reply, "alreadyLinked");
    assert_eq!(all_sessions(&desk.pool).await, 1);
    desk.drop_schema().await;
}

// ── userNotFound: the invite names a user row that is gone ────────────────

#[actix_web::test]
async fn an_invite_whose_user_is_gone_is_user_not_found() {
    let (desk, identity, app) = setup().await;
    let (user, code) = invite(&desk.pool).await;
    // `user_invites.user_id` cascades on delete (migration 20261011110000), so the only way to reach this branch
    // of `claim` is an invite row left without its user. This test's own schema drops the foreign key first.
    let fks: Vec<String> = sqlx::query_scalar(
        "SELECT conname::text FROM pg_constraint \
         WHERE conrelid = 'user_invites'::regclass AND contype = 'f'",
    )
    .fetch_all(&desk.pool)
    .await
    .expect("user_invites foreign keys");
    for fk in fks {
        sqlx::query(AssertSqlSafe(format!(
            "ALTER TABLE user_invites DROP CONSTRAINT \"{fk}\""
        )))
        .execute(&desk.pool)
        .await
        .expect("drop the invite's foreign key in this test's schema");
    }
    sqlx::query("DELETE FROM users WHERE id = $1::uuid")
        .bind(&user)
        .execute(&desk.pool)
        .await
        .expect("delete the invited user");

    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let reply = app.send(claim(&code)).await;
    refused_for(&reply, "userNotFound");
    assert_eq!(all_sessions(&desk.pool).await, 0);
    desk.drop_schema().await;
}

// ── userDisabled: the invited user was disabled before claiming ───────────

#[actix_web::test]
async fn an_invite_for_a_disabled_user_is_user_disabled() {
    let (desk, identity, app) = setup().await;
    let (user, code) = invite(&desk.pool).await;
    sqlx::query("UPDATE users SET status = 'DISABLED' WHERE id = $1::uuid")
        .bind(&user)
        .execute(&desk.pool)
        .await
        .expect("disable the invited user");

    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let reply = app.send(claim(&code)).await;
    // Not OR-3's sign-in refusal `403 userDisabled`: a refused claim is always `400 inviteRefused`.
    refused_for(&reply, "userDisabled");
    let (subject, status): (Option<String>, String) =
        sqlx::query_as("SELECT auth_subject, status FROM users WHERE id = $1::uuid")
            .bind(&user)
            .fetch_one(&desk.pool)
            .await
            .expect("the user");
    assert_eq!(
        (subject, status),
        (None, "DISABLED".to_owned()),
        "nothing bound"
    );
    assert_eq!(all_sessions(&desk.pool).await, 0);
    desk.drop_schema().await;
}

// ── register: a phone that already has an account ─────────────────────────

/// The invite step's "create one" tells "this phone already has an account" apart from every other register
/// refusal by the BFF's **code** `alreadyRegistered` (409), mapped from Identity's error code, never from
/// Identity's HTTP status (`http_identity_client.rs` `username_taken | email_taken | phone_taken`,
/// `identity_client.rs:95`). OR-3 pins that mapping on `/api/auth/login` only; this pins it on
/// `/api/auth/register`, the route the screen calls.
#[actix_web::test]
async fn register_for_a_taken_phone_answers_already_registered_by_code() {
    let (desk, identity, app) = setup().await;
    for (identity_status, identity_code) in [
        (409, "phone_taken"),
        (409, "email_taken"),
        (409, "username_taken"),
        // Same code, another status: the code decides.
        (400, "phone_taken"),
    ] {
        identity.answer(
            "/auth/register",
            identity_status,
            &json!({ "error": identity_code }).to_string(),
        );
        let reply = app
            .send(Req::post(
                "/api/auth/register",
                json!({ "phone": "+66812345678", "password": "pw-from-the-browser-01" }),
            ))
            .await;
        assert_eq!(
            reply.status, 409,
            "{identity_status} {identity_code}: {}",
            reply.text
        );
        assert_eq!(
            reply.json()["error"]["code"],
            json!("alreadyRegistered"),
            "{identity_status} {identity_code}: {}",
            reply.text
        );
        assert!(reply.sets_no_session(), "{identity_code}");
    }
    assert_eq!(all_sessions(&desk.pool).await, 0);
    desk.drop_schema().await;
}

// ── the type behind the wire value ─────────────────────────────────────────

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// [required] `reason` is a typed enum in xc-auth, serialised with serde camelCase, not a formatted string.
#[test]
fn the_reason_is_a_typed_camel_case_enum_in_xc_auth() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/xc-auth/src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    let found: Vec<(PathBuf, String)> = files
        .into_iter()
        .map(|p| {
            let text = read(&p);
            (p, text)
        })
        .filter(|(_, text)| text.contains("enum InviteRefusal"))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one `enum InviteRefusal` under api/crates/xc-auth/src: {:?}",
        found.iter().map(|(p, _)| p).collect::<Vec<_>>()
    );
    let text = &found[0].1;
    let at = text.find("enum InviteRefusal").expect("found above");
    // The attributes just above the enum, and its body.
    let head = &text[text[..at].rfind("\n\n").unwrap_or(0)..at];
    let body_end = text[at..].find('}').map(|i| at + i).expect("enum body");
    let body = &text[at..body_end];
    assert!(
        head.contains("rename_all = \"camelCase\""),
        "InviteRefusal is serialised with serde camelCase: {head}"
    );
    for variant in [
        "InviteInvalid",
        "AlreadyLinked",
        "UserNotFound",
        "UserDisabled",
    ] {
        assert!(
            body.contains(variant),
            "InviteRefusal has {variant}: {body}"
        );
    }
}

/// The reason is decided where each refusal is raised, never read back out of the Thai message: the mapping
/// (`session_service.rs`) and the response (`auth.rs`) hold none of `claim`'s refusal sentences.
#[test]
fn the_reason_is_not_derived_from_the_message() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for rel in [
        "crates/xc-auth/src/application/session/session_service.rs",
        "src/presentation/http/auth.rs",
    ] {
        let text = read(&root.join(rel));
        for sentence in [
            "รหัสเชิญไม่ถูกต้อง ถูกใช้งานแล้ว หรือหมดอายุแล้ว",
            "ผู้ใช้นี้ลงทะเบียนแล้ว",
            "ไม่พบผู้ใช้งาน",
        ] {
            assert!(
                !text.contains(sentence),
                "{rel} matches claim's sentence {sentence:?}"
            );
        }
    }
}
