//! Up-front tests for F2-BE (#5835): `POST /api/users/reset-password` (`userResetPassword`). An admin
//! resets a staff member's password through Identity's `POST /admin/identities/{id}/password` (ID-1,
//! `108-platform-services#459`) and sees the temporary password once. Design
//! `docs/design/blueprint/user-management-rbac.md` §13.
//!
//! Written by claude/b091f5e0 (BE sub-controller) before the fix. Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! Callers are the `x-test-as` Blueprint callers of `rbac_audit_ip_test.rs` over the whole route table;
//! Identity is the fake of `identity_signin_common` over real HTTP. Needs `TEST_DATABASE_URL`; nothing
//! skips.

#![allow(dead_code)]

mod common;
mod identity_signin_common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use identity_signin_common::{API_KEY, FakeIdentity, identity_env};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_testdb::people::{RoleRow, seed_roles};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const ORG_ADMIN: &str = "ADMIN@*";
const OWNER: &str = "EXECUTIVE@*+ADMIN@*";
const B09_ADMIN: &str = "ADMIN@XC-B09";
const ROUTE: &str = "/api/users/reset-password";
const INTERNAL_TOKEN: &str = "test-identity-internal-token-0001";
const TELLER_SUBJECT: &str = "22222222-bbbb-4bbb-8bbb-000000000001";
const ADMIN_SUBJECT: &str = "22222222-bbbb-4bbb-8bbb-000000000002";
const ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn identity_path(subject: &str) -> String {
    format!("/admin/identities/{subject}/password")
}

fn is_temp_password(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| ALPHABET.contains(&b))
}

// ------------------------------------------------------------ database

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("rbac_f2_reset");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B09', 'B09')")
            .execute(&pool)
            .await
            .expect("insert the branches");
        Self { pool, schema }
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn version(&self, user: &str) -> i64 {
        self.count(&format!(
            "SELECT version::bigint FROM users WHERE id = '{user}'::uuid"
        ))
        .await
    }

    async fn audits(&self, user: &str) -> i64 {
        self.count(&format!(
            "SELECT count(*) FROM user_audit WHERE action = 'userResetPassword' AND subject_id = '{user}'"
        ))
        .await
    }

    /// Rows of `table` whose whole text holds `needle`.
    async fn rows_holding(&self, table: &str, needle: &str) -> i64 {
        self.count(&format!(
            "SELECT count(*) FROM {table} t WHERE t::text LIKE '%{needle}%'"
        ))
        .await
    }

    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

/// A user of XC-B01's organisation with these roles; its `users.id`.
async fn a_user(db: &TestDb, auth_subject: Option<&str>, roles: &[(&str, Option<&str>)]) -> String {
    let name = unique("u");
    let user: String = sqlx::query_scalar(AssertSqlSafe(format!(
        "INSERT INTO users (organization_id, auth_subject, name, status, created_by) \
         SELECT organization_id, {}, '{name}', 'ACTIVE', 'test' FROM branches WHERE id = '{B01}' \
         RETURNING id::text",
        auth_subject.map_or("NULL".to_owned(), |s| format!("'{s}'")),
    )))
    .fetch_one(&db.pool)
    .await
    .expect("a user of XC-B01's organisation");
    let rows: Vec<RoleRow<'_>> = roles
        .iter()
        .map(|&(role_code, branch_id)| RoleRow {
            role_code,
            branch_id,
            valid_from: DateTime::<Utc>::UNIX_EPOCH,
            valid_to: None,
        })
        .collect();
    let mut conn = db.pool.acquire().await.expect("acquire connection");
    seed_roles(
        &mut conn,
        &user,
        xchange::presentation::pages::SEEDED_XC_ORG_ID,
        &rows,
    )
    .await
    .expect("insert user roles");
    user
}

async fn a_teller(db: &TestDb) -> String {
    a_user(db, Some(TELLER_SUBJECT), &[("TELLER", Some(B01))]).await
}

// ------------------------------------------------------------ callers

struct HeaderUsers;

fn user_id(spec: &str) -> String {
    match spec.split_once('#') {
        Some((_, id)) => id.to_owned(),
        None => format!("user-{}", spec.replace(['@', '*', '+'], "-")),
    }
}

fn session_branch(rows: &[ScopedRole]) -> Option<BranchId> {
    if rows.is_empty() {
        return None;
    }
    if rows.iter().any(|r| r.branch.is_none()) {
        return Some(BranchId::new("MAIN"));
    }
    let first = rows.first().and_then(|r| r.branch.as_ref())?;
    if rows.iter().all(|r| r.branch.as_ref() == Some(first)) {
        Some(first.clone())
    } else {
        None
    }
}

fn user_of(spec: &str) -> Option<User> {
    let roles_part = spec.split_once('#').map_or(spec, |(r, _)| r);
    let mut roles = Vec::new();
    for one in roles_part.split('+') {
        let (role, branch) = one.split_once('@')?;
        let role = match role {
            "FINANCE" => Role::Finance,
            "ADMIN" => Role::Admin,
            "EXECUTIVE" => Role::Executive,
            "BRANCH_MANAGER" => Role::BranchManager,
            "TELLER" => Role::Teller,
            other => panic!("unknown role {other}"),
        };
        let branch = (branch != "*").then(|| BranchId::new(branch));
        roles.push(ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        ));
    }
    let branch = session_branch(&roles);
    Some(
        User::new(
            &user_id(spec),
            xchange::presentation::pages::SEEDED_XC_ORG_ID,
            roles,
            None,
        )
        .with_branch(branch),
    )
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

/// The server's environment: Identity at the fake, with the internal token unless `with_token` is false.
fn env_for(identity: &FakeIdentity, with_token: bool) -> Vec<(&'static str, String)> {
    let mut env = identity_env(&identity.url());
    if with_token {
        env.push(("IDENTITY_INTERNAL_TOKEN", INTERNAL_TOKEN.to_owned()));
    }
    env
}

async fn reset(
    db: &TestDb,
    env: &[(&str, String)],
    who: &str,
    user: &str,
    key: Option<&str>,
) -> Answer {
    let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let state = AppState::new(Arc::new(common::settings(&pairs)), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let key = key.map_or_else(|| unique("key"), str::to_owned);
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(ROUTE)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", key))
        .set_payload(json!({ "userId": user }).to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn identity_ok(identity: &FakeIdentity, subject: &str) {
    identity.answer(&identity_path(subject), 200, r#"{"reset":true}"#);
}

// ------------------------------------------------------------ behaviour table

#[actix_web::test]
async fn org_admin_resets_a_teller_and_identity_gets_the_same_password() {
    let db = TestDb::start().await;
    let identity = FakeIdentity::start();
    identity_ok(&identity, TELLER_SUBJECT);
    let teller = a_teller(&db).await;

    let ans = reset(&db, &env_for(&identity, true), ORG_ADMIN, &teller, None).await;
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(ans.body["userId"], json!(teller));
    assert_eq!(ans.body["mustChangePassword"], json!(true));
    let temp = ans.body["temporaryPassword"]
        .as_str()
        .expect("temporaryPassword")
        .to_owned();
    assert!(is_temp_password(&temp), "not a TempPassword: {temp}");

    let sent = identity.received(&identity_path(TELLER_SUBJECT));
    assert_eq!(sent.len(), 1, "Identity called once");
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].json()["newPassword"], json!(temp));
    assert_eq!(sent[0].json()["mustChangePassword"], json!(true));
    assert_eq!(sent[0].header("x-internal-token"), Some(INTERNAL_TOKEN));
    assert_eq!(sent[0].header("x-api-key"), Some(API_KEY));

    db.finish().await;
}

#[actix_web::test]
async fn the_reset_is_audited_without_the_password_and_moves_the_version() {
    let db = TestDb::start().await;
    let identity = FakeIdentity::start();
    identity_ok(&identity, TELLER_SUBJECT);
    let teller = a_teller(&db).await;
    let before = db.version(&teller).await;

    let ans = reset(&db, &env_for(&identity, true), ORG_ADMIN, &teller, None).await;
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    let temp = ans.body["temporaryPassword"]
        .as_str()
        .expect("temporaryPassword");

    assert_eq!(db.audits(&teller).await, 1);
    assert_eq!(db.rows_holding("user_audit", temp).await, 0);
    assert!(
        db.version(&teller).await > before,
        "users.version must move so the teller's sessions end"
    );

    db.finish().await;
}

#[actix_web::test]
async fn the_idempotency_record_never_holds_the_password() {
    let db = TestDb::start().await;
    let identity = FakeIdentity::start();
    identity_ok(&identity, TELLER_SUBJECT);
    let teller = a_teller(&db).await;

    let ans = reset(&db, &env_for(&identity, true), ORG_ADMIN, &teller, None).await;
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    let temp = ans.body["temporaryPassword"]
        .as_str()
        .expect("temporaryPassword");

    assert!(
        db.rows_holding("idempotency_records", &teller).await >= 1,
        "the command keeps an idempotency record"
    );
    assert_eq!(db.rows_holding("idempotency_records", temp).await, 0);

    db.finish().await;
}

#[actix_web::test]
async fn a_replay_answers_without_the_password() {
    let db = TestDb::start().await;
    let identity = FakeIdentity::start();
    identity_ok(&identity, TELLER_SUBJECT);
    let teller = a_teller(&db).await;
    let env = env_for(&identity, true);
    let key = unique("replay");

    let first = reset(&db, &env, ORG_ADMIN, &teller, Some(&key)).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let temp = first.body["temporaryPassword"]
        .as_str()
        .expect("temporaryPassword")
        .to_owned();

    let again = reset(&db, &env, ORG_ADMIN, &teller, Some(&key)).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert_eq!(again.body["userId"], json!(teller));
    assert!(
        !again.text.contains(&temp),
        "a replay must not carry the password: {}",
        again.text
    );
    assert_eq!(identity.received(&identity_path(TELLER_SUBJECT)).len(), 1);
    assert_eq!(db.audits(&teller).await, 1);

    db.finish().await;
}

#[actix_web::test]
async fn a_branch_admin_cannot_reset_another_branchs_teller() {
    let db = TestDb::start().await;
    let identity = FakeIdentity::start();
    identity_ok(&identity, TELLER_SUBJECT);
    let teller = a_teller(&db).await;
    let before = db.version(&teller).await;

    let ans = reset(&db, &env_for(&identity, true), B09_ADMIN, &teller, None).await;
    assert_eq!(ans.status, StatusCode::FORBIDDEN, "{}", ans.text);
    assert_eq!(
        ans.body["error"]["code"],
        json!("outOfScope"),
        "{}",
        ans.text
    );
    assert!(identity.received(&identity_path(TELLER_SUBJECT)).is_empty());
    assert_eq!(db.audits(&teller).await, 0);
    assert_eq!(db.version(&teller).await, before);

    db.finish().await;
}

#[actix_web::test]
async fn a_non_owner_cannot_reset_an_admin() {
    let db = TestDb::start().await;
    let identity = FakeIdentity::start();
    identity_ok(&identity, ADMIN_SUBJECT);
    let admin = a_user(&db, Some(ADMIN_SUBJECT), &[("ADMIN", Some(B01))]).await;

    let ans = reset(&db, &env_for(&identity, true), ORG_ADMIN, &admin, None).await;
    assert_eq!(ans.status, StatusCode::BAD_REQUEST, "{}", ans.text);
    assert_eq!(
        ans.body["error"]["code"],
        json!("roleNotAllowed"),
        "{}",
        ans.text
    );
    assert!(identity.received(&identity_path(ADMIN_SUBJECT)).is_empty());
    assert_eq!(db.audits(&admin).await, 0);

    db.finish().await;
}

#[actix_web::test]
async fn an_owner_may_reset_an_admin() {
    let db = TestDb::start().await;
    let identity = FakeIdentity::start();
    identity_ok(&identity, ADMIN_SUBJECT);
    let admin = a_user(&db, Some(ADMIN_SUBJECT), &[("ADMIN", Some(B01))]).await;

    let ans = reset(&db, &env_for(&identity, true), OWNER, &admin, None).await;
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(identity.received(&identity_path(ADMIN_SUBJECT)).len(), 1);

    db.finish().await;
}

#[actix_web::test]
async fn nobody_resets_their_own_password_here() {
    let db = TestDb::start().await;
    let identity = FakeIdentity::start();
    identity_ok(&identity, ADMIN_SUBJECT);
    let me = a_user(&db, Some(ADMIN_SUBJECT), &[("ADMIN", None)]).await;

    let ans = reset(
        &db,
        &env_for(&identity, true),
        &format!("{ORG_ADMIN}#{me}"),
        &me,
        None,
    )
    .await;
    assert_eq!(ans.status, StatusCode::FORBIDDEN, "{}", ans.text);
    assert_eq!(
        ans.body["error"]["code"],
        json!("selfChange"),
        "{}",
        ans.text
    );
    assert!(identity.received(&identity_path(ADMIN_SUBJECT)).is_empty());

    db.finish().await;
}

#[actix_web::test]
async fn a_user_without_an_identity_is_not_bound() {
    let db = TestDb::start().await;
    let identity = FakeIdentity::start();
    let unbound = a_user(&db, None, &[("TELLER", Some(B01))]).await;
    let before = db.version(&unbound).await;

    let ans = reset(&db, &env_for(&identity, true), ORG_ADMIN, &unbound, None).await;
    assert_eq!(ans.status, StatusCode::CONFLICT, "{}", ans.text);
    assert_eq!(ans.body["error"]["code"], json!("notBound"), "{}", ans.text);
    assert!(identity.calls().is_empty(), "Identity must not be called");
    assert_eq!(db.audits(&unbound).await, 0);
    assert_eq!(db.version(&unbound).await, before);

    db.finish().await;
}

#[actix_web::test]
async fn identity_refusing_rolls_everything_back() {
    let db = TestDb::start().await;
    let identity = FakeIdentity::start();
    identity.answer(
        &identity_path(TELLER_SUBJECT),
        500,
        r#"{"error":"internal_error"}"#,
    );
    let teller = a_teller(&db).await;
    let before = db.version(&teller).await;

    let ans = reset(&db, &env_for(&identity, true), ORG_ADMIN, &teller, None).await;
    assert_eq!(ans.status, StatusCode::SERVICE_UNAVAILABLE, "{}", ans.text);
    assert_eq!(
        ans.body["error"]["code"],
        json!("identityUnavailable"),
        "{}",
        ans.text
    );
    assert!(ans.body.get("temporaryPassword").is_none());
    assert_eq!(identity.received(&identity_path(TELLER_SUBJECT)).len(), 1);
    assert_eq!(db.audits(&teller).await, 0, "no audit row after a refusal");
    assert_eq!(
        db.version(&teller).await,
        before,
        "the version must not move"
    );

    db.finish().await;
}

#[actix_web::test]
async fn no_internal_token_means_identity_unavailable() {
    let db = TestDb::start().await;
    let identity = FakeIdentity::start();
    identity_ok(&identity, TELLER_SUBJECT);
    let teller = a_teller(&db).await;
    let before = db.version(&teller).await;

    let ans = reset(&db, &env_for(&identity, false), ORG_ADMIN, &teller, None).await;
    assert_eq!(ans.status, StatusCode::SERVICE_UNAVAILABLE, "{}", ans.text);
    assert_eq!(
        ans.body["error"]["code"],
        json!("identityUnavailable"),
        "{}",
        ans.text
    );
    assert!(identity.received(&identity_path(TELLER_SUBJECT)).is_empty());
    assert_eq!(db.audits(&teller).await, 0);
    assert_eq!(db.version(&teller).await, before);

    db.finish().await;
}
