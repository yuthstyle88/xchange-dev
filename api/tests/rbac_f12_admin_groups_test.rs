//! Up-front tests for RBAC §11 F12 (design `docs/design/blueprint/user-management-rbac.md` §11.3, §11.6, §11.7):
//! `adminGroupAdd`, `adminGroupSave` and `adminGroupDelete` (`api/src/presentation/http/v3/commands/admin_groups.rs`).
//! Written before the implementation by the sub-controller claude/65357afb on `feat/rbac-groups`.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! What they pin:
//! - only a caller with a valid organisation-wide row whose base is EXECUTIVE changes groups (`400 roleNotAllowed`
//!   for anyone else who passes the seam; the seam itself refuses a caller without `UsersManage`);
//! - an add stores the requested module permissions plus the base's no-module seed; a save replaces the module set;
//! - the answer is `{code, version}` with `version` = `role_grant_version` after the change;
//! - the edited set is what `Authorizer::check` sees, in this process, right after the command answers (the
//!   §11.7 wiring mutant: `RoleGrants::of` returning the base's seed turns these red);
//! - guardrails answer `409 guardrail:<n>`; validation answers `400 validationFailed` with the field;
//! - delete refuses a built-in (`409 builtIn`), a group in use (`409 inUse`), an unknown code (`404 notFound`);
//! - every successful change writes exactly one `user_audit` row (`GROUP_ADD` / `PERMISSION_CHANGE` /
//!   `GROUP_DELETE`, subject = group code, actor = the caller, before/after).
//!
//! Every expected permission set is computed from `RoleGrants::seed`, `PermissionModule::module` and
//! `Permission::ALL`; none is typed by hand.
//!
//! Harness copied from `rbac_c1a_user_commands_test.rs`: the `x-test-as` caller is `ROLE@branch` (`@*` =
//! organisation-wide), several joined by `+`, a custom group written `GROUP:BASE@branch`. Each test stages its own
//! schema, runs every migration (M2 included), then does what startup does (§11.5 item 2): `seed_missing`, then
//! installs the loaded snapshot. `RoleGrants::install` is process-global, so the tests of this file run one at a
//! time (`SERIAL`) and each re-installs from its own schema before it starts.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing skips.

#![allow(dead_code)]

mod common;

use std::collections::BTreeSet;
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
use xc_auth::domain::access::permission_module::{Level, Module, PermissionModule};
use xc_auth::domain::access::{
    Actor as User, Authorizer, Permission, Refusal, Role, RoleGrantStore, RoleGrants, ScopedRole,
    Target,
};
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

const EXEC: &str = "EXECUTIVE@*";
const ORG_ADMIN: &str = "ADMIN@*";
const TELLER_B01: &str = "TELLER@XC-B01";

const NIGHT: &str = "NIGHT_TELLER";

/// `RoleGrants::install` is process-global: one test of this file at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

// ------------------------------------------------------------ permissions, computed

/// The serde name a permission is stored and sent by (`"ExchangeTransact"`).
fn name(p: Permission) -> String {
    serde_json::to_value(p)
        .expect("a permission serialises")
        .as_str()
        .expect("a permission serialises to a string")
        .to_owned()
}

fn names(ps: &[Permission]) -> Vec<String> {
    ps.iter().copied().map(name).collect()
}

fn name_set(ps: &[Permission]) -> BTreeSet<String> {
    ps.iter().copied().map(name).collect()
}

/// The module permissions of `role`'s seed, in `Permission::ALL` order.
fn module_part(role: Role) -> Vec<Permission> {
    RoleGrants::seed(role)
        .into_iter()
        .filter(|p| PermissionModule::module(*p).is_some())
        .collect()
}

/// The no-module permissions of `role`'s seed (§11.3 rule 5: never edited in the matrix).
fn no_module_part(role: Role) -> Vec<Permission> {
    RoleGrants::seed(role)
        .into_iter()
        .filter(|p| PermissionModule::module(*p).is_none())
        .collect()
}

/// A strict, non-empty subset of TELLER's module permissions: every second one.
fn night_set() -> Vec<Permission> {
    let all = module_part(Role::Teller);
    assert!(
        all.len() >= 2,
        "TELLER's seed must hold at least two module permissions, holds {all:?}"
    );
    let subset: Vec<Permission> = all.iter().copied().step_by(2).collect();
    assert!(!subset.is_empty() && subset.len() < all.len());
    subset
}

/// The first permission in `Permission::ALL` order whose module is `module` and whose level is above VIEW.
fn above_view_in(module: Module) -> Permission {
    Permission::ALL
        .into_iter()
        .find(|p| matches!(PermissionModule::module(*p), Some((m, l)) if m == module && l > Level::View))
        .unwrap_or_else(|| panic!("a {module:?} permission above VIEW"))
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
        let schema = unique("rbacf12");
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
        let store = PgRoleGrantStore::new();
        let mut tx = self.pool.begin().await.expect("begin the seed");
        store
            .seed_missing(&mut tx)
            .await
            .expect("seed the built-in groups");
        tx.commit().await.expect("commit the seed");
        self.reload().await;
    }

    /// Installs the snapshot this schema holds.
    async fn reload(&self) {
        let mut conn = self.pool.acquire().await.expect("a connection");
        let snapshot = PgRoleGrantStore::new()
            .load(&mut conn)
            .await
            .expect("load the groups");
        RoleGrants::install(snapshot);
    }

    async fn text(&self, sql: &str) -> Option<String> {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_optional(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn texts(&self, sql: &str) -> Vec<String> {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_all(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// `role_grant_version`.
    async fn grant_version(&self) -> i64 {
        self.count("SELECT version::bigint FROM role_grant_version")
            .await
    }

    /// The stored set of `code`, by serde name.
    async fn stored(&self, code: &str) -> BTreeSet<String> {
        self.texts(&format!(
            "SELECT permission FROM role_permission_grants WHERE role_code = '{code}'"
        ))
        .await
        .into_iter()
        .collect()
    }

    async fn group_exists(&self, code: &str) -> bool {
        self.count(&format!("SELECT count(*) FROM roles WHERE code = '{code}'"))
            .await
            == 1
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

/// `ROLE@branch[+ROLE@branch…][#user-id]`; a custom group is `GROUP:BASE@branch`.
fn user_id(spec: &str) -> String {
    match spec.split_once('#') {
        Some((_, id)) => id.to_owned(),
        None => format!("user-{}", spec.replace(['@', '*', '+', ':'], "-")),
    }
}

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

async fn add(db: &TestDb, who: &str, code: &str, base: &str, perms: &[String]) -> Answer {
    add_named(db, who, code, "Night teller", base, perms).await
}

async fn add_named(
    db: &TestDb,
    who: &str,
    code: &str,
    name: &str,
    base: &str,
    perms: &[String],
) -> Answer {
    post(
        db,
        who,
        "/api/admin/groups/add",
        json!({ "code": code, "name": name, "base": base, "permissions": perms }),
    )
    .await
}

async fn save(db: &TestDb, who: &str, code: &str, perms: &[String]) -> Answer {
    post(
        db,
        who,
        "/api/admin/groups/save",
        json!({ "code": code, "permissions": perms }),
    )
    .await
}

async fn delete(db: &TestDb, who: &str, code: &str) -> Answer {
    post(db, who, "/api/admin/groups/delete", json!({ "code": code })).await
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

fn invalid(a: &Answer, want_field: &str, what: &str) {
    refused(a, StatusCode::BAD_REQUEST, "validationFailed", what);
    assert_eq!(field(a), want_field, "{what}: {}", a.text);
}

/// A successful change: `200 {code, version}` with `version` = `role_grant_version` now, above `before`.
async fn changed(db: &TestDb, a: &Answer, want_code: &str, before: i64, what: &str) -> i64 {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    assert_eq!(a.body["code"], json!(want_code), "{what}: {}", a.text);
    let version = a.body["version"]
        .as_i64()
        .unwrap_or_else(|| panic!("{what}: a numeric version: {}", a.text));
    assert_eq!(
        version,
        db.grant_version().await,
        "{what}: the answer's version is role_grant_version"
    );
    assert!(version > before, "{what}: {version} > {before}");
    version
}

/// A user of XC-B01's organisation holding `group` at XC-B01; answers the user id.
async fn a_holder(db: &TestDb, group: &str) -> String {
    let name = unique("u");
    let user = db
        .text(&format!(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT organization_id, '{name}', 'ACTIVE', 'test' FROM branches WHERE id = '{B01}' \
             RETURNING id::text"
        ))
        .await
        .expect("a user of XC-B01's organisation");
    db.count(&format!(
        "WITH r AS (INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
         SELECT '{user}'::uuid, '{group}', organization_id, id FROM branches WHERE id = '{B01}' \
         RETURNING 1) SELECT count(*) FROM r"
    ))
    .await;
    user
}

/// The audit rows of the group changes F12 writes.
async fn group_audit_rows(db: &TestDb) -> i64 {
    db.count(
        "SELECT count(*) FROM user_audit \
         WHERE action IN ('GROUP_ADD', 'PERMISSION_CHANGE', 'GROUP_DELETE')",
    )
    .await
}

fn now() -> DateTime<Utc> {
    Utc::now()
}

fn at_b01() -> Target {
    Target::new(SEEDED_XC_ORG_ID, Some(BranchId::new(B01)), None)
}

// ------------------------------------------------------------ add

#[actix_web::test]
async fn an_executive_adds_a_group_on_a_teller_base() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let before = db.grant_version().await;
    let wanted = night_set();

    let a = add(&db, EXEC, NIGHT, "TELLER", &names(&wanted)).await;
    changed(&db, &a, NIGHT, before, "add NIGHT_TELLER").await;

    let (name, base, built_in): (String, String, bool) =
        sqlx::query_as("SELECT name, base_role, built_in FROM roles WHERE code = $1")
            .bind(NIGHT)
            .fetch_one(&db.pool)
            .await
            .expect("the added roles row");
    assert_eq!(name, "Night teller");
    assert_eq!(base, "TELLER");
    assert!(!built_in, "an added group is not built in");

    let mut want = name_set(&wanted);
    want.extend(names(&no_module_part(Role::Teller)));
    assert_eq!(
        db.stored(NIGHT).await,
        want,
        "the requested module permissions plus TELLER's no-module seed"
    );
    db.finish().await;
}

/// §11.7: an added group behaves as its own set for permissions, in this process, right after the add.
#[actix_web::test]
async fn an_added_group_authorises_by_its_own_set() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let wanted = night_set();
    let left_out: Vec<Permission> = module_part(Role::Teller)
        .into_iter()
        .filter(|p| !wanted.contains(p))
        .collect();
    assert!(!left_out.is_empty());

    let a = add(&db, EXEC, NIGHT, "TELLER", &names(&wanted)).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);

    let night = user_of("NIGHT_TELLER:TELLER@XC-B01").expect("a caller");
    for p in &wanted {
        assert!(
            Authorizer::check(Some(&night), *p, &at_b01(), now()).is_ok(),
            "NIGHT_TELLER holds {p:?}"
        );
    }
    for p in &left_out {
        assert_eq!(
            Authorizer::check(Some(&night), *p, &at_b01(), now()),
            Err(Refusal::RoleNotAllowed),
            "NIGHT_TELLER does not hold {p:?}, though its base TELLER does"
        );
    }
    db.finish().await;
}

#[actix_web::test]
async fn adding_an_existing_code_is_code_taken() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let wanted = night_set();
    let first = add(&db, EXEC, NIGHT, "TELLER", &names(&wanted)).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let version = db.grant_version().await;
    let stored = db.stored(NIGHT).await;

    let again = add(
        &db,
        EXEC,
        NIGHT,
        "TELLER",
        &names(&module_part(Role::Teller)),
    )
    .await;
    refused(
        &again,
        StatusCode::CONFLICT,
        "codeTaken",
        "add NIGHT_TELLER twice",
    );
    assert_eq!(db.grant_version().await, version, "nothing moved");
    assert_eq!(db.stored(NIGHT).await, stored, "the first set stays");
    db.finish().await;
}

#[actix_web::test]
async fn add_validates_code_name_base_and_permissions() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let before = db.grant_version().await;
    let good = names(&night_set());

    let too_long = "A".repeat(41);
    for bad in ["night_teller", "N", too_long.as_str(), "NIGHT-TELLER", ""] {
        let a = add(&db, EXEC, bad, "TELLER", &good).await;
        invalid(&a, "code", &format!("code {bad:?}"));
    }

    let a = add_named(&db, EXEC, NIGHT, "   ", "TELLER", &good).await;
    invalid(&a, "name", "a blank name");
    let long_name = "n".repeat(81);
    let a = add_named(&db, EXEC, NIGHT, &long_name, "TELLER", &good).await;
    invalid(&a, "name", "an 81-character name");

    for bad in ["EXECUTIVE", "CUSTOMER", "NOBODY", "teller"] {
        let a = add(&db, EXEC, NIGHT, bad, &good).await;
        invalid(&a, "base", &format!("base {bad:?}"));
    }

    let mut unknown = good.clone();
    unknown.push("NoSuchPermission".to_owned());
    let a = add(&db, EXEC, NIGHT, "TELLER", &unknown).await;
    invalid(&a, "permissions", "an unknown permission name");

    assert!(!db.group_exists(NIGHT).await, "nothing was added");
    assert_eq!(db.grant_version().await, before, "nothing moved");
    assert_eq!(group_audit_rows(&db).await, 0, "no audit row");
    db.finish().await;
}

// ------------------------------------------------------------ save

/// §11.7 wiring: the edited set is what `Authorizer::check` sees. The mutant `RoleGrants::of` returning the base's
/// seed keeps `removed` granted and turns this red.
#[actix_web::test]
async fn a_save_replaces_the_set_and_authorizer_sees_it() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let teller = user_of(TELLER_B01).expect("a caller");
    let all = module_part(Role::Teller);
    let removed = all[0];
    let kept: Vec<Permission> = all[1..].to_vec();
    assert!(!kept.is_empty());
    assert!(
        Authorizer::check(Some(&teller), removed, &at_b01(), now()).is_ok(),
        "TELLER holds {removed:?} before the save"
    );
    let before = db.grant_version().await;

    let a = save(&db, EXEC, "TELLER", &names(&kept)).await;
    changed(&db, &a, "TELLER", before, "save TELLER").await;

    let mut want = name_set(&kept);
    want.extend(names(&no_module_part(Role::Teller)));
    assert_eq!(
        db.stored("TELLER").await,
        want,
        "the module set is replaced; the no-module seed stays"
    );

    assert_eq!(
        Authorizer::check(Some(&teller), removed, &at_b01(), now()),
        Err(Refusal::RoleNotAllowed),
        "TELLER no longer holds {removed:?}"
    );
    for p in &kept {
        assert!(
            Authorizer::check(Some(&teller), *p, &at_b01(), now()).is_ok(),
            "TELLER still holds {p:?}"
        );
    }
    for p in no_module_part(Role::Teller) {
        assert!(
            Authorizer::check(Some(&teller), p, &at_b01(), now()).is_ok(),
            "TELLER keeps the no-module {p:?}"
        );
    }
    db.finish().await;
}

#[actix_web::test]
async fn save_validates_code_and_permissions() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let before = db.grant_version().await;
    let stored = db.stored("TELLER").await;

    let mut unknown = names(&module_part(Role::Teller));
    unknown.push("NoSuchPermission".to_owned());
    let a = save(&db, EXEC, "TELLER", &unknown).await;
    invalid(&a, "permissions", "an unknown permission name");

    let a = save(&db, EXEC, "NO_SUCH_GROUP", &names(&night_set())).await;
    refused(
        &a,
        StatusCode::NOT_FOUND,
        "notFound",
        "save an unknown group",
    );

    assert_eq!(db.stored("TELLER").await, stored, "TELLER unchanged");
    assert_eq!(db.grant_version().await, before, "nothing moved");
    assert_eq!(group_audit_rows(&db).await, 0, "no audit row");
    db.finish().await;
}

// ------------------------------------------------------------ who may

/// §11.3 (V.9 :66): only an organisation-wide EXECUTIVE base. ADMIN holds `UsersManage` and passes the seam; a
/// branch-scoped EXECUTIVE row is not organisation-wide.
#[actix_web::test]
async fn only_an_org_wide_executive_base_changes_groups() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let spare = add(&db, EXEC, "SPARE_GROUP", "TELLER", &names(&night_set())).await;
    assert_eq!(spare.status, StatusCode::OK, "{}", spare.text);
    let version = db.grant_version().await;
    let teller = db.stored("TELLER").await;
    let audits = group_audit_rows(&db).await;

    for who in [ORG_ADMIN, "EXECUTIVE@XC-B01"] {
        let a = add(&db, who, NIGHT, "TELLER", &names(&night_set())).await;
        refused(
            &a,
            StatusCode::BAD_REQUEST,
            "roleNotAllowed",
            &format!("{who} adds"),
        );
        let s = save(&db, who, "TELLER", &names(&night_set())).await;
        refused(
            &s,
            StatusCode::BAD_REQUEST,
            "roleNotAllowed",
            &format!("{who} saves"),
        );
        let d = delete(&db, who, "SPARE_GROUP").await;
        refused(
            &d,
            StatusCode::BAD_REQUEST,
            "roleNotAllowed",
            &format!("{who} deletes"),
        );
    }

    assert!(!db.group_exists(NIGHT).await, "nothing was added");
    assert!(db.group_exists("SPARE_GROUP").await, "nothing was deleted");
    assert_eq!(db.stored("TELLER").await, teller, "nothing was saved");
    assert_eq!(db.grant_version().await, version, "nothing moved");
    assert_eq!(group_audit_rows(&db).await, audits, "no audit row");
    db.finish().await;
}

/// A TELLER holds no `UsersManage`: the seam refuses before the command runs.
#[actix_web::test]
async fn a_teller_is_refused_at_the_seam() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let version = db.grant_version().await;
    let teller = db.stored("TELLER").await;

    let a = add(&db, TELLER_B01, NIGHT, "TELLER", &names(&night_set())).await;
    refused(
        &a,
        // Restated by claude/65357afb: the seam answers `roleNotAllowedHere` with 400 (`policy_error.rs:110,124`,
        // ruling #1157 issuecomment-6101472652; F12 #3298 issuecomment by gemini/studio-12).
        StatusCode::BAD_REQUEST,
        "roleNotAllowedHere",
        "a TELLER adds",
    );
    let s = save(&db, TELLER_B01, "TELLER", &names(&night_set())).await;
    refused(
        &s,
        // Restated by claude/65357afb: the seam answers `roleNotAllowedHere` with 400 (`policy_error.rs:110,124`,
        // ruling #1157 issuecomment-6101472652; F12 #3298 issuecomment by gemini/studio-12).
        StatusCode::BAD_REQUEST,
        "roleNotAllowedHere",
        "a TELLER saves",
    );
    let d = delete(&db, TELLER_B01, "TELLER").await;
    refused(
        &d,
        // Restated by claude/65357afb: the seam answers `roleNotAllowedHere` with 400 (`policy_error.rs:110,124`,
        // ruling #1157 issuecomment-6101472652; F12 #3298 issuecomment by gemini/studio-12).
        StatusCode::BAD_REQUEST,
        "roleNotAllowedHere",
        "a TELLER deletes",
    );

    assert!(!db.group_exists(NIGHT).await);
    assert_eq!(db.stored("TELLER").await, teller);
    assert_eq!(db.grant_version().await, version);
    assert_eq!(group_audit_rows(&db).await, 0);
    db.finish().await;
}

// ------------------------------------------------------------ guardrails (409 guardrail:<n>)

#[actix_web::test]
async fn saving_the_executive_group_is_guardrail_4() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let version = db.grant_version().await;
    let stored = db.stored("EXECUTIVE").await;
    let all = module_part(Role::Executive);
    let fewer = names(&all[1..]);

    let a = save(&db, EXEC, "EXECUTIVE", &fewer).await;
    refused(
        &a,
        StatusCode::CONFLICT,
        "guardrail:4",
        "save EXECUTIVE with fewer",
    );
    let same = save(&db, EXEC, "EXECUTIVE", &names(&all)).await;
    refused(
        &same,
        StatusCode::CONFLICT,
        "guardrail:4",
        "save EXECUTIVE unchanged",
    );

    assert_eq!(db.stored("EXECUTIVE").await, stored, "EXECUTIVE is fixed");
    assert_eq!(db.grant_version().await, version);
    assert_eq!(group_audit_rows(&db).await, 0);
    db.finish().await;
}

#[actix_web::test]
async fn an_admin_base_above_view_outside_its_modules_is_guardrail_3() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let version = db.grant_version().await;
    let finance_full = above_view_in(Module::Finance);

    let a = add(&db, EXEC, "NIGHT_ADMIN", "ADMIN", &names(&[finance_full])).await;
    refused(
        &a,
        StatusCode::CONFLICT,
        "guardrail:3",
        &format!("an ADMIN base with {finance_full:?}"),
    );

    let mut admin_plus = module_part(Role::Admin);
    admin_plus.push(finance_full);
    let s = save(&db, EXEC, "ADMIN", &names(&admin_plus)).await;
    refused(
        &s,
        StatusCode::CONFLICT,
        "guardrail:3",
        &format!("ADMIN saved with {finance_full:?}"),
    );

    assert!(!db.group_exists("NIGHT_ADMIN").await);
    assert_eq!(db.grant_version().await, version);
    assert_eq!(group_audit_rows(&db).await, 0);
    db.finish().await;
}

#[actix_web::test]
async fn user_management_above_view_on_a_teller_base_is_guardrail_2() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let version = db.grant_version().await;
    let users_full = above_view_in(Module::UserManagement);
    let settings_full = above_view_in(Module::SystemSettings);

    let mut with_users = night_set();
    with_users.push(users_full);
    let a = add(&db, EXEC, NIGHT, "TELLER", &names(&with_users)).await;
    refused(
        &a,
        StatusCode::CONFLICT,
        "guardrail:2",
        &format!("a TELLER base with {users_full:?}"),
    );

    let mut with_settings = module_part(Role::Teller);
    with_settings.push(settings_full);
    let s = save(&db, EXEC, "TELLER", &names(&with_settings)).await;
    refused(
        &s,
        StatusCode::CONFLICT,
        "guardrail:2",
        &format!("TELLER saved with {settings_full:?}"),
    );

    assert!(!db.group_exists(NIGHT).await);
    assert_eq!(db.grant_version().await, version);
    assert_eq!(group_audit_rows(&db).await, 0);
    db.finish().await;
}

#[actix_web::test]
async fn a_no_module_permission_in_the_request_is_guardrail_5() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    assert!(
        PermissionModule::module(Permission::SignedIn).is_none(),
        "SignedIn is a no-module permission"
    );
    let version = db.grant_version().await;
    let teller = db.stored("TELLER").await;

    let mut with_signed_in = night_set();
    with_signed_in.push(Permission::SignedIn);
    let a = add(&db, EXEC, NIGHT, "TELLER", &names(&with_signed_in)).await;
    refused(&a, StatusCode::CONFLICT, "guardrail:5", "add with SignedIn");

    let mut save_signed_in = module_part(Role::Teller);
    save_signed_in.push(Permission::SignedIn);
    let s = save(&db, EXEC, "TELLER", &names(&save_signed_in)).await;
    refused(
        &s,
        StatusCode::CONFLICT,
        "guardrail:5",
        "save with SignedIn",
    );

    assert!(!db.group_exists(NIGHT).await);
    assert_eq!(db.stored("TELLER").await, teller);
    assert_eq!(db.grant_version().await, version);
    assert_eq!(group_audit_rows(&db).await, 0);
    db.finish().await;
}

// ------------------------------------------------------------ delete

#[actix_web::test]
async fn a_built_in_group_cannot_be_deleted() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let version = db.grant_version().await;
    for built_in in ["TELLER", "EXECUTIVE"] {
        let a = delete(&db, EXEC, built_in).await;
        refused(
            &a,
            StatusCode::CONFLICT,
            "builtIn",
            &format!("delete {built_in}"),
        );
        assert!(db.group_exists(built_in).await, "{built_in} stays");
    }
    assert_eq!(db.grant_version().await, version);
    assert_eq!(group_audit_rows(&db).await, 0);
    db.finish().await;
}

#[actix_web::test]
async fn a_group_a_user_holds_cannot_be_deleted() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let a = add(&db, EXEC, NIGHT, "TELLER", &names(&night_set())).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let _holder = a_holder(&db, NIGHT).await;
    let version = db.grant_version().await;
    let stored = db.stored(NIGHT).await;
    let audits = group_audit_rows(&db).await;

    let d = delete(&db, EXEC, NIGHT).await;
    refused(&d, StatusCode::CONFLICT, "inUse", "delete a held group");
    assert!(db.group_exists(NIGHT).await);
    assert_eq!(db.stored(NIGHT).await, stored);
    assert_eq!(db.grant_version().await, version);
    assert_eq!(group_audit_rows(&db).await, audits);
    db.finish().await;
}

#[actix_web::test]
async fn an_unused_custom_group_is_deleted() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let a = add(&db, EXEC, NIGHT, "TELLER", &names(&night_set())).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let before = db.grant_version().await;

    let d = delete(&db, EXEC, NIGHT).await;
    changed(&db, &d, NIGHT, before, "delete NIGHT_TELLER").await;
    assert!(!db.group_exists(NIGHT).await, "the roles row is gone");
    assert!(db.stored(NIGHT).await.is_empty(), "its set is gone");

    let night = user_of("NIGHT_TELLER:TELLER@XC-B01").expect("a caller");
    for p in RoleGrants::seed(Role::Teller) {
        assert_eq!(
            Authorizer::check(Some(&night), p, &at_b01(), now()),
            Err(Refusal::RoleNotAllowed),
            "a deleted group holds nothing, not its base's {p:?}"
        );
    }
    db.finish().await;
}

#[actix_web::test]
async fn deleting_an_unknown_group_is_not_found() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let version = db.grant_version().await;
    let d = delete(&db, EXEC, "NO_SUCH_GROUP").await;
    refused(
        &d,
        StatusCode::NOT_FOUND,
        "notFound",
        "delete an unknown group",
    );
    assert_eq!(db.grant_version().await, version);
    assert_eq!(group_audit_rows(&db).await, 0);
    db.finish().await;
}

// ------------------------------------------------------------ audit (§11.3: one row per change)

async fn audit_rows(db: &TestDb, subject: &str, action: &str, actor: &str) -> i64 {
    db.count(&format!(
        "SELECT count(*) FROM user_audit \
         WHERE subject_id = '{subject}' AND action = '{action}' AND actor_id = '{actor}'"
    ))
    .await
}

/// `before`/`after` of the one row: `"null"` for SQL NULL or JSON null, else the JSON text.
async fn before_after(db: &TestDb, subject: &str, action: &str) -> (String, String) {
    sqlx::query_as(AssertSqlSafe(format!(
        "SELECT COALESCE(before, 'null'::jsonb)::text, COALESCE(after, 'null'::jsonb)::text \
         FROM user_audit WHERE subject_id = '{subject}' AND action = '{action}'"
    )))
    .fetch_one(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("the {action} row of {subject}: {e}"))
}

#[actix_web::test]
async fn add_save_and_delete_each_write_one_audit_row() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let actor = user_id(EXEC);

    let a = add(&db, EXEC, NIGHT, "TELLER", &names(&night_set())).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        audit_rows(&db, NIGHT, "GROUP_ADD", &actor).await,
        1,
        "GROUP_ADD"
    );
    let (before, after) = before_after(&db, NIGHT, "GROUP_ADD").await;
    assert_eq!(before, "null", "an add has no before");
    assert_ne!(after, "null", "an add has an after");

    let s = save(&db, EXEC, NIGHT, &names(&module_part(Role::Teller))).await;
    assert_eq!(s.status, StatusCode::OK, "{}", s.text);
    assert_eq!(
        audit_rows(&db, NIGHT, "PERMISSION_CHANGE", &actor).await,
        1,
        "PERMISSION_CHANGE"
    );
    let (before, after) = before_after(&db, NIGHT, "PERMISSION_CHANGE").await;
    assert_ne!(before, "null", "a save has a before");
    assert_ne!(after, "null", "a save has an after");
    assert_ne!(before, after, "the set changed");

    let d = delete(&db, EXEC, NIGHT).await;
    assert_eq!(d.status, StatusCode::OK, "{}", d.text);
    assert_eq!(
        audit_rows(&db, NIGHT, "GROUP_DELETE", &actor).await,
        1,
        "GROUP_DELETE"
    );
    let (before, after) = before_after(&db, NIGHT, "GROUP_DELETE").await;
    assert_ne!(before, "null", "a delete has a before");
    assert_eq!(after, "null", "a delete has no after");

    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM user_audit WHERE subject_id = '{NIGHT}'"
        ))
        .await,
        3,
        "nothing else"
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_save_of_a_built_in_group_is_audited_under_its_code() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let all = module_part(Role::Teller);
    let s = save(&db, EXEC, "TELLER", &names(&all[1..])).await;
    assert_eq!(s.status, StatusCode::OK, "{}", s.text);
    assert_eq!(
        audit_rows(&db, "TELLER", "PERMISSION_CHANGE", &user_id(EXEC)).await,
        1
    );
    assert_eq!(group_audit_rows(&db).await, 1, "one row in all");
    db.finish().await;
}
