//! Up-front tests for AP-12 (design `docs/design/blueprint/user-management-rbac.md` §11.9, rows A7 and A8): the
//! matrix save (`adminGroupAdd` / `adminGroupSave`, `check_and_build_full_set` in
//! `api/src/presentation/http/v3/commands/admin_groups.rs`) lets the nine approval kinds through as sent and sets
//! `ApprovalsDecide` iff the stored set holds one of them.
//! Written before the implementation on the BE lane controller's order (`feat/approval-perms`).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! §11.9: today `check_and_build_full_set` rebuilds every no-module permission from the base's seed and refuses any
//! it is sent (`guardrail:5`). After AP-12 the nine pass through as sent (A8: the save normalises
//! `ApprovalsDecide`; it does not refuse), and every other no-module permission is still the base's seed.
//!
//! These rely on AP-2 (`matrix_guardrails.rs`, `33084a4e`) as well as AP-12: the full set is still checked by
//! `MatrixGuardrails::check`, whose rule 5 must not compare the nine and `ApprovalsDecide` against the base's seed,
//! and whose rule 6 (`guardrail:6`, A7) is AP-2's. Written against `c913791c` (before AP-2) and re-run on
//! `ef5c204d` (AP-1 and AP-2 merged): both times they fail at `check_and_build_full_set` (`guardrail:5`, or the
//! base's no-module seed stored). Tests marked "passes today" are regression locks.
//!
//! Harness copied from `rbac_f12_admin_groups_test.rs`: the `x-test-as` caller is `ROLE@branch` (`@*` =
//! organisation-wide). Each test stages its own schema, runs every migration, then does what startup does
//! (`seed_missing`, then installs the loaded snapshot). `RoleGrants::install` is process-global, so the tests of this
//! file run one at a time (`SERIAL`) and each re-installs from its own schema before it starts; CI runs nextest, one
//! process per test.
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
use xc_auth::domain::access::permission_module::{APPROVAL_KINDS, PermissionModule};
use xc_auth::domain::access::{
    Actor as User, Permission, Role, RoleGrantStore, RoleGrants, ScopedRole,
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
        let schema = unique("ap12groups");
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


// ------------------------------------------------------------ AP-12

use Permission as P;

/// `perms` with `ApprovalsDecide` added iff they hold one of the nine (§11.9 "`ApprovalsDecide` follows the kinds").
fn with_decide_following(mut perms: BTreeSet<String>) -> BTreeSet<String> {
    let kinds: BTreeSet<String> = APPROVAL_KINDS.into_iter().map(name).collect();
    perms.remove(&name(P::ApprovalsDecide));
    if perms.iter().any(|p| kinds.contains(p)) {
        perms.insert(name(P::ApprovalsDecide));
    }
    perms
}

/// `role`'s no-module seed without the nine and without `ApprovalsDecide`: what the save still rebuilds from the base.
fn other_no_module(role: Role) -> Vec<Permission> {
    no_module_part(role)
        .into_iter()
        .filter(|p| !APPROVAL_KINDS.contains(p) && *p != P::ApprovalsDecide)
        .collect()
}

/// What a save of `module` + `kinds` on a group based on `base` stores (§11.9, A8).
fn expected(base: Role, module: &[Permission], kinds: &[Permission]) -> BTreeSet<String> {
    let mut want = name_set(module);
    want.extend(names(&other_no_module(base)));
    want.extend(names(kinds));
    with_decide_following(want)
}

/// §11.9 A8 (A2's group through the HTTP save): adding `NIGHT_FIN`, base TELLER, with `ApproveTransactionReversal`
/// stores the kind and `ApprovalsDecide`, and the process's snapshot holds it right after the answer.
#[actix_web::test]
async fn ap12_a8_a_teller_based_group_with_a_kind_stores_it_and_approvals_decide() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let before = db.grant_version().await;
    let module = night_set();
    let mut sent = names(&module);
    sent.push(name(P::ApproveTransactionReversal));

    let a = add(&db, EXEC, "NIGHT_FIN", "TELLER", &sent).await;
    changed(&db, &a, "NIGHT_FIN", before, "add NIGHT_FIN with the reversal kind").await;

    let stored = db.stored("NIGHT_FIN").await;
    assert_eq!(
        stored,
        expected(Role::Teller, &module, &[P::ApproveTransactionReversal]),
        "the module set, TELLER's other no-module seed, the kind and ApprovalsDecide"
    );
    assert!(stored.contains(&name(P::ApprovalsDecide)));
    assert!(
        RoleGrants::holds("NIGHT_FIN", P::ApproveTransactionReversal),
        "the process's snapshot holds the kind after the add"
    );
    db.finish().await;
}

/// §11.9 A8 through `adminGroupSave`: saving built-in FINANCE with its module set and only
/// `ApproveTransactionReversal` stores that kind and `ApprovalsDecide`, and drops `ApproveCountDifference` (A3's setup).
/// Saving it again with none of the nine stores no kind and no `ApprovalsDecide`.
#[actix_web::test]
async fn ap12_a8_a_save_lets_the_kinds_through_as_sent() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let module = module_part(Role::Finance);

    let mut sent = names(&module);
    sent.push(name(P::ApproveTransactionReversal));
    let before = db.grant_version().await;
    let s = save(&db, EXEC, "FINANCE", &sent).await;
    changed(&db, &s, "FINANCE", before, "save FINANCE with the reversal kind only").await;
    let stored = db.stored("FINANCE").await;
    assert_eq!(
        stored,
        expected(Role::Finance, &module, &[P::ApproveTransactionReversal])
    );
    assert!(!stored.contains(&name(P::ApproveCountDifference)), "the count-difference kind is dropped");
    assert!(!RoleGrants::holds("FINANCE", P::ApproveCountDifference));

    let before = db.grant_version().await;
    let s = save(&db, EXEC, "FINANCE", &names(&module)).await;
    changed(&db, &s, "FINANCE", before, "save FINANCE with none of the nine").await;
    let stored = db.stored("FINANCE").await;
    assert_eq!(stored, expected(Role::Finance, &module, &[]));
    assert!(
        !stored.contains(&name(P::ApprovalsDecide)),
        "FINANCE holding none of the nine holds no ApprovalsDecide"
    );
    db.finish().await;
}

/// §11.9 A8: adding a group based on BRANCH_MANAGER (whose seed holds three kinds) with none of the nine stores no
/// kind and no `ApprovalsDecide`.
#[actix_web::test]
async fn ap12_a8_a_group_with_none_of_the_nine_stores_no_approvals_decide() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let before = db.grant_version().await;
    let module = module_part(Role::BranchManager);

    let a = add(&db, EXEC, "DESK_LEAD", "BRANCH_MANAGER", &names(&module)).await;
    changed(&db, &a, "DESK_LEAD", before, "add DESK_LEAD with none of the nine").await;

    let stored = db.stored("DESK_LEAD").await;
    assert_eq!(stored, expected(Role::BranchManager, &module, &[]));
    assert!(!stored.contains(&name(P::ApprovalsDecide)));
    db.finish().await;
}

/// §11.9 A8 (passes today): a TELLER-based group saved with none of the nine holds no `ApprovalsDecide`.
#[actix_web::test]
async fn ap12_a8_a_teller_based_group_with_none_of_the_nine_holds_no_approvals_decide() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let before = db.grant_version().await;
    let module = night_set();

    let a = add(&db, EXEC, NIGHT, "TELLER", &names(&module)).await;
    changed(&db, &a, NIGHT, before, "add NIGHT_TELLER with none of the nine").await;

    let stored = db.stored(NIGHT).await;
    assert!(!stored.contains(&name(P::ApprovalsDecide)));
    assert!(APPROVAL_KINDS.iter().all(|p| !stored.contains(&name(*p))));
    db.finish().await;
}

/// §11.9 A7 (needs AP-2's rule 6 as well): adding or saving a group based on ADMIN with an approval kind is refused
/// `409 guardrail:6`, and nothing is stored.
#[actix_web::test]
async fn ap12_a7_an_admin_based_group_with_a_kind_is_guardrail_6() {
    let _serial = SERIAL.lock().await;
    let db = TestDb::start().await;
    let version = db.grant_version().await;
    let admin_before = db.stored("ADMIN").await;

    let a = add(
        &db,
        EXEC,
        "NIGHT_ADMIN",
        "ADMIN",
        &names(&[P::ApproveNegativeStock]),
    )
    .await;
    refused(&a, StatusCode::CONFLICT, "guardrail:6", "add an ADMIN-based group with ApproveNegativeStock");
    assert!(!db.group_exists("NIGHT_ADMIN").await, "nothing is added");

    let mut sent = names(&module_part(Role::Admin));
    sent.push(name(P::ApproveNegativeStock));
    let s = save(&db, EXEC, "ADMIN", &sent).await;
    refused(&s, StatusCode::CONFLICT, "guardrail:6", "save ADMIN with ApproveNegativeStock");
    assert_eq!(db.stored("ADMIN").await, admin_before, "ADMIN is unchanged");
    assert_eq!(db.grant_version().await, version, "no version bump");
    db.finish().await;
}
