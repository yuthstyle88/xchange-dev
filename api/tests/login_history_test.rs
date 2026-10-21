//! Up-front tests for C4 (design `docs/design/blueprint/user-management-rbac.md` §3.4 and §4, PR 3083; owner's Q5 A
//! and Q6 A): every `POST /api/auth/login` that reaches Identity leaves one `login_history` row, and a session
//! keeps Identity's `mustChangePassword` in `user_sessions.must_change_password`.
//!
//! Written before the implementation by the ticket author (claude/f03345fe) for the BE controller claude/c81c8986.
//! Do not edit, delete or weaken these tests; add your own in another file.
//!
//! These tests need M's tables (`login_history`, `user_sessions.must_change_password`). On a `main` without M they
//! fail at the first query. Identity is faked over real HTTP (`identity_signin_common::FakeIdentity`).
//!
//! `TEST_DATABASE_URL` must name a database whose name ends in `_test`; each test gets its own schema.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use identity_signin_common::*;
use serde_json::json;
use sqlx::PgPool;

const PASSWORD: &str = "pw-from-the-browser-01";
/// A phone in the form a person types it; Identity is asked with `+66812345678`.
const TYPED_PHONE: &str = "081-234-5678";
const SENT_PHONE: &str = "+66812345678";

fn login(identifier: &str) -> Req {
    Req::post(
        "/api/auth/login",
        json!({ "identifier": identifier, "password": PASSWORD }),
    )
}

async fn setup() -> (Desk, FakeIdentity, App) {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    (desk, identity, app)
}

/// One `login_history` row as `(result, user_id, organization_id, identifier_hash, host(ip))`.
type Row = (
    String,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
);

async fn history(pool: &PgPool) -> Vec<Row> {
    sqlx::query_as(
        "SELECT result, user_id::text, organization_id::text, identifier_hash, host(ip) \
         FROM login_history ORDER BY at, id",
    )
    .fetch_all(pool)
    .await
    .expect("read login_history")
}

async fn only_row(pool: &PgPool) -> Row {
    let rows = history(pool).await;
    assert_eq!(rows.len(), 1, "exactly one row per attempt, got {rows:?}");
    rows.into_iter().next().unwrap()
}

async fn session_flags(pool: &PgPool, user_id: &str) -> Vec<bool> {
    sqlx::query_scalar("SELECT must_change_password FROM user_sessions WHERE user_id = $1::uuid")
        .bind(user_id)
        .fetch_all(pool)
        .await
        .expect("read user_sessions")
}

fn identity_refuses(identity: &FakeIdentity, status: u16, code: &str) {
    identity.answer(
        "/auth/login",
        status,
        &json!({ "error": code, "message": "x" }).to_string(),
    );
}

fn login_body_with_flag(flag: Option<bool>) -> String {
    let mut body = login_body(VALID_A, REFRESH_1);
    match flag {
        Some(f) => body["mustChangePassword"] = json!(f),
        None => {
            body.as_object_mut().unwrap().remove("mustChangePassword");
        }
    }
    body.to_string()
}

/// H1: a successful sign-in leaves one OK row naming the user and the organisation. The identifier is stored only
/// as the SHA-256 of what was sent to Identity (the normalised phone), and the browser's address is stored.
#[actix_web::test]
async fn h1_a_successful_sign_in_is_recorded_ok_with_user_org_hash_and_ip() {
    let (desk, identity, app) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Ploy",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    let org = organization(&desk.pool).await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let reply = app.send(login(TYPED_PHONE)).await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(
        only_row(&desk.pool).await,
        (
            "OK".to_owned(),
            Some(user),
            Some(org),
            sha256_hex(SENT_PHONE),
            Some(BROWSER_ADDRESS.to_owned()),
        )
    );
    desk.drop_schema().await;
}

/// H2: Identity refuses the password → one BAD_PASSWORD row with no user and no organisation; the answer is
/// unchanged (401 `invalidCredentials`).
#[actix_web::test]
async fn h2_a_wrong_password_is_recorded_bad_password_without_a_user() {
    let (desk, identity, app) = setup().await;
    identity_refuses(&identity, 401, "invalid_credentials");
    let reply = app.send(login("ploy")).await;
    assert_eq!(reply.refusal().0, "invalidCredentials", "{}", reply.text);
    assert_eq!(
        only_row(&desk.pool).await,
        (
            "BAD_PASSWORD".to_owned(),
            None,
            None,
            sha256_hex("ploy"),
            Some(BROWSER_ADDRESS.to_owned()),
        )
    );
    desk.drop_schema().await;
}

/// H3: Identity signs the person in but no v3 user is bound to the subject → UNKNOWN_USER, no user.
#[actix_web::test]
async fn h3_a_subject_with_no_user_is_recorded_unknown_user() {
    let (desk, identity, app) = setup().await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let reply = app.send(login("stranger")).await;
    assert_eq!(reply.refusal().0, "signIn", "{}", reply.text);
    let (result, user, org, _, _) = only_row(&desk.pool).await;
    assert_eq!((result.as_str(), user, org), ("UNKNOWN_USER", None, None));
    desk.drop_schema().await;
}

/// H4: a bound user who is DISABLED or PENDING → NOT_ACTIVE, naming the user and the organisation, and no session.
#[actix_web::test]
async fn h4_a_disabled_or_pending_user_is_recorded_not_active_with_the_user() {
    for status in ["DISABLED", "PENDING"] {
        let (desk, identity, app) = setup().await;
        let user = bound_user(
            &desk.pool,
            SUB_A,
            status,
            "Gone",
            &[("TELLER", Some("MAIN"))],
        )
        .await;
        let org = organization(&desk.pool).await;
        identity.login_answers("/auth/login", VALID_A, REFRESH_1);
        let reply = app.send(login("gone")).await;
        assert_ne!(reply.status, 200, "{status}: {}", reply.text);
        let (result, u, o, _, _) = only_row(&desk.pool).await;
        assert_eq!(
            (result.as_str(), u, o),
            ("NOT_ACTIVE", Some(user.clone()), Some(org)),
            "{status}"
        );
        assert_eq!(sessions_of(&desk.pool, &user).await, 0, "{status}");
        desk.drop_schema().await;
    }
}

/// H5: Identity's refusal codes that are not a wrong password map as the ticket's §8 table says.
#[actix_web::test]
async fn h5_identity_refusals_map_to_their_results() {
    let rows: &[(u16, &str, &str)] = &[
        (429, "rate_limited", "RATE_LIMITED"),
        (429, "too_many_attempts", "RATE_LIMITED"),
        (423, "account_locked", "RATE_LIMITED"),
        (403, "identity_disabled", "NOT_ACTIVE"),
    ];
    for (status, code, want) in rows {
        let (desk, identity, app) = setup().await;
        identity_refuses(&identity, *status, code);
        let reply = app.send(login("ploy")).await;
        assert_ne!(reply.status, 200, "{code}");
        let (result, user, org, _, _) = only_row(&desk.pool).await;
        assert_eq!(
            (result.as_str(), user, org),
            (*want, None, None),
            "Identity code {code}"
        );
        desk.drop_schema().await;
    }
}

/// H6: Identity cannot be used (5xx, an unknown code) → UNAVAILABLE.
#[actix_web::test]
async fn h6_identity_unavailable_is_recorded_unavailable() {
    for (status, body) in [
        (503, json!({ "error": "down" }).to_string()),
        (500, "not json".to_owned()),
    ] {
        let (desk, identity, app) = setup().await;
        identity.answer("/auth/login", status, &body);
        let reply = app.send(login("ploy")).await;
        assert_eq!(reply.refusal().0, "identityUnavailable", "{}", reply.text);
        let (result, _, _, _, _) = only_row(&desk.pool).await;
        assert_eq!(result, "UNAVAILABLE", "Identity answered {status} {body}");
        desk.drop_schema().await;
    }
}

/// H7: neither the identifier nor the password is stored in any column of the row.
#[actix_web::test]
async fn h7_the_row_holds_neither_the_identifier_nor_the_password() {
    let (desk, identity, app) = setup().await;
    identity_refuses(&identity, 401, "invalid_credentials");
    app.send(login("ploy.secret.name")).await;
    let whole: String = sqlx::query_scalar("SELECT row_to_json(l)::text FROM login_history l")
        .fetch_one(&desk.pool)
        .await
        .expect("one row");
    assert!(!whole.contains("ploy.secret.name"), "{whole}");
    assert!(!whole.contains(PASSWORD), "{whole}");
    desk.drop_schema().await;
}

/// H8: a request refused before Identity is asked (cross-site, no password) leaves no row.
#[actix_web::test]
async fn h8_a_request_refused_before_identity_leaves_no_row() {
    let (desk, _identity, app) = setup().await;
    app.send(login("ploy").cross_site()).await;
    app.send(Req::post(
        "/api/auth/login",
        json!({ "identifier": "ploy" }),
    ))
    .await;
    assert_eq!(history(&desk.pool).await, Vec::<Row>::new());
    desk.drop_schema().await;
}

/// S1: Identity answers `mustChangePassword: true` → the new session row has `must_change_password = true`.
/// S2: `false` or absent → `false`.
#[actix_web::test]
async fn s1_s2_the_session_keeps_identitys_must_change_password() {
    for (flag, want) in [(Some(true), true), (Some(false), false), (None, false)] {
        let (desk, identity, app) = setup().await;
        let user = bound_user(
            &desk.pool,
            SUB_A,
            "ACTIVE",
            "Ploy",
            &[("TELLER", Some("MAIN"))],
        )
        .await;
        identity.answer("/auth/login", 200, &login_body_with_flag(flag));
        let reply = app.send(login("ploy")).await;
        assert_eq!(reply.status, 200, "{flag:?}: {}", reply.text);
        assert_eq!(
            session_flags(&desk.pool, &user).await,
            vec![want],
            "Identity said {flag:?}"
        );
        desk.drop_schema().await;
    }
}
