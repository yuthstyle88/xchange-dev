//! Up-front tests for RBAC §11 F5 (design `docs/design/blueprint/user-management-rbac.md` §11.4, §11.6): the
//! `admin_permission_matrix` view (`api/src/presentation/http/v3/views/admin_permission_matrix.rs`,
//! data `api/src/presentation/dto/v3/admin_permission_matrix.rs`).
//! Written before the implementation by the sub-controller claude/65357afb on `feat/rbac-groups`.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! What they pin:
//! - `modules` is the ten `Module::ALL` in order (`Module::as_str`);
//! - `permissions` is `Permission::ALL` in order, each with its module and level from `PermissionModule::module` and
//!   `editable` = it has a module (§11.3 rule 5);
//! - `groups` is the eight built-in staff groups in `RoleGrants::BUILT_IN` order without CUSTOMER, then every custom
//!   group by code; each with its name, base, `builtIn`, its stored set in `Permission::ALL` order, and the highest
//!   level per module among them;
//! - `editable`: for an EXECUTIVE caller every group but EXECUTIVE (§11.3 rule 4); for anyone else, none;
//! - the view needs `UsersManage` (`adminPermissionMatrix`): ADMIN reads it, a TELLER is refused;
//! - its version is `role_grant_version`, and it moves when a group's set is saved, with no help from the test
//!   (the view reloads a stale process snapshot itself).
//!
//! Every expected permission set is computed from `RoleGrants::seed`, `PermissionModule::module` and
//! `Permission::ALL`, or read back from `role_permission_grants`; none is typed by hand.
//!
//! Harness copied from `rbac_c1a_user_commands_test.rs`: the `x-test-as` caller is `ROLE@branch` (`@*` =
//! organisation-wide). Each test stages its own schema, runs every migration (M2 included), then does what startup
//! does (§11.5 item 2): `seed_missing`, then installs the loaded snapshot. Custom groups and saves go through
//! `PgRoleGrantStore` (C6), not F12's commands, so F5 does not wait for F12. `RoleGrants::install` is
//! process-global, so the tests of this file run one at a time (`SERIAL`) and each re-installs from its own schema
//! before it starts.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing skips.

#![allow(dead_code)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::permission_module::{Level, Module, PermissionModule};
use xc_auth::domain::access::{
    Actor as User, Group, Permission, Role, RoleGrantStore, RoleGrants, ScopedRole,
};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
use xc_auth::infrastructure::access::PgRoleGrantStore;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SEEDED_XC_ORG_ID, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const B01: &str = "XC-B01";

const EXEC: &str = "EXECUTIVE@*";
const ORG_ADMIN: &str = "ADMIN@*";
const TELLER_B01: &str = "TELLER@XC-B01";

const VIEW: &str = "/api/v3/views/admin_permission_matrix";

/// `RoleGrants::install` is process-global: one test of this file at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
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

/// `ps` in `Permission::ALL` order, by serde name.
fn in_all_order(ps: &BTreeSet<Permission>) -> Vec<String> {
    Permission::ALL
        .into_iter()
        .filter(|p| ps.contains(p))
        .map(name)
        .collect()
}

/// The permission whose serde name is `n`.
fn by_name(n: &str) -> Permission {
    Permission::ALL
        .into_iter()
        .find(|p| name(*p) == n)
        .unwrap_or_else(|| panic!("a stored permission name that is no Permission: {n}"))
}

/// The highest level per module among `ps`, keyed by `Module::as_str`.
fn levels_of(ps: &BTreeSet<Permission>) -> Value {
    let mut top: BTreeMap<Module, Level> = BTreeMap::new();
    for p in ps {
        if let Some((m, l)) = PermissionModule::module(*p) {
            top.entry(m)
                .and_modify(|cur| {
                    if l > *cur {
                        *cur = l;
                    }
                })
                .or_insert(l);
        }
    }
    let out: BTreeMap<String, String> = top
        .into_iter()
        .map(|(m, l)| (m.as_str().to_owned(), l.as_str().to_owned()))
        .collect();
    serde_json::to_value(out).expect("levels serialise")
}

fn module_part(role: Role) -> Vec<Permission> {
    RoleGrants::seed(role)
        .into_iter()
        .filter(|p| PermissionModule::module(*p).is_some())
        .collect()
}

fn no_module_part(role: Role) -> Vec<Permission> {
    RoleGrants::seed(role)
        .into_iter()
        .filter(|p| PermissionModule::module(*p).is_none())
        .collect()
}

/// The eight staff roles in `RoleGrants::BUILT_IN` order: CUSTOMER is not shown.
fn staff_roles() -> Vec<Role> {
    RoleGrants::BUILT_IN
        .into_iter()
        .filter(|r| *r != Role::Customer)
        .collect()
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
        let schema = unique("rbacf5");
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
        let mut conn = self.pool.acquire().await.expect("a connection");
        let snapshot = store.load(&mut conn).await.expect("load the groups");
        RoleGrants::install(snapshot);
    }

    /// Adds a custom group through the store, committed. The process snapshot is left stale on purpose.
    async fn add_group(&self, code: &str, name: &str, base: Role, set: &[Permission]) {
        let group = Group {
            code: code.to_owned(),
            name: name.to_owned(),
            base,
            built_in: false,
        };
        let mut tx = self.pool.begin().await.expect("begin");
        PgRoleGrantStore::new()
            .add_group(&mut tx, &group, set)
            .await
            .expect("add the group");
        tx.commit().await.expect("commit the group");
    }

    /// Replaces `code`'s set through the store, committed. The process snapshot is left stale on purpose.
    async fn save_set(&self, code: &str, set: &[Permission]) {
        let mut tx = self.pool.begin().await.expect("begin");
        PgRoleGrantStore::new()
            .save_set(&mut tx, code, set)
            .await
            .expect("save the set");
        tx.commit().await.expect("commit the set");
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn texts(&self, sql: &str) -> Vec<String> {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_all(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn grant_version(&self) -> i64 {
        self.count("SELECT version::bigint FROM role_grant_version")
            .await
    }

    /// The stored set of `code`.
    async fn stored(&self, code: &str) -> BTreeSet<Permission> {
        self.texts(&format!(
            "SELECT permission FROM role_permission_grants WHERE role_code = '{code}'"
        ))
        .await
        .iter()
        .map(|n| by_name(n))
        .collect()
    }

    /// What `groups` must be, read back from `roles` and `role_permission_grants`: the staff built-ins in
    /// `RoleGrants::BUILT_IN` order, then custom groups by code.
    async fn expected_groups(&self, caller_is_executive: bool) -> Value {
        type GroupRow = (String, String, String, bool);
        let rows: Vec<GroupRow> = sqlx::query_as(
            "SELECT code, name, base_role, built_in FROM roles WHERE code <> 'CUSTOMER'",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read the groups");
        let order: Vec<&str> = staff_roles().into_iter().map(Role::as_str).collect();
        let mut rows: Vec<((usize, String), GroupRow)> = rows
            .into_iter()
            .map(|row| {
                let key = match order.iter().position(|c| *c == row.0) {
                    Some(i) if row.3 => (i, String::new()),
                    _ => (order.len(), row.0.clone()),
                };
                (key, row)
            })
            .collect();
        rows.sort();
        let mut out = Vec::new();
        for (_, (code, name, base, built_in)) in rows {
            let set = self.stored(&code).await;
            let editable = caller_is_executive && code != "EXECUTIVE";
            out.push(json!({
                "code": code,
                "name": name,
                "base": base,
                "builtIn": built_in,
                "editable": editable,
                "permissions": in_all_order(&set),
                "levels": levels_of(&set),
            }));
        }
        Value::Array(out)
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

async fn get(db: &TestDb, who: &str, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who));
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// A `200` with data: answers `data`.
fn data_of(a: &Answer, what: &str) -> Value {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    assert_eq!(a.body["changed"], json!(true), "{what}: {}", a.text);
    a.body["data"].clone()
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// The group `code` in `data.groups`.
fn group<'a>(data: &'a Value, code: &str) -> &'a Value {
    data["groups"]
        .as_array()
        .expect("groups is an array")
        .iter()
        .find(|g| g["code"] == json!(code))
        .unwrap_or_else(|| panic!("group {code} in {data}"))
}

// ------------------------------------------------------------ modules and permissions

#[actix_web::test]
async fn an_executive_reads_the_modules_and_permissions() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let a = get(&db, EXEC, VIEW).await;
    let data = data_of(&a, "EXECUTIVE reads the matrix");

    let modules: Vec<&str> = Module::ALL.into_iter().map(Module::as_str).collect();
    assert_eq!(
        data["modules"],
        json!(modules),
        "the ten modules in V.9 order"
    );

    let permissions: Vec<Value> = Permission::ALL
        .into_iter()
        .map(|p| {
            let at = PermissionModule::module(p);
            json!({
                "name": name(p),
                "module": at.map(|(m, _)| m.as_str()),
                "level": at.map(|(_, l)| l.as_str()),
                "editable": at.is_some(),
            })
        })
        .collect();
    assert_eq!(
        data["permissions"],
        Value::Array(permissions),
        "every permission in Permission::ALL order"
    );
    db.finish().await;
}

// ------------------------------------------------------------ groups

/// A fresh store: the eight staff groups in `RoleGrants::BUILT_IN` order, each holding its seed.
#[actix_web::test]
async fn an_executive_reads_the_built_in_groups_with_their_seed() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let a = get(&db, EXEC, VIEW).await;
    let data = data_of(&a, "EXECUTIVE reads the matrix");

    let groups = data["groups"].as_array().expect("groups is an array");
    let codes: Vec<&str> = groups
        .iter()
        .map(|g| g["code"].as_str().expect("a code"))
        .collect();
    let want: Vec<&str> = staff_roles().into_iter().map(Role::as_str).collect();
    assert_eq!(codes, want, "the staff built-ins, CUSTOMER left out");

    for role in staff_roles() {
        let seed: BTreeSet<Permission> = RoleGrants::seed(role).into_iter().collect();
        let g = group(&data, role.as_str());
        assert_eq!(g["name"], json!(role.as_str()), "{role:?}: name");
        assert_eq!(
            g["base"],
            json!(role.as_str()),
            "{role:?}: its base is itself"
        );
        assert_eq!(g["builtIn"], json!(true), "{role:?}: built in");
        assert_eq!(
            g["permissions"],
            json!(in_all_order(&seed)),
            "{role:?}: its seed, in Permission::ALL order"
        );
        assert_eq!(
            g["levels"],
            levels_of(&seed),
            "{role:?}: highest level per module"
        );
        assert_eq!(
            g["editable"],
            json!(role != Role::Executive),
            "{role:?}: editable for EXECUTIVE, except EXECUTIVE itself"
        );
    }
    assert_eq!(data["groups"], db.expected_groups(true).await);
    db.finish().await;
}

/// Custom groups follow the built-ins, by code, with their stored set; the view reloads the stale snapshot itself.
#[actix_web::test]
async fn custom_groups_follow_the_built_ins_by_code() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let module = module_part(Role::Teller);
    let mut night: Vec<Permission> = no_module_part(Role::Teller);
    night.extend(module.iter().copied().step_by(2));
    let mut zed: Vec<Permission> = no_module_part(Role::Teller);
    zed.extend(module.iter().copied().skip(1).step_by(2));
    db.add_group("ZED_TELLER", "Zed teller", Role::Teller, &zed)
        .await;
    db.add_group("NIGHT_TELLER", "Night teller", Role::Teller, &night)
        .await;

    let a = get(&db, EXEC, VIEW).await;
    let data = data_of(&a, "EXECUTIVE reads the matrix");
    let codes: Vec<&str> = data["groups"]
        .as_array()
        .expect("groups is an array")
        .iter()
        .map(|g| g["code"].as_str().expect("a code"))
        .collect();
    let mut want: Vec<&str> = staff_roles().into_iter().map(Role::as_str).collect();
    want.extend(["NIGHT_TELLER", "ZED_TELLER"]);
    assert_eq!(codes, want, "built-ins first, then custom groups by code");

    let night_set: BTreeSet<Permission> = night.iter().copied().collect();
    let g = group(&data, "NIGHT_TELLER");
    assert_eq!(g["name"], json!("Night teller"));
    assert_eq!(g["base"], json!("TELLER"));
    assert_eq!(g["builtIn"], json!(false));
    assert_eq!(g["editable"], json!(true));
    assert_eq!(g["permissions"], json!(in_all_order(&night_set)));
    assert_eq!(g["levels"], levels_of(&night_set));

    assert_eq!(data["groups"], db.expected_groups(true).await);
    db.finish().await;
}

/// ADMIN holds `UsersManage` and reads the matrix; only EXECUTIVE edits, so nothing is editable for ADMIN.
#[actix_web::test]
async fn an_admin_reads_every_group_read_only() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let module = module_part(Role::Teller);
    let mut night: Vec<Permission> = no_module_part(Role::Teller);
    night.extend(module.iter().copied().step_by(2));
    db.add_group("NIGHT_TELLER", "Night teller", Role::Teller, &night)
        .await;

    let a = get(&db, ORG_ADMIN, VIEW).await;
    let data = data_of(&a, "ADMIN reads the matrix");
    for g in data["groups"].as_array().expect("groups is an array") {
        assert_eq!(g["editable"], json!(false), "not editable for ADMIN: {g}");
    }
    assert_eq!(data["groups"], db.expected_groups(false).await);
    db.finish().await;
}

#[actix_web::test]
async fn a_teller_is_refused() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let a = get(&db, TELLER_B01, VIEW).await;
    // Restated by claude/65357afb: the seam answers `roleNotAllowedHere` with 400 (`policy_error.rs:110,124`,
    // ruling #1157 issuecomment-6101472652).
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{}", a.text);
    assert_eq!(code(&a), "roleNotAllowedHere", "{}", a.text);
    assert!(a.body.get("data").is_none(), "no data: {}", a.text);
    db.finish().await;
}

// ------------------------------------------------------------ version

#[actix_web::test]
async fn the_version_is_role_grant_version_and_moves_on_a_save() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;

    let first = get(&db, EXEC, VIEW).await;
    let data = data_of(&first, "first read");
    let v1 = first.body["version"].as_i64().expect("a numeric version");
    assert_eq!(
        v1,
        db.grant_version().await,
        "the version is role_grant_version"
    );
    assert_eq!(data["version"], json!(v1), "data.version is the same");

    let same = get(&db, EXEC, &format!("{VIEW}?since={v1}")).await;
    assert_eq!(same.status, StatusCode::OK, "{}", same.text);
    assert_eq!(
        same.body["changed"],
        json!(false),
        "nothing moved: {}",
        same.text
    );

    let all = module_part(Role::Teller);
    let mut fewer: Vec<Permission> = no_module_part(Role::Teller);
    fewer.extend(all.iter().copied().skip(1));
    db.save_set("TELLER", &fewer).await;
    let stored = db.grant_version().await;
    assert!(stored > v1, "the store bumped the version");

    let after = get(&db, EXEC, &format!("{VIEW}?since={v1}")).await;
    let data = data_of(&after, "read after the save");
    let v2 = after.body["version"].as_i64().expect("a numeric version");
    assert_eq!(
        v2, stored,
        "the version is role_grant_version after the save"
    );
    assert_eq!(data["version"], json!(v2));

    let fewer_set: BTreeSet<Permission> = fewer.iter().copied().collect();
    let g = group(&data, "TELLER");
    assert_eq!(
        g["permissions"],
        json!(in_all_order(&fewer_set)),
        "TELLER shows the saved set, not the stale snapshot's"
    );
    assert_eq!(g["levels"], levels_of(&fewer_set));
    assert_eq!(data["groups"], db.expected_groups(true).await);
    db.finish().await;
}
