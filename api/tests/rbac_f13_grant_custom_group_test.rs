//! Up-front tests for RBAC §11 F13 (design `docs/design/blueprint/user-management-rbac.md` §11.1, §11.7):
//! `userGrant` and `userRevoke` (`api/src/presentation/http/v3/commands/user_grant.rs`, `user_revoke.rs`) accept a
//! group code, and every rule kept by role judges a custom group by its base.
//! Written before the implementation by the sub-controller claude/65357afb on `feat/rbac-groups`.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! What they pin:
//! - `role` may name a custom group: the `user_roles` row is written under the group's own code;
//! - `GrantGuard`, `ScopeRule` and `creator_may_grant` judge that row by the group's base (a TELLER-based group
//!   needs a branch and takes one branch only; an ADMIN-based group is the owner's alone to grant, and its holder is
//!   the owner's alone to change);
//! - an unknown code is `400 validationFailed` on `role`, as an unknown role is today;
//! - `userRevoke` of a custom group ends that row and no other;
//! - a built-in code behaves exactly as today.
//!
//! Harness copied from `rbac_c1a_user_commands_test.rs`: the `x-test-as` caller is `ROLE@branch` (`@*` =
//! organisation-wide), several joined by `+`. Each test stages its own schema, runs every migration (M2 included),
//! then does what startup does (§11.5 item 2): `seed_missing`, then installs the loaded snapshot. Custom groups go in
//! through `PgRoleGrantStore::add_group` (C6), which bumps `role_grant_version`; the test then installs the reloaded
//! snapshot itself, as `read_session_in` would on the next request (the `x-test-as` harness bypasses
//! `read_session_in`). `RoleGrants::install` is process-global, so the tests of this file run one at a time
//! (`SERIAL`) and each re-installs from its own schema before it starts.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing skips.

#![allow(dead_code)]

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Group, Role, RoleGrantStore, RoleGrants, ScopedRole};
use xc_auth::infrastructure::access::PgRoleGrantStore;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SEEDED_XC_ORG_ID, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";

const ORG_ADMIN: &str = "ADMIN@*";
const OWNER: &str = "EXECUTIVE@*+ADMIN@*";

const NIGHT_TELLER: &str = "NIGHT_TELLER";
const NIGHT_ADMIN: &str = "NIGHT_ADMIN";

/// `RoleGrants::install` is process-global: one test of this file at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
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
        let schema = unique("rbacf13");
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
        let db = Self { pool, schema };
        db.boot().await;
        db
    }

    /// What startup does (§11.5 item 2): seed every empty built-in group, then install the stored snapshot.
    async fn boot(&self) {
        let mut tx = self.pool.begin().await.expect("begin the seed");
        PgRoleGrantStore::new()
            .seed_missing(&mut tx)
            .await
            .expect("seed the built-in groups");
        tx.commit().await.expect("commit the seed");
        self.reload().await;
    }

    /// Installs the snapshot this schema holds, as `read_session_in` does when `role_grant_version` moved.
    async fn reload(&self) {
        let mut conn = self.pool.acquire().await.expect("a connection");
        let snapshot = PgRoleGrantStore::new()
            .load(&mut conn)
            .await
            .expect("load the groups");
        RoleGrants::install(snapshot);
    }

    /// A custom group on `base` holding `base`'s seed, added through the store; then the snapshot is reloaded.
    async fn custom_group(&self, code: &str, base: Role) {
        let before = self.grant_version().await;
        let group = Group {
            code: code.to_owned(),
            name: code.to_owned(),
            base,
            built_in: false,
        };
        let mut tx = self.pool.begin().await.expect("begin");
        PgRoleGrantStore::new()
            .add_group(&mut tx, &group, &RoleGrants::seed(base))
            .await
            .expect("add the group");
        tx.commit().await.expect("commit the group");
        assert!(
            self.grant_version().await > before,
            "adding a group bumps role_grant_version"
        );
        self.reload().await;
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

    async fn grant_version(&self) -> i64 {
        self.count("SELECT version::bigint FROM role_grant_version")
            .await
    }

    async fn version(&self, user: &str) -> i64 {
        self.count(&format!(
            "SELECT version::bigint FROM users WHERE id = '{user}'::uuid"
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

struct HeaderUsers;

fn user_id(spec: &str) -> String {
    match spec.split_once('#') {
        Some((_, id)) => id.to_owned(),
        None => format!("user-{}", spec.replace(['@', '*', '+', ':'], "-")),
    }
}

/// `ROLE@branch`, or `GROUP:BASE@branch` for a custom group.
fn scoped(one: &str) -> ScopedRole {
    let (group, branch) = one
        .split_once('@')
        .unwrap_or_else(|| panic!("`ROLE@branch`: {one}"));
    let (code, base) = group.split_once(':').unwrap_or((group, group));
    let base = Role::parse(base).unwrap_or_else(|| panic!("unknown role {base}"));
    let branch = (branch != "*").then(|| BranchId::new(branch));
    ScopedRole::in_group(code, base, branch, DateTime::<Utc>::UNIX_EPOCH, None)
}

fn user_of(spec: &str) -> Option<User> {
    let roles_part = spec.split_once('#').map_or(spec, |(r, _)| r);
    if roles_part.is_empty() {
        return None;
    }
    let roles = roles_part.split('+').map(scoped).collect();
    Some(User::new(&user_id(spec), SEEDED_XC_ORG_ID, roles, None))
}

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
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

async fn grant(db: &TestDb, who: &str, user: &str, role: &str, branch: Option<&str>) -> Answer {
    post(
        db,
        who,
        "/api/users/grant",
        json!({ "userId": user, "role": role, "branch": branch }),
    )
    .await
}

async fn revoke(db: &TestDb, who: &str, user: &str, role: &str, branch: Option<&str>) -> Answer {
    post(
        db,
        who,
        "/api/users/revoke",
        json!({ "userId": user, "role": role, "branch": branch }),
    )
    .await
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn field(a: &Answer) -> String {
    a.body["error"]["field"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn refused(a: &Answer, status: StatusCode, want: &str, what: &str) {
    assert_eq!(a.status, status, "{what}: {}", a.text);
    assert_eq!(code(a), want, "{what}: {}", a.text);
}

/// A user of XC-B01's organisation with `status` holding each `(group, branch)` (`None` = organisation-wide);
/// answers the user id.
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

/// Live `user_roles` rows of `user` under exactly the code `role`.
async fn live_roles(db: &TestDb, user: &str, role: &str) -> i64 {
    db.count(&format!(
        "SELECT count(*) FROM user_roles WHERE user_id = '{user}'::uuid AND role_code = '{role}' AND valid_to IS NULL"
    ))
    .await
}

async fn all_live_roles(db: &TestDb, user: &str) -> i64 {
    db.count(&format!(
        "SELECT count(*) FROM user_roles WHERE user_id = '{user}'::uuid AND valid_to IS NULL"
    ))
    .await
}

// ------------------------------------------------------------ grant a custom group

#[actix_web::test]
async fn a_custom_group_code_is_granted_under_its_own_code() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    db.custom_group(NIGHT_TELLER, Role::Teller).await;
    let user = a_user(&db, "ACTIVE", &[("FINANCE", Some(B01))]).await;

    let a = grant(&db, ORG_ADMIN, &user, NIGHT_TELLER, Some(B01)).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        a.body,
        json!({ "userId": user, "status": "ACTIVE", "version": 2 })
    );
    assert_eq!(
        live_roles(&db, &user, NIGHT_TELLER).await,
        1,
        "one NIGHT_TELLER row"
    );
    assert_eq!(
        live_roles(&db, &user, "TELLER").await,
        0,
        "stored under the group's code, not its base's"
    );
    assert_eq!(
        db.text(&format!(
            "SELECT branch_id FROM user_roles WHERE user_id = '{user}'::uuid AND role_code = '{NIGHT_TELLER}'"
        ))
        .await
        .as_deref(),
        Some(B01)
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM user_audit WHERE subject_id = '{user}' AND action = 'userGrant' \
             AND actor_id = '{}'",
            user_id(ORG_ADMIN)
        ))
        .await,
        1,
        "one userGrant audit row"
    );
    db.finish().await;
}

/// `ScopeRule` by base: TELLER needs a branch and takes one branch only, so its custom group does too.
#[actix_web::test]
async fn a_teller_based_group_follows_the_teller_scope_rule() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    db.custom_group(NIGHT_TELLER, Role::Teller).await;

    let user = a_user(&db, "ACTIVE", &[("FINANCE", Some(B01))]).await;
    let built_in = grant(&db, ORG_ADMIN, &user, "TELLER", None).await;
    refused(
        &built_in,
        StatusCode::BAD_REQUEST,
        "validationFailed",
        "TELLER organisation-wide",
    );
    let custom = grant(&db, ORG_ADMIN, &user, NIGHT_TELLER, None).await;
    refused(
        &custom,
        StatusCode::BAD_REQUEST,
        "validationFailed",
        "NIGHT_TELLER organisation-wide, as its base TELLER",
    );
    assert_eq!(live_roles(&db, &user, NIGHT_TELLER).await, 0);
    assert_eq!(db.version(&user).await, 1, "unchanged");

    let teller = a_user(&db, "ACTIVE", &[("TELLER", Some(B01))]).await;
    let second = grant(&db, ORG_ADMIN, &teller, NIGHT_TELLER, Some(B09)).await;
    refused(
        &second,
        StatusCode::BAD_REQUEST,
        "validationFailed",
        "a TELLER@B01 holder given NIGHT_TELLER@B09: one TELLER-based branch only",
    );
    assert_eq!(live_roles(&db, &teller, NIGHT_TELLER).await, 0);
    assert_eq!(db.version(&teller).await, 1, "unchanged");
    db.finish().await;
}

/// `GrantGuard` rule 4 by base: only the owner (EXECUTIVE + ADMIN organisation-wide) grants an ADMIN-based group.
#[actix_web::test]
async fn an_admin_based_group_is_the_owners_alone_to_grant() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    db.custom_group(NIGHT_ADMIN, Role::Admin).await;
    let user = a_user(&db, "ACTIVE", &[("FINANCE", Some(B01))]).await;

    let a = grant(&db, ORG_ADMIN, &user, NIGHT_ADMIN, Some(B01)).await;
    refused(
        &a,
        StatusCode::FORBIDDEN,
        "roleNotAllowed",
        "an ADMIN grants an ADMIN-based group",
    );
    assert_eq!(live_roles(&db, &user, NIGHT_ADMIN).await, 0);
    assert_eq!(db.version(&user).await, 1, "unchanged");

    let o = grant(&db, OWNER, &user, NIGHT_ADMIN, Some(B01)).await;
    assert_eq!(o.status, StatusCode::OK, "the owner grants it: {}", o.text);
    assert_eq!(live_roles(&db, &user, NIGHT_ADMIN).await, 1);
    db.finish().await;
}

/// `GrantGuard` rule 3 by base: the holder of an ADMIN-based group is the owner's alone to change.
#[actix_web::test]
async fn a_holder_of_an_admin_based_group_is_a_protected_target() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    db.custom_group(NIGHT_ADMIN, Role::Admin).await;
    let holder = a_user(&db, "ACTIVE", &[(NIGHT_ADMIN, Some(B01))]).await;

    let a = grant(&db, ORG_ADMIN, &holder, "FINANCE", Some(B01)).await;
    refused(
        &a,
        StatusCode::FORBIDDEN,
        "roleNotAllowed",
        "an ADMIN grants to a NIGHT_ADMIN holder",
    );
    let r = revoke(&db, ORG_ADMIN, &holder, NIGHT_ADMIN, Some(B01)).await;
    refused(
        &r,
        StatusCode::FORBIDDEN,
        "roleNotAllowed",
        "an ADMIN revokes NIGHT_ADMIN from its holder",
    );
    assert_eq!(live_roles(&db, &holder, "FINANCE").await, 0);
    assert_eq!(live_roles(&db, &holder, NIGHT_ADMIN).await, 1);
    assert_eq!(db.version(&holder).await, 1, "unchanged");
    db.finish().await;
}

/// `creator_may_grant` is by the wanted row's branch, whatever the group: a branch ADMIN grants a TELLER-based
/// group in its own branch and not in another.
#[actix_web::test]
async fn a_branch_admin_grants_a_custom_group_in_its_branch_only() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    db.custom_group(NIGHT_TELLER, Role::Teller).await;
    let user = a_user(&db, "ACTIVE", &[("FINANCE", Some(B01))]).await;

    let other = grant(&db, "ADMIN@XC-B01", &user, NIGHT_TELLER, Some(B09)).await;
    refused(
        &other,
        StatusCode::FORBIDDEN,
        "outOfScope",
        "ADMIN@B01 grants NIGHT_TELLER@B09",
    );
    assert_eq!(live_roles(&db, &user, NIGHT_TELLER).await, 0);

    let own = grant(&db, "ADMIN@XC-B01", &user, NIGHT_TELLER, Some(B01)).await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.text);
    assert_eq!(live_roles(&db, &user, NIGHT_TELLER).await, 1);
    db.finish().await;
}

// ------------------------------------------------------------ unknown codes

#[actix_web::test]
async fn an_unknown_group_code_is_validation_failed_on_role() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    db.custom_group(NIGHT_TELLER, Role::Teller).await;
    let user = a_user(&db, "ACTIVE", &[("FINANCE", Some(B01))]).await;

    for unknown in ["NO_SUCH_GROUP", "night_teller", ""] {
        let g = grant(&db, ORG_ADMIN, &user, unknown, Some(B01)).await;
        refused(
            &g,
            StatusCode::BAD_REQUEST,
            "validationFailed",
            &format!("grant {unknown:?}"),
        );
        assert_eq!(field(&g), "role", "grant {unknown:?}: {}", g.text);
        let r = revoke(&db, ORG_ADMIN, &user, unknown, Some(B01)).await;
        refused(
            &r,
            StatusCode::BAD_REQUEST,
            "validationFailed",
            &format!("revoke {unknown:?}"),
        );
        assert_eq!(field(&r), "role", "revoke {unknown:?}: {}", r.text);
    }
    assert_eq!(all_live_roles(&db, &user).await, 1, "only FINANCE");
    assert_eq!(db.version(&user).await, 1, "unchanged");
    db.finish().await;
}

// ------------------------------------------------------------ revoke a custom group

#[actix_web::test]
async fn a_custom_group_is_revoked_and_nothing_else() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    db.custom_group(NIGHT_TELLER, Role::Teller).await;
    let user = a_user(&db, "ACTIVE", &[("FINANCE", Some(B01))]).await;
    let g = grant(&db, ORG_ADMIN, &user, NIGHT_TELLER, Some(B01)).await;
    assert_eq!(g.status, StatusCode::OK, "{}", g.text);
    assert_eq!(live_roles(&db, &user, NIGHT_TELLER).await, 1);

    let r = revoke(&db, ORG_ADMIN, &user, NIGHT_TELLER, Some(B01)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(
        r.body,
        json!({ "userId": user, "status": "ACTIVE", "version": 3 })
    );
    assert_eq!(
        live_roles(&db, &user, NIGHT_TELLER).await,
        0,
        "the NIGHT_TELLER row ended"
    );
    assert_eq!(
        live_roles(&db, &user, "FINANCE").await,
        1,
        "the FINANCE row stays"
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM user_audit WHERE subject_id = '{user}' AND action = 'userRevoke'"
        ))
        .await,
        1,
        "one userRevoke audit row"
    );
    db.finish().await;
}

/// Revoking the base code does not end a custom group's row: rows are matched by code.
#[actix_web::test]
async fn revoking_the_base_code_leaves_the_custom_group() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    db.custom_group(NIGHT_TELLER, Role::Teller).await;
    let user = a_user(&db, "ACTIVE", &[(NIGHT_TELLER, Some(B01))]).await;

    let _ = revoke(&db, ORG_ADMIN, &user, "TELLER", Some(B01)).await;
    assert_eq!(
        live_roles(&db, &user, NIGHT_TELLER).await,
        1,
        "revoking TELLER does not touch NIGHT_TELLER"
    );
    db.finish().await;
}

// ------------------------------------------------------------ built-in codes, as today

#[actix_web::test]
async fn built_in_codes_work_as_before() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    db.custom_group(NIGHT_TELLER, Role::Teller).await;
    let user = a_user(&db, "ACTIVE", &[("FINANCE", Some(B01))]).await;

    let g = grant(&db, ORG_ADMIN, &user, "TELLER", Some(B01)).await;
    assert_eq!(g.status, StatusCode::OK, "{}", g.text);
    assert_eq!(
        g.body,
        json!({ "userId": user, "status": "ACTIVE", "version": 2 })
    );
    assert_eq!(live_roles(&db, &user, "TELLER").await, 1);

    let admin = grant(&db, ORG_ADMIN, &user, "ADMIN", Some(B01)).await;
    refused(
        &admin,
        StatusCode::FORBIDDEN,
        "roleNotAllowed",
        "an ADMIN grants ADMIN, as today",
    );
    assert_eq!(live_roles(&db, &user, "ADMIN").await, 0);

    let r = revoke(&db, ORG_ADMIN, &user, "TELLER", Some(B01)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(live_roles(&db, &user, "TELLER").await, 0);
    assert_eq!(live_roles(&db, &user, "FINANCE").await, 1);
    db.finish().await;
}
