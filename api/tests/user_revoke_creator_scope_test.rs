//! Security (#2393): `POST /api/users/revoke` (`userRevoke`) checks `creator_may_grant` against
//! the caller's `Actor.roles` before revoking a role, preventing a branch-scoped `ADMIN` from
//! revoking an organisation-wide role (`branch: null`).
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
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const ORG_ADMIN: &str = "ADMIN@*";
const BRANCH_ADMIN_B01: &str = "ADMIN@XC-B01";

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
        let schema = unique("v3revokescope");
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

    async fn live_role_count(&self, user: &str, role: &str, branch: Option<&str>) -> i64 {
        let branch_filter = match branch {
            Some(b) => format!("branch_id = '{b}'"),
            None => "branch_id IS NULL".to_owned(),
        };
        self.count(&format!(
            "SELECT count(*) FROM user_roles \
             WHERE user_id = '{user}'::uuid AND role_code = '{role}' AND {branch_filter} AND valid_to IS NULL"
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
            let s = legacy_of(&spec);
            let user = user_of(&spec);
            Callers {
                owner: s.owner,
                staff: s.staff,
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

async fn post(db: &TestDb, who: &str, uri: &str, body: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// Seeds an ACTIVE user in XC-B01's organisation holding `(role, branch)`.
async fn seed_user_with_role(db: &TestDb, role: &str, branch: Option<&str>) -> String {
    let user = db
        .text(&format!(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT organization_id, 'Somchai', 'ACTIVE', 'test' FROM branches WHERE id = '{B01}' \
             RETURNING id::text"
        ))
        .await
        .expect("a user of XC-B01's organisation");
    // RBAC C1a (owner decision Q4 A): an ADMIN no longer grants ADMIN or EXECUTIVE over HTTP, so
    // the role row is seeded directly, as the other user tests seed theirs.
    let branch_sql = branch.map_or("NULL".to_owned(), |b| format!("'{b}'"));
    db.text(&format!(
        "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
         SELECT '{user}'::uuid, '{role}', organization_id, {branch_sql} FROM branches WHERE id = '{B01}' \
         RETURNING user_id::text"
    ))
    .await
    .expect("seed the role row");
    assert_eq!(db.live_role_count(&user, role, branch).await, 1);
    user
}

// ------------------------------------------------------------ §9 behaviour table

/// R1: `ADMIN@XC-B01` revoking `EXECUTIVE` (`branch: null`) on a user holding it org-wide is
/// refused with `403` `outOfScope`; the `EXECUTIVE` row remains live and `users.version` is unchanged.
#[actix_web::test]
async fn r1_branch_admin_cannot_revoke_org_wide_executive() {
    let db = TestDb::start().await;
    let user = seed_user_with_role(&db, "EXECUTIVE", None).await;
    let version_before = db.version(&user).await;

    let a = post(
        &db,
        BRANCH_ADMIN_B01,
        "/api/users/revoke",
        json!({ "userId": user, "role": "EXECUTIVE", "branch": Value::Null }),
    )
    .await;

    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(code(&a), "outOfScope", "{}", a.text);
    assert_eq!(
        db.live_role_count(&user, "EXECUTIVE", None).await,
        1,
        "EXECUTIVE row must still be live (valid_to IS NULL)"
    );
    assert_eq!(
        db.version(&user).await,
        version_before,
        "users.version must be unchanged"
    );
    db.finish().await;
}

/// R2: `ADMIN@XC-B01` revoking `ADMIN` (`branch: null`) on a user holding it org-wide is
/// refused with `403` `outOfScope`; the `ADMIN` row remains live and `users.version` is unchanged.
#[actix_web::test]
async fn r2_branch_admin_cannot_revoke_org_wide_admin() {
    let db = TestDb::start().await;
    let user = seed_user_with_role(&db, "ADMIN", None).await;
    let version_before = db.version(&user).await;

    let a = post(
        &db,
        BRANCH_ADMIN_B01,
        "/api/users/revoke",
        json!({ "userId": user, "role": "ADMIN", "branch": Value::Null }),
    )
    .await;

    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(code(&a), "outOfScope", "{}", a.text);
    assert_eq!(
        db.live_role_count(&user, "ADMIN", None).await,
        1,
        "ADMIN row must still be live (valid_to IS NULL)"
    );
    assert_eq!(
        db.version(&user).await,
        version_before,
        "users.version must be unchanged"
    );
    db.finish().await;
}

/// R3: `ADMIN@XC-B01` revoking `FINANCE` (`branch: "XC-B01"`) succeeds (`200`) and ends the row.
#[actix_web::test]
async fn r3_branch_admin_can_revoke_role_at_own_branch() {
    let db = TestDb::start().await;
    let user = seed_user_with_role(&db, "FINANCE", Some(B01)).await;
    let version_before = db.version(&user).await;

    let a = post(
        &db,
        BRANCH_ADMIN_B01,
        "/api/users/revoke",
        json!({ "userId": user, "role": "FINANCE", "branch": B01 }),
    )
    .await;

    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        db.live_role_count(&user, "FINANCE", Some(B01)).await,
        0,
        "FINANCE@XC-B01 row must be ended"
    );
    assert_eq!(db.version(&user).await, version_before + 1);
    db.finish().await;
}

/// R4, changed by RBAC C1a (owner decision Q4 A, PR 3083 §2 and §3.1 rule 3): `ADMIN@*` revoking
/// `EXECUTIVE` from an EXECUTIVE is refused (`403` `roleNotAllowed`); only EXECUTIVE + ADMIN (the
/// owner, B3) changes an EXECUTIVE. The owner's case is in `rbac_c1a_user_commands_test.rs`.
#[actix_web::test]
async fn r4_org_admin_alone_cannot_revoke_org_wide_executive() {
    let db = TestDb::start().await;
    let user = seed_user_with_role(&db, "EXECUTIVE", None).await;
    let version_before = db.version(&user).await;

    let a = post(
        &db,
        ORG_ADMIN,
        "/api/users/revoke",
        json!({ "userId": user, "role": "EXECUTIVE", "branch": Value::Null }),
    )
    .await;

    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(code(&a), "roleNotAllowed", "{}", a.text);
    assert_eq!(
        db.live_role_count(&user, "EXECUTIVE", None).await,
        1,
        "EXECUTIVE row must still be live"
    );
    assert_eq!(db.version(&user).await, version_before);
    db.finish().await;
}

/// R5: `ADMIN@XC-B01` revoking `FINANCE` (`branch: "XC-B09"`) is refused with `403` `outOfScope`.
#[actix_web::test]
async fn r5_branch_admin_cannot_revoke_role_at_another_branch() {
    let db = TestDb::start().await;
    let user = seed_user_with_role(&db, "FINANCE", Some(B09)).await;
    let version_before = db.version(&user).await;

    let a = post(
        &db,
        BRANCH_ADMIN_B01,
        "/api/users/revoke",
        json!({ "userId": user, "role": "FINANCE", "branch": B09 }),
    )
    .await;

    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(code(&a), "outOfScope", "{}", a.text);
    assert_eq!(
        db.live_role_count(&user, "FINANCE", Some(B09)).await,
        1,
        "FINANCE@XC-B09 row must still be live"
    );
    assert_eq!(
        db.version(&user).await,
        version_before,
        "users.version must be unchanged"
    );
    db.finish().await;
}
