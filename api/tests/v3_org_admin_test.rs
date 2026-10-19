//! OR-5a (#945) second batch — A04 and A05 over HTTP: `branchSave`, `userGrant`, `userRevoke`,
//! `userDisable`, and the `admin_branches_profile` and `admin_users` views
//! (design `docs/design/blueprint/org-roles-branches.md` §5.3, §6, §3.4: `BranchesManage` and
//! `UsersManage` are ADMIN's). The first batch, `BranchService::save_profile`, is in
//! `api/crates/xc-rate/tests/branch_profile_test.rs`.
//! Written before the implementation by the ticket author (claude/89bd61d9). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! The callers are Blueprint `access::Actor`s with exactly the role under test, through
//! `SessionReader::callers`. A policy refusal is pinned by its code, not its HTTP status.
//! Not here, intentionally (see #945): a **branch** of another organisation (the seam compares no
//! organisation until OR-7.7 #1030). There is no invite (owner, 2026-10-10): creating a user is
//! #1157's `userCreate`, so these tests seed the user they act on.
//! Needs `TEST_DATABASE_URL`; nothing skips.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
use xc_kernel::ids::BranchId;
use xc_testdb::people::{RoleRow, seed_person};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const ADMIN: &str = "ADMIN@*";
// Follows §11.2 seed: EXECUTIVE holds USER_MANAGEMENT FULL (UsersManage, BranchesManage); remaining staff are refused
const REFUSED: [&str; 3] = ["BRANCH_MANAGER@XC-B01", "TELLER@XC-B01", "FINANCE@XC-B01"];

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
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
        let schema = unique("v3orgadmin");
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

    async fn text(&self, sql: &str) -> Option<String> {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_optional(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
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

    async fn branch_version(&self, branch: &str) -> i64 {
        self.count(&format!(
            "SELECT COALESCE((SELECT version FROM branch_versions WHERE branch_id = '{branch}'), 0)"
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

// ------------------------------------------------------------ callers

/// `x-test-as: <ROLE>@<branch>` (or `@*` for organisation-wide): one Blueprint role row.
struct HeaderUsers;

fn user_id(spec: &str) -> String {
    format!("user-{}", spec.replace(['@', '*'], "-"))
}

fn user_of(spec: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = match role {
        "FINANCE" => Role::Finance,
        "ADMIN" => Role::Admin,
        "EXECUTIVE" => Role::Executive,
        "BRANCH_MANAGER" => Role::BranchManager,
        "TELLER" => Role::Teller,
        other => panic!("unknown role {other}"),
    };
    let branch = (branch != "*").then(|| BranchId::new(branch));
    Some(User::new(
        &user_id(spec),
        xchange::presentation::pages::SEEDED_XC_ORG_ID,
        vec![ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
}

/// The legacy actor the seam still carries beside the user (`view.rs:359-371`); its role is not
/// what decides here.
fn legacy_of(spec: &str) -> Sessions {
    match spec.split_once('@') {
        None => Sessions {
            owner: None,
            staff: None,
        },
        Some((_, "*")) => Sessions {
            owner: Some(Actor::owner(user_id(spec))),
            staff: None,
        },
        Some((_, branch)) => Sessions {
            owner: None,
            staff: Actor::staff(user_id(spec), OldRole::Finance, BranchId::new(branch)),
        },
    }
}

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move { legacy_of(&spec) })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            let _s = legacy_of(&spec);
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

enum Key<'a> {
    Fresh,
    This(&'a str),
}

async fn call(
    db: &TestDb,
    who: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: Key<'_>,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who));
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .set_payload(body.to_string());
        match key {
            Key::Fresh => req = req.insert_header(("idempotency-key", unique("key"))),
            Key::This(k) => req = req.insert_header(("idempotency-key", k.to_owned())),
        }
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn post(
    db: &TestDb,
    who: &str,
    method: Method,
    uri: &str,
    body: Value,
    key: Key<'_>,
) -> Answer {
    call(db, who, method, uri, Some(body), key).await
}

async fn get(db: &TestDb, who: &str, uri: &str) -> Answer {
    call(db, who, Method::GET, uri, None, Key::Fresh).await
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn profile(code: &str) -> Value {
    json!({
        "branch": B01,
        "code": code,
        "address": "1 Silom Rd",
        "timezone": "Asia/Bangkok",
        "openingHours": { "mon": [{ "open": "09:00", "close": "17:00" }] },
        "bookingEnabled": true
    })
}

/// An ACTIVE TELLER of XC-B01, seeded as #1157's `userCreate` leaves one; answers the user id.
async fn a_teller(db: &TestDb) -> String {
    let mut conn = db.pool.acquire().await.expect("acquire connection");
    seed_person(
        &mut conn,
        xchange::presentation::pages::SEEDED_XC_ORG_ID,
        "Somchai",
        &[RoleRow {
            role_code: "TELLER",
            branch_id: Some(B01),
            valid_from: DateTime::<Utc>::UNIX_EPOCH,
            valid_to: None,
        }],
    )
    .await
    .expect("a user of XC-B01's organisation")
}

// ------------------------------------------------------------ A04: the branch profile

#[actix_web::test]
async fn an_admin_saves_a_branch_profile_and_reads_it_back() {
    let db = TestDb::start().await;
    let before = db.branch_version(B01).await;
    let a = post(
        &db,
        ADMIN,
        Method::PUT,
        "/api/branches/profile",
        profile("SILOM"),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let want = json!({
        "branch": B01,
        "name": "B01",
        "code": "SILOM",
        "address": "1 Silom Rd",
        "timezone": "Asia/Bangkok",
        "openingHours": { "mon": [{ "open": "09:00", "close": "17:00" }] },
        "bookingEnabled": true
    });
    assert_eq!(a.body, want, "the saved profile");
    let r = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_branches_profile?branch={B01}"),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.body["data"], want, "the view answers what was saved");
    assert!(
        db.branch_version(B01).await > before,
        "the branch version moved"
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_bad_zone_is_refused_and_a_code_another_branch_holds_is_a_conflict() {
    let db = TestDb::start().await;
    let mut bad = profile("SILOM");
    bad["timezone"] = json!("Mars/Olympus");
    let a = post(
        &db,
        ADMIN,
        Method::PUT,
        "/api/branches/profile",
        bad,
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{}", a.text);
    assert_eq!(code(&a), "validationFailed", "{}", a.text);

    let mut b09 = profile("SILOM");
    b09["branch"] = json!(B09);
    let taken = post(
        &db,
        ADMIN,
        Method::PUT,
        "/api/branches/profile",
        b09,
        Key::Fresh,
    )
    .await;
    assert_eq!(taken.status, StatusCode::OK, "{}", taken.text);
    let c = post(
        &db,
        ADMIN,
        Method::PUT,
        "/api/branches/profile",
        profile("SILOM"),
        Key::Fresh,
    )
    .await;
    assert_eq!(c.status, StatusCode::CONFLICT, "{}", c.text);
    assert_eq!(code(&c), "conflict", "{}", c.text);
    db.finish().await;
}

// ------------------------------------------------------------ who may

#[actix_web::test]
async fn only_an_admin_may_use_any_of_them() {
    let db = TestDb::start().await;
    let user = a_teller(&db).await;
    let calls: Vec<(Method, &str, Value)> = vec![
        (Method::PUT, "/api/branches/profile", profile("SILOM")),
        (
            Method::POST,
            "/api/users/grant",
            json!({ "userId": user, "role": "FINANCE", "branch": B01 }),
        ),
        (
            Method::POST,
            "/api/users/revoke",
            json!({ "userId": user, "role": "TELLER", "branch": B01 }),
        ),
        (
            Method::POST,
            "/api/users/disable",
            json!({ "userId": user }),
        ),
    ];
    for who in REFUSED {
        for (m, uri, body) in &calls {
            let a = post(&db, who, m.clone(), uri, body.clone(), Key::Fresh).await;
            assert_ne!(a.status, StatusCode::OK, "{who} {uri}: {}", a.text);
            assert_eq!(code(&a), "roleNotAllowedHere", "{who} {uri}: {}", a.text);
        }
        for uri in [
            format!("/api/v3/views/admin_branches_profile?branch={B01}"),
            format!("/api/v3/views/admin_users?branch={B01}"),
        ] {
            let r = get(&db, who, &uri).await;
            assert_eq!(code(&r), "roleNotAllowedHere", "{who} {uri}: {}", r.text);
        }
    }
    assert_eq!(db.version(&user).await, 1, "nothing changed the user");
    assert_eq!(db.count("SELECT count(*) FROM users").await, 1);
    db.finish().await;
}

/// §11.2: EXECUTIVE holds USER_MANAGEMENT FULL (`UsersManage`, `BranchesManage`), so it may save a
/// branch profile and read both org-admin views (#3501).
#[actix_web::test]
async fn an_executive_holds_user_management_and_is_allowed() {
    let db = TestDb::start().await;
    let who = "EXECUTIVE@*";
    let a = post(
        &db,
        who,
        Method::PUT,
        "/api/branches/profile",
        profile("SILOM"),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{who}: {}", a.text);
    for uri in [
        format!("/api/v3/views/admin_branches_profile?branch={B01}"),
        format!("/api/v3/views/admin_users?branch={B01}"),
    ] {
        let r = get(&db, who, &uri).await;
        assert_eq!(r.status, StatusCode::OK, "{who} {uri}: {}", r.text);
    }
    db.finish().await;
}

// ------------------------------------------------------------ A05: the users view, grant, revoke, disable

#[actix_web::test]
async fn the_users_view_lists_a_user_with_roles_and_no_auth_subject() {
    let db = TestDb::start().await;
    let user = a_teller(&db).await;
    let r = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_users?branch={B01}"),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let rows = r.body["data"].as_array().expect("rows");
    let row = rows
        .iter()
        .find(|x| x["userId"] == json!(user))
        .expect("the user is listed");
    assert_eq!(row["name"], "Somchai");
    assert_eq!(row["status"], "ACTIVE");
    assert_eq!(row["version"], 1);
    assert_eq!(row["roles"][0]["role"], "TELLER");
    assert_eq!(row["roles"][0]["branch"], B01);
    assert!(
        !r.text.contains("authSubject") && !r.text.contains("inviteCode"),
        "no auth subject and no invite code in the view: {}",
        r.text
    );
    db.finish().await;
}

#[actix_web::test]
async fn grant_revoke_and_disable_each_move_the_users_version() {
    let db = TestDb::start().await;
    let user = a_teller(&db).await;
    assert_eq!(db.version(&user).await, 1);

    let g = post(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/grant",
        json!({ "userId": user, "role": "FINANCE", "branch": B01 }),
        Key::Fresh,
    )
    .await;
    assert_eq!(g.status, StatusCode::OK, "{}", g.text);
    assert_eq!(
        g.body,
        json!({ "userId": user, "status": "ACTIVE", "version": 2 })
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM user_roles WHERE user_id = '{user}'::uuid AND role_code = 'FINANCE' AND valid_to IS NULL"
        ))
        .await,
        1
    );

    let r = post(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/revoke",
        json!({ "userId": user, "role": "FINANCE", "branch": B01 }),
        Key::Fresh,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.body["version"], 3);
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM user_roles WHERE user_id = '{user}'::uuid AND role_code = 'FINANCE' AND valid_to IS NULL"
        ))
        .await,
        0,
        "the role row is ended"
    );

    let d = post(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/disable",
        json!({ "userId": user }),
        Key::Fresh,
    )
    .await;
    assert_eq!(d.status, StatusCode::OK, "{}", d.text);
    assert_eq!(
        d.body,
        json!({ "userId": user, "status": "DISABLED", "version": 4 })
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_grant_that_breaks_the_scope_rule_is_refused_and_changes_nothing() {
    let db = TestDb::start().await;
    let user = a_teller(&db).await;
    let a = post(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/grant",
        json!({ "userId": user, "role": "TELLER", "branch": B09 }),
        Key::Fresh,
    )
    .await;
    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "a teller of two branches: {}",
        a.text
    );
    assert_eq!(code(&a), "validationFailed", "{}", a.text);
    assert_eq!(db.version(&user).await, 1);
    db.finish().await;
}

#[actix_web::test]
async fn a_user_of_another_organisation_is_out_of_scope() {
    let db = TestDb::start().await;
    db.count(
        "WITH o AS (INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ('00000000-0000-0000-0000-0000000000b2', 'YY', NULL, 'Asia/Bangkok', now()) RETURNING 1) \
         SELECT count(*) FROM o",
    )
    .await;
    let other: String = db
        .text(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ('00000000-0000-0000-0000-0000000000b2', 'Other', 'ACTIVE', 'test') RETURNING id::text",
        )
        .await
        .expect("a user of YY");
    for (uri, body) in [
        (
            "/api/users/grant",
            json!({ "userId": other, "role": "FINANCE", "branch": B01 }),
        ),
        ("/api/users/disable", json!({ "userId": other })),
    ] {
        let a = post(&db, ADMIN, Method::POST, uri, body, Key::Fresh).await;
        assert_eq!(code(&a), "outOfScope", "{uri}: {}", a.text);
    }
    assert_eq!(db.version(&other).await, 1, "unchanged");
    db.finish().await;
}

#[actix_web::test]
async fn the_same_key_applies_once_and_another_body_conflicts() {
    let db = TestDb::start().await;
    let user = a_teller(&db).await;
    let body = json!({ "userId": user, "role": "FINANCE", "branch": B01 });
    let first = post(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/grant",
        body.clone(),
        Key::This("g-1"),
    )
    .await;
    let again = post(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/grant",
        body,
        Key::This("g-1"),
    )
    .await;
    assert_eq!(again.body, first.body, "the stored answer");
    assert_eq!(db.version(&user).await, 2, "applied once");
    let other = post(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/grant",
        json!({ "userId": user, "role": "FINANCE", "branch": B09 }),
        Key::This("g-1"),
    )
    .await;
    assert_eq!(code(&other), "idempotencyConflict", "{}", other.text);
    db.finish().await;
}

// ------------------------------------------------------------ #2390: userGrant creator scope check (G1–G7)

#[actix_web::test]
async fn g1_branch_admin_cannot_grant_org_wide_executive() {
    let db = TestDb::start().await;
    let user = a_teller(&db).await;
    let a = post(
        &db,
        "ADMIN@XC-B01",
        Method::POST,
        "/api/users/grant",
        json!({ "userId": user, "role": "EXECUTIVE", "branch": Value::Null }),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(code(&a), "outOfScope", "{}", a.text);
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM user_roles WHERE user_id = '{user}'::uuid AND role_code = 'EXECUTIVE' AND valid_to IS NULL"
        ))
        .await,
        0
    );
    assert_eq!(db.version(&user).await, 1);
    db.finish().await;
}

#[actix_web::test]
async fn g2_branch_admin_cannot_grant_org_wide_admin() {
    let db = TestDb::start().await;
    let user = a_teller(&db).await;
    let a = post(
        &db,
        "ADMIN@XC-B01",
        Method::POST,
        "/api/users/grant",
        json!({ "userId": user, "role": "ADMIN", "branch": Value::Null }),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(code(&a), "outOfScope", "{}", a.text);
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM user_roles WHERE user_id = '{user}'::uuid AND role_code = 'ADMIN' AND valid_to IS NULL"
        ))
        .await,
        0
    );
    assert_eq!(db.version(&user).await, 1);
    db.finish().await;
}

#[actix_web::test]
async fn g3_branch_admin_cannot_grant_org_wide_verification_reviewer() {
    let db = TestDb::start().await;
    let user = a_teller(&db).await;
    let a = post(
        &db,
        "ADMIN@XC-B01",
        Method::POST,
        "/api/users/grant",
        json!({ "userId": user, "role": "VERIFICATION_REVIEWER", "branch": Value::Null }),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(code(&a), "outOfScope", "{}", a.text);
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM user_roles WHERE user_id = '{user}'::uuid AND role_code = 'VERIFICATION_REVIEWER' AND valid_to IS NULL"
        ))
        .await,
        0
    );
    assert_eq!(db.version(&user).await, 1);
    db.finish().await;
}

#[actix_web::test]
async fn g4_branch_admin_grants_finance_on_own_branch() {
    let db = TestDb::start().await;
    let user = a_teller(&db).await;
    let a = post(
        &db,
        "ADMIN@XC-B01",
        Method::POST,
        "/api/users/grant",
        json!({ "userId": user, "role": "FINANCE", "branch": B01 }),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        a.body,
        json!({ "userId": user, "status": "ACTIVE", "version": 2 })
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM user_roles WHERE user_id = '{user}'::uuid AND role_code = 'FINANCE' AND branch_id = '{B01}' AND valid_to IS NULL"
        ))
        .await,
        1
    );
    db.finish().await;
}

#[actix_web::test]
async fn g5_branch_admin_cannot_grant_finance_on_other_branch() {
    let db = TestDb::start().await;
    let user = a_teller(&db).await;
    let a = post(
        &db,
        "ADMIN@XC-B01",
        Method::POST,
        "/api/users/grant",
        json!({ "userId": user, "role": "FINANCE", "branch": B09 }),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(code(&a), "outOfScope", "{}", a.text);
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM user_roles WHERE user_id = '{user}'::uuid AND role_code = 'FINANCE' AND valid_to IS NULL"
        ))
        .await,
        0
    );
    assert_eq!(db.version(&user).await, 1);
    db.finish().await;
}

/// Changed by RBAC C1a (owner decision Q4 A, PR 3083 §2 and §3.1 rule 4): an organisation-wide
/// ADMIN no longer grants EXECUTIVE; only EXECUTIVE + ADMIN (the owner, B3) does. The owner's
/// grant is in `rbac_c1a_user_commands_test.rs`.
#[actix_web::test]
async fn g6_org_wide_admin_alone_cannot_grant_org_wide_executive() {
    let db = TestDb::start().await;
    let user = a_teller(&db).await;
    let a = post(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/grant",
        json!({ "userId": user, "role": "EXECUTIVE", "branch": Value::Null }),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(code(&a), "roleNotAllowed", "{}", a.text);
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM user_roles WHERE user_id = '{user}'::uuid AND role_code = 'EXECUTIVE' AND valid_to IS NULL"
        ))
        .await,
        0
    );
    assert_eq!(db.version(&user).await, 1);
    db.finish().await;
}

#[actix_web::test]
async fn g7_org_wide_admin_grants_branch_scoped_finance() {
    let db = TestDb::start().await;
    let user = a_teller(&db).await;
    let a = post(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/grant",
        json!({ "userId": user, "role": "FINANCE", "branch": B01 }),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        a.body,
        json!({ "userId": user, "status": "ACTIVE", "version": 2 })
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM user_roles WHERE user_id = '{user}'::uuid AND role_code = 'FINANCE' AND branch_id = '{B01}' AND valid_to IS NULL"
        ))
        .await,
        1
    );
    db.finish().await;
}
