//! RBAC C1a up-front tests (design `docs/design/blueprint/user-management-rbac.md`, PR 3083, §3.1,
//! §3.2, §3.4; owner decisions Q3 A and Q4 A): `userGrant`, `userRevoke` and `userDisable` decide
//! through `GrantGuard`, refuse a grant to a SUSPENDED user, read an unknown status as an error, and
//! write a `user_audit` row for every change.
//! Written before the implementation by the ticket author (claude/83f682). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Harness copied from `v3_org_admin_test.rs`; the `x-test-as` caller may hold several roles
//! (`EXECUTIVE@*+ADMIN@*`, the owner of B3) and may name its user id after `#`.
//! A test that needs a `SUSPENDED` row widens `users_status_check` in its own schema the way
//! migration M (§4) will, so these tests do not wait for M.
//! Needs `TEST_DATABASE_URL`; nothing skips.

#![allow(dead_code)]

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
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const ADMIN: &str = "ADMIN@*";
const REFUSED: [&str; 4] = [
    "EXECUTIVE@*",
    "BRANCH_MANAGER@XC-B01",
    "TELLER@XC-B01",
    "FINANCE@XC-B01",
];

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

/// `ROLE@branch[+ROLE@branch…][#user-id]`: the roles the caller holds and, after `#`, its user id
/// (a real `users.id` when a test needs the caller to be the target).
fn user_id(spec: &str) -> String {
    match spec.split_once('#') {
        Some((_, id)) => id.to_owned(),
        None => format!("user-{}", spec.replace(['@', '*', '+'], "-")),
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
    Some(User::new(
        &user_id(spec),
        xchange::presentation::pages::SEEDED_XC_ORG_ID,
        roles,
        None,
    ))
}

/// The legacy actor the seam still carries beside the user (`view.rs:359-371`); its role is not
/// what decides here.
fn legacy_of(spec: &str) -> Sessions {
    let first = spec.split(['+', '#']).next().unwrap_or("");
    match first.split_once('@') {
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
    let user = db
        .text(&format!(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT organization_id, 'Somchai', 'ACTIVE', 'test' FROM branches WHERE id = '{B01}' \
             RETURNING id::text"
        ))
        .await
        .expect("a user of XC-B01's organisation");
    db.count(&format!(
        "WITH r AS (INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
         SELECT '{user}'::uuid, 'TELLER', organization_id, id FROM branches WHERE id = '{B01}' \
         RETURNING 1) SELECT count(*) FROM r"
    ))
    .await;
    user
}

// ------------------------------------------------------------ RBAC C1a

const ORG_ADMIN: &str = "ADMIN@*";
const OWNER: &str = "EXECUTIVE@*+ADMIN@*";

/// A user of XC-B01's organisation with `status` holding each `(role, branch)` (`None` =
/// organisation-wide); answers the user id. Names are unique per call.
async fn a_user(db: &TestDb, status: &str, roles: &[(&str, Option<&str>)]) -> String {
    let name = unique("u");
    let user = db
        .text(&format!(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT organization_id, '{name}', '{status}', 'test' FROM branches WHERE id = '{B01}' \
             RETURNING id::text"
        ))
        .await
        .expect("a user of XC-B01's organisation");
    for (role, branch) in roles {
        let branch_sql = branch.map_or("NULL".to_owned(), |b| format!("'{b}'"));
        db.count(&format!(
            "WITH r AS (INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
             SELECT '{user}'::uuid, '{role}', organization_id, {branch_sql} FROM branches WHERE id = '{B01}' \
             RETURNING 1) SELECT count(*) FROM r"
        ))
        .await;
    }
    user
}

/// The status CHECK as migration M (§4) leaves it, so a test can hold a `SUSPENDED` row.
async fn allow_suspended(db: &TestDb) {
    sqlx::query("ALTER TABLE users DROP CONSTRAINT IF EXISTS users_status_check")
        .execute(&db.pool)
        .await
        .expect("drop the status check");
    sqlx::query(
        "ALTER TABLE users ADD CONSTRAINT users_status_check \
         CHECK (status IN ('PENDING', 'ACTIVE', 'SUSPENDED', 'DISABLED'))",
    )
    .execute(&db.pool)
    .await
    .expect("the widened status check");
}

async fn status_of(db: &TestDb, user: &str) -> String {
    db.text(&format!(
        "SELECT status FROM users WHERE id = '{user}'::uuid"
    ))
    .await
    .expect("the user row")
}

async fn live_roles(db: &TestDb, user: &str, role: &str) -> i64 {
    db.count(&format!(
        "SELECT count(*) FROM user_roles WHERE user_id = '{user}'::uuid AND role_code = '{role}' AND valid_to IS NULL"
    ))
    .await
}

async fn grant(db: &TestDb, who: &str, user: &str, role: &str, branch: Option<&str>) -> Answer {
    post(
        db,
        who,
        Method::POST,
        "/api/users/grant",
        json!({ "userId": user, "role": role, "branch": branch }),
        Key::Fresh,
    )
    .await
}

async fn revoke(db: &TestDb, who: &str, user: &str, role: &str, branch: Option<&str>) -> Answer {
    post(
        db,
        who,
        Method::POST,
        "/api/users/revoke",
        json!({ "userId": user, "role": role, "branch": branch }),
        Key::Fresh,
    )
    .await
}

async fn disable(db: &TestDb, who: &str, user: &str) -> Answer {
    post(
        db,
        who,
        Method::POST,
        "/api/users/disable",
        json!({ "userId": user }),
        Key::Fresh,
    )
    .await
}

fn refused(a: &Answer, status: StatusCode, want: &str, what: &str) {
    assert_eq!(a.status, status, "{what}: {}", a.text);
    assert_eq!(code(a), want, "{what}: {}", a.text);
}

// ---- rule 4 (Q4 A): only EXECUTIVE + ADMIN grants ADMIN or EXECUTIVE

#[actix_web::test]
async fn an_org_wide_admin_cannot_grant_admin() {
    let db = TestDb::start().await;
    let user = a_user(&db, "ACTIVE", &[("TELLER", Some(B01))]).await;
    let a = grant(&db, ORG_ADMIN, &user, "ADMIN", Some(B01)).await;
    refused(
        &a,
        StatusCode::FORBIDDEN,
        "roleNotAllowed",
        "ADMIN grants ADMIN@B01",
    );
    assert_eq!(live_roles(&db, &user, "ADMIN").await, 0, "no ADMIN row");
    assert_eq!(db.version(&user).await, 1, "unchanged");
    db.finish().await;
}

#[actix_web::test]
async fn an_org_wide_admin_cannot_grant_executive() {
    let db = TestDb::start().await;
    let user = a_user(&db, "ACTIVE", &[("TELLER", Some(B01))]).await;
    let a = grant(&db, ORG_ADMIN, &user, "EXECUTIVE", None).await;
    refused(
        &a,
        StatusCode::FORBIDDEN,
        "roleNotAllowed",
        "ADMIN grants EXECUTIVE",
    );
    assert_eq!(live_roles(&db, &user, "EXECUTIVE").await, 0);
    assert_eq!(db.version(&user).await, 1);
    db.finish().await;
}

#[actix_web::test]
async fn the_owner_grants_admin_and_executive() {
    let db = TestDb::start().await;
    let user = a_user(&db, "ACTIVE", &[("TELLER", Some(B01))]).await;
    let a = grant(&db, OWNER, &user, "ADMIN", None).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let e = grant(&db, OWNER, &user, "EXECUTIVE", None).await;
    assert_eq!(e.status, StatusCode::OK, "{}", e.text);
    assert_eq!(live_roles(&db, &user, "ADMIN").await, 1);
    assert_eq!(live_roles(&db, &user, "EXECUTIVE").await, 1);
    db.finish().await;
}

/// B3: the owner is EXECUTIVE + ADMIN organisation-wide; a branch ADMIN beside EXECUTIVE is not.
#[actix_web::test]
async fn executive_with_only_a_branch_admin_cannot_grant_admin() {
    let db = TestDb::start().await;
    let user = a_user(&db, "ACTIVE", &[("TELLER", Some(B01))]).await;
    let a = grant(&db, "EXECUTIVE@*+ADMIN@XC-B01", &user, "ADMIN", Some(B01)).await;
    refused(
        &a,
        StatusCode::FORBIDDEN,
        "roleNotAllowed",
        "EXECUTIVE + ADMIN@B01 grants ADMIN@B01",
    );
    assert_eq!(live_roles(&db, &user, "ADMIN").await, 0);
    db.finish().await;
}

// ---- rule 3 (Q4 A): an ADMIN or EXECUTIVE target is the owner's alone

#[actix_web::test]
async fn an_org_wide_admin_cannot_disable_another_admin() {
    let db = TestDb::start().await;
    let other = a_user(&db, "ACTIVE", &[("ADMIN", Some(B01))]).await;
    let a = disable(&db, ORG_ADMIN, &other).await;
    refused(
        &a,
        StatusCode::FORBIDDEN,
        "roleNotAllowed",
        "ADMIN disables ADMIN@B01",
    );
    assert_eq!(status_of(&db, &other).await, "ACTIVE");
    assert_eq!(db.version(&other).await, 1);
    db.finish().await;
}

#[actix_web::test]
async fn an_org_wide_admin_cannot_disable_an_executive() {
    let db = TestDb::start().await;
    let exec = a_user(&db, "ACTIVE", &[("EXECUTIVE", None)]).await;
    let a = disable(&db, ORG_ADMIN, &exec).await;
    refused(
        &a,
        StatusCode::FORBIDDEN,
        "roleNotAllowed",
        "ADMIN disables EXECUTIVE",
    );
    assert_eq!(status_of(&db, &exec).await, "ACTIVE");
    db.finish().await;
}

#[actix_web::test]
async fn an_org_wide_admin_cannot_change_the_roles_of_an_executive() {
    let db = TestDb::start().await;
    let exec = a_user(
        &db,
        "ACTIVE",
        &[("EXECUTIVE", None), ("FINANCE", Some(B01))],
    )
    .await;
    let r = revoke(&db, ORG_ADMIN, &exec, "FINANCE", Some(B01)).await;
    refused(
        &r,
        StatusCode::FORBIDDEN,
        "roleNotAllowed",
        "ADMIN revokes from an EXECUTIVE",
    );
    assert_eq!(
        live_roles(&db, &exec, "FINANCE").await,
        1,
        "the row stays live"
    );
    let g = grant(&db, ORG_ADMIN, &exec, "RATE_MANAGER", Some(B01)).await;
    refused(
        &g,
        StatusCode::FORBIDDEN,
        "roleNotAllowed",
        "ADMIN grants to an EXECUTIVE",
    );
    assert_eq!(db.version(&exec).await, 1);
    db.finish().await;
}

#[actix_web::test]
async fn the_owner_disables_an_executive_who_is_not_an_owner() {
    let db = TestDb::start().await;
    let exec = a_user(&db, "ACTIVE", &[("EXECUTIVE", None)]).await;
    let a = disable(&db, OWNER, &exec).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(status_of(&db, &exec).await, "DISABLED");
    db.finish().await;
}

// ---- rule 2 (Q4 A): nobody changes their own account

#[actix_web::test]
async fn an_admin_cannot_disable_itself() {
    let db = TestDb::start().await;
    let me = a_user(&db, "ACTIVE", &[("ADMIN", None)]).await;
    let a = disable(&db, &format!("ADMIN@*#{me}"), &me).await;
    refused(
        &a,
        StatusCode::FORBIDDEN,
        "selfChange",
        "ADMIN disables itself",
    );
    assert_eq!(status_of(&db, &me).await, "ACTIVE");
    db.finish().await;
}

#[actix_web::test]
async fn an_admin_cannot_grant_itself_a_role() {
    let db = TestDb::start().await;
    let me = a_user(&db, "ACTIVE", &[("ADMIN", None)]).await;
    let a = grant(&db, &format!("ADMIN@*#{me}"), &me, "FINANCE", Some(B01)).await;
    refused(
        &a,
        StatusCode::FORBIDDEN,
        "selfChange",
        "ADMIN grants itself FINANCE",
    );
    assert_eq!(live_roles(&db, &me, "FINANCE").await, 0);
    assert_eq!(db.version(&me).await, 1);
    db.finish().await;
}

#[actix_web::test]
async fn the_owner_cannot_revoke_its_own_role() {
    let db = TestDb::start().await;
    let me = a_user(
        &db,
        "ACTIVE",
        &[("EXECUTIVE", None), ("ADMIN", None), ("FINANCE", Some(B01))],
    )
    .await;
    let _second_owner = a_user(&db, "ACTIVE", &[("EXECUTIVE", None), ("ADMIN", None)]).await;
    let a = revoke(&db, &format!("{OWNER}#{me}"), &me, "FINANCE", Some(B01)).await;
    refused(
        &a,
        StatusCode::FORBIDDEN,
        "selfChange",
        "the owner revokes its own FINANCE",
    );
    assert_eq!(live_roles(&db, &me, "FINANCE").await, 1);
    db.finish().await;
}

// ---- rule 6: the last owner stays

#[actix_web::test]
async fn the_last_active_owner_cannot_be_disabled() {
    let db = TestDb::start().await;
    let only = a_user(&db, "ACTIVE", &[("EXECUTIVE", None), ("ADMIN", None)]).await;
    let a = disable(&db, OWNER, &only).await;
    refused(
        &a,
        StatusCode::CONFLICT,
        "lastOwner",
        "disable the only owner",
    );
    assert_eq!(status_of(&db, &only).await, "ACTIVE");
    assert_eq!(db.version(&only).await, 1);
    db.finish().await;
}

#[actix_web::test]
async fn of_two_owners_one_may_go_and_the_second_stays() {
    let db = TestDb::start().await;
    let first = a_user(&db, "ACTIVE", &[("EXECUTIVE", None), ("ADMIN", None)]).await;
    let second = a_user(&db, "ACTIVE", &[("EXECUTIVE", None), ("ADMIN", None)]).await;
    let a = disable(&db, OWNER, &first).await;
    assert_eq!(a.status, StatusCode::OK, "the first of two: {}", a.text);
    let b = disable(&db, OWNER, &second).await;
    refused(
        &b,
        StatusCode::CONFLICT,
        "lastOwner",
        "the second, now the last",
    );
    assert_eq!(status_of(&db, &second).await, "ACTIVE");
    db.finish().await;
}

#[actix_web::test]
async fn the_last_owner_keeps_executive_and_org_wide_admin() {
    let db = TestDb::start().await;
    let only = a_user(&db, "ACTIVE", &[("EXECUTIVE", None), ("ADMIN", None)]).await;
    let e = revoke(&db, OWNER, &only, "EXECUTIVE", None).await;
    refused(
        &e,
        StatusCode::CONFLICT,
        "lastOwner",
        "revoke EXECUTIVE from the only owner",
    );
    let a = revoke(&db, OWNER, &only, "ADMIN", None).await;
    refused(
        &a,
        StatusCode::CONFLICT,
        "lastOwner",
        "revoke ADMIN@* from the only owner",
    );
    assert_eq!(live_roles(&db, &only, "EXECUTIVE").await, 1);
    assert_eq!(live_roles(&db, &only, "ADMIN").await, 1);
    assert_eq!(db.version(&only).await, 1);
    db.finish().await;
}

/// A DISABLED owner is not an active owner: one ACTIVE owner left is still the last.
#[actix_web::test]
async fn a_disabled_owner_does_not_count_as_an_owner_left() {
    let db = TestDb::start().await;
    let only = a_user(&db, "ACTIVE", &[("EXECUTIVE", None), ("ADMIN", None)]).await;
    let _gone = a_user(&db, "DISABLED", &[("EXECUTIVE", None), ("ADMIN", None)]).await;
    let a = disable(&db, OWNER, &only).await;
    refused(
        &a,
        StatusCode::CONFLICT,
        "lastOwner",
        "the only ACTIVE owner",
    );
    db.finish().await;
}

// ---- §3.2: a SUSPENDED user gets no role; an unknown status is an error

#[actix_web::test]
async fn a_grant_to_a_suspended_user_is_refused() {
    let db = TestDb::start().await;
    allow_suspended(&db).await;
    let user = a_user(&db, "SUSPENDED", &[("TELLER", Some(B01))]).await;
    let a = grant(&db, ORG_ADMIN, &user, "FINANCE", Some(B01)).await;
    refused(
        &a,
        StatusCode::CONFLICT,
        "userNotActive",
        "grant to a SUSPENDED user",
    );
    assert_eq!(live_roles(&db, &user, "FINANCE").await, 0);
    assert_eq!(status_of(&db, &user).await, "SUSPENDED");
    assert_eq!(db.version(&user).await, 1);
    db.finish().await;
}

#[actix_web::test]
async fn a_suspended_user_may_be_disabled() {
    let db = TestDb::start().await;
    allow_suspended(&db).await;
    let user = a_user(&db, "SUSPENDED", &[("TELLER", Some(B01))]).await;
    let a = disable(&db, ORG_ADMIN, &user).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        a.body,
        json!({ "userId": user, "status": "DISABLED", "version": 2 })
    );
    db.finish().await;
}

#[actix_web::test]
async fn an_unknown_status_is_a_server_error_and_changes_nothing() {
    let db = TestDb::start().await;
    sqlx::query("ALTER TABLE users DROP CONSTRAINT IF EXISTS users_status_check")
        .execute(&db.pool)
        .await
        .expect("drop the status check");
    let user = a_user(&db, "ON_LEAVE", &[("TELLER", Some(B01))]).await;
    let a = disable(&db, ORG_ADMIN, &user).await;
    refused(
        &a,
        StatusCode::INTERNAL_SERVER_ERROR,
        "serverError",
        "disable an ON_LEAVE row",
    );
    let g = grant(&db, ORG_ADMIN, &user, "FINANCE", Some(B01)).await;
    refused(
        &g,
        StatusCode::INTERNAL_SERVER_ERROR,
        "serverError",
        "grant to an ON_LEAVE row",
    );
    assert_eq!(
        status_of(&db, &user).await,
        "ON_LEAVE",
        "not read as PENDING and moved"
    );
    assert_eq!(db.version(&user).await, 1);
    db.finish().await;
}

// ---- §3.4: every change writes one user_audit row, in the same transaction

async fn audit_rows(db: &TestDb, user: &str, action: &str, actor: &str) -> i64 {
    db.count(&format!(
        "SELECT count(*) FROM user_audit a JOIN users u ON u.id::text = a.subject_id \
         WHERE a.subject_id = '{user}' AND a.action = '{action}' AND a.actor_id = '{actor}' \
         AND a.organization_id = u.organization_id"
    ))
    .await
}

#[actix_web::test]
async fn grant_revoke_and_disable_each_write_one_audit_row() {
    let db = TestDb::start().await;
    let user = a_user(&db, "ACTIVE", &[("TELLER", Some(B01))]).await;
    let actor = user_id(ORG_ADMIN);
    let g = grant(&db, ORG_ADMIN, &user, "FINANCE", Some(B01)).await;
    assert_eq!(g.status, StatusCode::OK, "{}", g.text);
    assert_eq!(
        audit_rows(&db, &user, "userGrant", &actor).await,
        1,
        "userGrant"
    );
    let r = revoke(&db, ORG_ADMIN, &user, "FINANCE", Some(B01)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(
        audit_rows(&db, &user, "userRevoke", &actor).await,
        1,
        "userRevoke"
    );
    let d = disable(&db, ORG_ADMIN, &user).await;
    assert_eq!(d.status, StatusCode::OK, "{}", d.text);
    assert_eq!(
        audit_rows(&db, &user, "userDisable", &actor).await,
        1,
        "userDisable"
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM user_audit WHERE subject_id = '{user}'"
        ))
        .await,
        3,
        "nothing else"
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_refused_change_writes_no_audit_row() {
    let db = TestDb::start().await;
    let other = a_user(&db, "ACTIVE", &[("ADMIN", Some(B01))]).await;
    let a = disable(&db, ORG_ADMIN, &other).await;
    assert_ne!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM user_audit WHERE subject_id = '{other}'"
        ))
        .await,
        0
    );
    db.finish().await;
}
