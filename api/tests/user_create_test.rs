//! Ticket #1157 — the shop owner/ADMIN creates staff accounts; withdraw the invite system (B6, §5.3, §9.4).
//!
//! Written before the implementation by the test-writer (gemini/warut-4, rule A).
//! Tests assert the RESULT the ticket requires:
//! 1. Shop owner/ADMIN creates account via `userCreate` (`POST /api/users`), server registers in Identity using fake Identity on 127.0.0.1:0.
//! 2. User ends ACTIVE with `auth_subject` set and signs in via `/login`.
//! 3. `PhoneTaken` is refused with `phoneTaken`, user row stays PENDING, never bound.
//! 4. Missing phone is refused `validationFailed` on phone, Identity never called.
//! 5. TELLER or unauthenticated caller is refused by policy, Identity never called.
//! 6. Identity down answers 503 `identityUnavailable`, user row stays PENDING.
//! 7. Register succeeded but bind failed writes returned `identityId` to `user_audit` and displays it.
//! 8. `create-owner` on a fresh install creates ACTIVE EXECUTIVE + ADMIN user with `must_change_password: true`.
//! 9. No password is stored on `users`.
//! 10. `POST /api/auth/invite/claim` returns 404, and public `POST /api/auth/register` returns 404.
//! 11. `user_invites` table is dropped and `users.status` CHECK becomes ('PENDING', 'ACTIVE', 'DISABLED').
//!
//! (`POST /api/employees {action:"create"}` stays in this hotfix; OR-7.m #1027 removes it with the employees table —
//! lane controller's ruling on #1157, issuecomment-6101472652.)

mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use chrono::Utc;
use identity_signin_common::*;
use serde_json::json;
use xc_auth::application::session::SessionService;

async fn setup() -> (Desk, FakeIdentity, App) {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    (desk, identity, app)
}

async fn owner_session(desk: &Desk, identity: &FakeIdentity, app: &App) -> String {
    let _ = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Owner",
        &[("ADMIN", None), ("EXECUTIVE", None)],
    )
    .await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let reply = app
        .send(Req::post(
            "/api/auth/login",
            json!({ "identifier": "+66811111111", "password": "pw-owner-01" }),
        ))
        .await;
    reply.session()
}

async fn teller_session(desk: &Desk, identity: &FakeIdentity, app: &App) -> String {
    let _ = bound_user(
        &desk.pool,
        SUB_B,
        "ACTIVE",
        "Teller",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    identity.login_answers("/auth/login", VALID_B, REFRESH_2);
    let reply = app
        .send(Req::post(
            "/api/auth/login",
            json!({ "identifier": "+66822222222", "password": "pw-teller-01" }),
        ))
        .await;
    reply.session()
}

/// §4.1 & §8 Case 1: Owner creates a staff member with a new phone.
/// Identity account made (with must_change_password=false), user ACTIVE, person signs in on /login.
#[actix_web::test]
async fn test_01_owner_creates_staff_with_new_phone_registers_in_identity_and_signs_in() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;

    // The new identity is SUB_B: the fake's login below answers VALID_B, whose `sub` is SUB_B, so the sign-in
    // resolves the row this create binds (fixed by the ticket author, claude/709ec749: was …0099, which no
    // token of the fake carries).
    const NEW_SUB: &str = SUB_B;
    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": NEW_SUB }).to_string(),
    );

    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "Somchai",
                    "username": "somchai",
                    "phone": "+66812345678",
                    "password": "temp-password-123",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }]
                }),
            )
            .with_session(&token),
        )
        .await;

    assert_eq!(reply.status, 200, "{}", reply.text);
    let body = reply.json();
    assert_eq!(body["status"], "ACTIVE");
    let user_id = body["userId"].as_str().expect("userId in response");

    let sent = identity.received("/auth/register");
    assert_eq!(sent.len(), 1, "Identity received one /auth/register");
    let sent_body = sent[0].json();
    assert_eq!(sent_body["phone"], "+66812345678");
    // Restated by the RBAC controller (claude/e17fff2a, #3155) for the owner's Q5 A (RBAC design §2, decided
    // 2026-10-18, issuecomment-6245259930): the typed password is ignored, xchange generates a 16-character
    // temporary password, answers it once, and Identity is told to force a change.
    let sent_password = sent_body["password"]
        .as_str()
        .expect("a password sent to Identity");
    assert_ne!(
        sent_password, "temp-password-123",
        "Q5 A: the typed password is ignored"
    );
    assert_eq!(
        sent_password.chars().count(),
        16,
        "Q5 A: a TempPassword is 16 characters"
    );
    assert_eq!(
        body["temporaryPassword"], sent_password,
        "Q5 A: the answer carries the generated password once"
    );
    assert_eq!(
        sent_body["mustChangePassword"], true,
        "Q5 A: must_change_password true for userCreate"
    );
    assert!(
        sent_body.get("otpCode").is_none() || sent_body["otpCode"].is_null(),
        "no otpCode sent"
    );
    assert!(
        sent_body.get("challengeId").is_none() || sent_body["challengeId"].is_null(),
        "no challengeId sent"
    );

    let row: (String, Option<String>) =
        sqlx::query_as("SELECT status, auth_subject FROM users WHERE id = $1::uuid")
            .bind(user_id)
            .fetch_one(&desk.pool)
            .await
            .expect("user in db");
    assert_eq!(row.0, "ACTIVE");
    assert_eq!(row.1.as_deref(), Some(NEW_SUB));

    identity.login_answers("/auth/login", VALID_B, REFRESH_2);
    let login_reply = app
        .send(Req::post(
            "/api/auth/login",
            json!({ "identifier": "+66812345678", "password": "temp-password-123" }),
        ))
        .await;
    assert_eq!(login_reply.status, 200, "{}", login_reply.text);
    assert_eq!(
        login_reply.json()["user"]["id"],
        json!(user_id),
        "§4.1: the session's user is the row userCreate made"
    );

    desk.drop_schema().await;
}

/// §8 Case 2: Phone already used anywhere in 108 (PhoneTaken) -> refused phoneTaken,
/// never bound to existing identity (ruling a); the row stays PENDING.
#[actix_web::test]
async fn test_02_phone_taken_refused_and_user_stays_pending_never_bound() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;

    identity.answer("/auth/register", 409, r#"{"error":"phone_taken"}"#);

    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "DupeStaff",
                    "phone": "+66899999999",
                    "password": "pw",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }]
                }),
            )
            .with_session(&token),
        )
        .await;

    assert_eq!(reply.status, 400, "{}", reply.text);
    assert_eq!(reply.refusal().0, "phoneTaken");

    let pending_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM users WHERE name = 'DupeStaff' AND status = 'PENDING' AND auth_subject IS NULL",
    )
    .fetch_one(&desk.pool)
    .await
    .expect("query pending");
    assert_eq!(pending_count, 1, "row stays PENDING and is never bound");

    let bound_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM users WHERE name = 'DupeStaff' AND auth_subject IS NOT NULL",
    )
    .fetch_one(&desk.pool)
    .await
    .expect("query bound");
    assert_eq!(bound_count, 0, "never bound to existing identity");

    desk.drop_schema().await;
}

/// §8 Case 3: No phone -> refused validationFailed on phone. Identity not called.
#[actix_web::test]
async fn test_03_no_phone_refused_validation_failed_on_phone() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;

    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "NoPhoneStaff",
                    "password": "pw",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }]
                }),
            )
            .with_session(&token),
        )
        .await;

    assert_eq!(reply.status, 400, "{}", reply.text);
    assert_eq!(reply.refusal().0, "validationFailed");
    assert!(
        identity.received("/auth/register").is_empty(),
        "Identity not called"
    );

    desk.drop_schema().await;
}

/// §4.2 & §8 Case 4: A TELLER, or nobody signed in -> refused by policy, Identity not called.
#[actix_web::test]
async fn test_04_teller_or_unauthenticated_caller_refused_by_policy_identity_not_called() {
    let (desk, identity, app) = setup().await;
    let teller_token = teller_session(&desk, &identity, &app).await;

    let reply_teller = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "StaffByTeller",
                    "phone": "+66812345678",
                    "password": "pw",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }]
                }),
            )
            .with_session(&teller_token),
        )
        .await;
    // A permission refusal on a v3 command is 400 `roleNotAllowedHere` by design (`policy_error.rs` `status()`,
    // pinned by `v3_error_test`); lane controller's ruling on #1157 (issuecomment-6101472652).
    assert_eq!(
        reply_teller.status, 400,
        "TELLER refused by policy: {}",
        reply_teller.text
    );
    assert_eq!(reply_teller.refusal().0, "roleNotAllowedHere");

    let reply_anon = app
        .send(Req::post(
            "/api/users",
            json!({
                "name": "StaffByAnon",
                "phone": "+66812345678",
                "password": "pw",
                "roles": [{ "role": "TELLER", "branch": "MAIN" }]
            }),
        ))
        .await;
    assert_eq!(
        reply_anon.status, 401,
        "unauthenticated refused: {}",
        reply_anon.text
    );

    assert!(
        identity.received("/auth/register").is_empty(),
        "Identity never called"
    );

    desk.drop_schema().await;
}

/// §8 Case 5: Identity down -> 503 identityUnavailable; the row stays PENDING, retry completes it.
#[actix_web::test]
async fn test_05_identity_down_answers_503_identity_unavailable_and_user_stays_pending() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;

    identity.answer(
        "/auth/register",
        503,
        &json!({ "error": "service_unavailable" }).to_string(),
    );

    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "DownStaff",
                    "phone": "+66877777777",
                    "password": "pw",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }]
                }),
            )
            .with_session(&token),
        )
        .await;

    assert_eq!(reply.status, 503, "{}", reply.text);
    assert_eq!(reply.refusal().0, "identityUnavailable");

    let pending_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM users WHERE name = 'DownStaff' AND status = 'PENDING' AND auth_subject IS NULL",
    )
    .fetch_one(&desk.pool)
    .await
    .expect("query");
    assert_eq!(pending_count, 1, "row stays PENDING when Identity is down");

    desk.drop_schema().await;
}

/// §7 & §8 Case 6: Register succeeded, bind failed -> refused, identityId written to user_audit and shown.
#[actix_web::test]
async fn test_06_register_succeeded_bind_failed_writes_identity_id_to_user_audit() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;

    const CONFLICT_SUB: &str = "11111111-aaaa-4aaa-8aaa-000000000099";
    let _ = bound_user(&desk.pool, CONFLICT_SUB, "ACTIVE", "AlreadyBoundUser", &[]).await;

    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": CONFLICT_SUB }).to_string(),
    );

    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "BindFailStaff",
                    "phone": "+66866666666",
                    "password": "pw",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }]
                }),
            )
            .with_session(&token),
        )
        .await;

    assert!(
        reply.status >= 400,
        "refused on bind failure: {}",
        reply.text
    );
    assert!(
        reply.text.contains(CONFLICT_SUB),
        "refusal displays identityId: {}",
        reply.text
    );

    let pending_user_id: String = sqlx::query_scalar(
        "SELECT id::text FROM users WHERE name = 'BindFailStaff' AND status = 'PENDING'",
    )
    .fetch_one(&desk.pool)
    .await
    .expect("pending user in db");

    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_audit WHERE subject_id = $1 AND action LIKE '%' || $2 || '%'",
    )
    .bind(&pending_user_id)
    .bind(CONFLICT_SUB)
    .fetch_one(&desk.pool)
    .await
    .expect("query audit");
    assert_eq!(
        audit_count, 1,
        "user_audit row with subject_id = pending user's id and action containing identityId"
    );

    desk.drop_schema().await;
}

/// §4.5 & §8 Case 7: create-owner on a fresh install -> one ACTIVE EXECUTIVE + ADMIN user who signs in.
#[actix_web::test]
async fn test_07_create_owner_on_fresh_install_makes_active_executive_admin_user_who_signs_in() {
    let (desk, identity, app) = setup().await;

    // SUB_A: the fake's login below answers VALID_A, whose `sub` is SUB_A (fixed by the ticket author: was …0077).
    const ROOT_SUB: &str = SUB_A;
    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": ROOT_SUB }).to_string(),
    );

    // create_owner talks to Identity: the service gets a client pointed at the fake, built from the same settings
    // the app uses (`presentation/http/auth.rs:44-50`). Fixed by the ticket author: was `None`, so the fake
    // could never receive `/auth/register`.
    let settings = state_with(&desk.pool, &identity_env(&identity.url()))
        .settings
        .identity
        .clone()
        .expect("identity settings from identity_env");
    let client: std::sync::Arc<dyn xc_auth::domain::identity::identity_client::IdentityClient> =
        std::sync::Arc::new(xc_auth::infrastructure::identity::HttpIdentityClient::new(
            settings,
            "x-forwarded-for",
        ));
    let service = SessionService::new(desk.pool.clone(), Some(client), None, None);
    let now = Utc::now();
    let res = service
        .create_owner("Root Owner", "+66810000001", "root-password-01", now)
        .await;
    assert!(res.is_ok(), "create_owner succeeds: {:?}", res.err());

    let owner_row: (String, Option<String>) =
        sqlx::query_as("SELECT status, auth_subject FROM users WHERE name = 'Root Owner'")
            .fetch_one(&desk.pool)
            .await
            .expect("owner in db");
    assert_eq!(owner_row.0, "ACTIVE");
    assert_eq!(owner_row.1.as_deref(), Some(ROOT_SUB));

    let roles: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT role_code, branch_id FROM user_roles r JOIN users u ON u.id = r.user_id WHERE u.name = 'Root Owner' ORDER BY role_code",
    )
    .fetch_all(&desk.pool)
    .await
    .expect("roles");
    assert_eq!(
        roles,
        vec![("ADMIN".to_string(), None), ("EXECUTIVE".to_string(), None)]
    );

    let sent = identity.received("/auth/register");
    assert_eq!(sent.len(), 1, "Identity received one /auth/register");
    assert_eq!(
        sent[0].json()["mustChangePassword"],
        true,
        "create-owner has must_change_password: true"
    );

    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let login_reply = app
        .send(Req::post(
            "/api/auth/login",
            json!({ "identifier": "+66810000001", "password": "root-password-01" }),
        ))
        .await;
    assert_eq!(login_reply.status, 200, "{}", login_reply.text);
    let owner_id: String =
        sqlx::query_scalar("SELECT id::text FROM users WHERE name = 'Root Owner'")
            .fetch_one(&desk.pool)
            .await
            .expect("owner id");
    assert_eq!(
        login_reply.json()["user"]["id"],
        json!(owner_id),
        "§4.5: the session's user is the owner create-owner made"
    );

    desk.drop_schema().await;
}

/// §4.4: No password stored: `userCreate` writes no column named like `password` on `users`.
#[actix_web::test]
async fn user_create_stores_no_password() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;

    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": "11111111-aaaa-4aaa-8aaa-000000000088" }).to_string(),
    );

    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "NoHashStaff",
                    "phone": "+66815555555",
                    "password": "super-secret-pw",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }]
                }),
            )
            .with_session(&token),
        )
        .await;
    assert_eq!(reply.status, 200, "{}", reply.text);

    let password_cols: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'users' AND column_name ILIKE '%password%'",
    )
    .fetch_one(&desk.pool)
    .await
    .expect("query information_schema.columns");
    assert_eq!(
        password_cols, 0,
        "users table has no column named like password"
    );

    desk.drop_schema().await;
}

/// §4.3 & §4.6: No invite left: POST /api/auth/invite/claim answers 404,
/// and public POST /api/auth/register answers 404.
#[actix_web::test]
async fn test_09_claim_invite_route_returns_404_and_no_public_register_route() {
    let (desk, _identity, app) = setup().await;

    let reply_claim = app
        .send(Req::post(
            "/api/auth/invite/claim",
            json!({ "inviteCode": "any-code-1234", "password": "pw" }),
        ))
        .await;
    assert_eq!(
        reply_claim.status, 404,
        "claim route is withdrawn / answers 404: {}",
        reply_claim.text
    );

    let reply_reg = app
        .send(Req::post(
            "/api/auth/register",
            json!({ "phone": "+66812345678", "password": "pw" }),
        ))
        .await;
    assert_eq!(
        reply_reg.status, 404,
        "public register is 404: {}",
        reply_reg.text
    );

    desk.drop_schema().await;
}

/// §4.6 & §9: Data migration: DROP TABLE user_invites, INVITED renamed PENDING,
/// users.status CHECK becomes ('PENDING','ACTIVE','DISABLED').
#[actix_web::test]
async fn test_10_user_invites_table_dropped_and_users_status_check_is_pending_active_disabled() {
    // A schema of its own, migrated in two steps: everything up to main's newest migration at the test base
    // (24ad08ae: 20261110130000_referral_attributions), then the seed, then the rest, which holds this ticket's
    // migration. `Desk::start` cannot be used here: it applies every migration first, so with the new CHECK in place
    // the INVITED seed itself would be refused (ticket author's review, issuecomment-6101468237).
    const LAST_BEFORE_THIS_TICKET: i64 = 20261110130000;
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
    let options: sqlx::postgres::PgConnectOptions = url.parse().expect("a PostgreSQL URL");
    let schema = format!(
        "t1157_{}",
        uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .get(..12)
            .unwrap_or("x")
    );
    let admin = sqlx::PgPool::connect_with(options.clone())
        .await
        .expect("connect to the test database");
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("create the schema");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .expect("connect to the schema");
    let migrations = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut before = sqlx::migrate::Migrator::new(migrations.as_path())
        .await
        .expect("migrator");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version <= LAST_BEFORE_THIS_TICKET)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before
        .run(&pool)
        .await
        .expect("migrate up to the last migration before this ticket's");
    let org = organization(&pool).await;

    // Seed one INVITED user before this ticket's migration
    let user_id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, version, created_by) \
         VALUES ($1::uuid, 'SeedInvited', 'INVITED', 1, 'test') RETURNING id::text",
    )
    .bind(&org)
    .fetch_one(&pool)
    .await
    .expect("seed an INVITED user before this ticket's migration");

    sqlx::query(
        "INSERT INTO user_invites (user_id, code_hash, expires_at) \
         VALUES ($1::uuid, 'dummy_hash', now() + interval '7 days')",
    )
    .bind(&user_id)
    .execute(&pool)
    .await
    .expect("seed an unspent invite");

    // Run migrations including this ticket's migration
    let mut migrator = sqlx::migrate::Migrator::new(migrations.as_path())
        .await
        .expect("migrator");
    migrator.set_ignore_missing(true);
    migrator
        .run(&pool)
        .await
        .expect("run migrations including this ticket's migration");

    // 1. Existing INVITED user must be backfilled to PENDING
    let status: String = sqlx::query_scalar("SELECT status FROM users WHERE id = $1::uuid")
        .bind(&user_id)
        .fetch_one(&pool)
        .await
        .expect("fetch user");
    assert_eq!(
        status, "PENDING",
        "§9 backfill: existing INVITED user becomes PENDING"
    );

    // 2. user_invites table is dropped
    let table_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema = current_schema() AND table_name = 'user_invites'",
    )
    .fetch_one(&pool)
    .await
    .expect("query table");
    assert_eq!(table_count, 0, "user_invites table does not exist");

    // 3. Status check constraint: PENDING is accepted, INVITED is rejected
    let pending_insert = sqlx::query(
        "INSERT INTO users (organization_id, name, status, version, created_by) VALUES ($1::uuid, 'TestPending', 'PENDING', 1, 'test')",
    )
    .bind(&org)
    .execute(&pool)
    .await;
    assert!(
        pending_insert.is_ok(),
        "inserting 'PENDING' must succeed: {:?}",
        pending_insert.err()
    );

    let invited_insert = sqlx::query(
        "INSERT INTO users (organization_id, name, status, version, created_by) VALUES ($1::uuid, 'TestInvited', 'INVITED', 1, 'test')",
    )
    .bind(&org)
    .execute(&pool)
    .await;
    assert!(
        invited_insert.is_err(),
        "inserting 'INVITED' must be rejected by check constraint"
    );

    let invited_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE status = 'INVITED'")
            .fetch_one(&pool)
            .await
            .expect("query count");
    assert_eq!(invited_count, 0, "no INVITED users in users table");

    pool.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .expect("drop the schema");
    admin.close().await;
}
