//! Specification tests for V0-B (docs/design/xchange-v3-web.md §4.1, §4.2, §4.4, §5.1, §9):
//! the per-screen views that answer only what changed.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! - `presentation::http::v3::view::{BranchView, PagedView, ViewEndpoint, RowId}`:
//!   `ViewEndpoint::register(cfg, Arc<V>)` serves `GET /api/v3/views/{V::NAME}`,
//!   `ViewEndpoint::register_paged(cfg, Arc<V>)` the same for a paged view.
//! - `xc_kernel::branch_versions::BranchVersions::new(pool).current(&branch)`
//!   reads the `branch_versions` row `BranchTx` writes (0 when there is none).
//! - Who is signed in comes from `web::Data<dyn pages::SessionReader>`, as for
//!   every other route. A view on `EntryPath::Rpc` answers the owner session
//!   first, else the staff session (as `rpc.rs` `caller_of_rpc`); a view on
//!   `EntryPath::StaffRpc` answers the staff session only (`caller_of_staff_rpc`).
//! - `GET /api/v3/views/me` is registered by the generated view table
//!   (`http/v3/views/me.rs`), with no edit to any shared file.
//!
//! These tests need a real PostgreSQL: `TEST_DATABASE_URL` must name an
//! existing database whose name ends in `_test`. They fail without it; they do
//! not skip. Each test works in a schema of its own.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use serde::Serialize;
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use ts_rs::TS;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::dto::v3::common::NoFilter;
use xchange::presentation::http::v3::view::{BranchView, PagedView, RowId, ViewEndpoint};
use xchange::presentation::pages::{SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
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
                "TEST_DATABASE_URL is not set.\n\
                 The v3 view tests need a real PostgreSQL and do not skip. Point it at an \
                 existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`."
        );
        let schema = unique("v3view");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
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
            .expect("apply api/migrations to the test schema");
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B02', 'B02'), ('XC-NEW', 'NEW')")
            .execute(&pool)
            .await
            .expect("insert the branches this file names into branches");
        Self { pool, schema }
    }

    /// One committed `BranchTx` run on `branch` that writes nothing: the
    /// version still rises (#231).
    async fn write(&self, branch: &str) {
        BranchTx::new(Transactor::new(self.pool.clone()))
            .run(&BranchId::new(branch), |_tx| {
                Box::pin(async move { AppResult::Ok(()) })
            })
            .await
            .expect("a BranchTx run commits");
    }

    /// Drop the schema. Reached only by a test that passed.
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

/// Who is signed in, from the request header `x-test-as`: `owner`,
/// `staff:<ROLE>:<branch>`, `owner+staff:<ROLE>:<branch>`, or absent.
struct HeaderSessions;

fn staff_of(spec: &str) -> Option<Actor> {
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = Role::parse(parts.next().expect("a role")).expect("a known role");
    let branch = parts.next().expect("a branch");
    Actor::staff("employee-7", role, BranchId::new(branch))
}

fn sessions_of(spec: &str) -> Sessions {
    if spec.is_empty() {
        return Sessions {
            owner: None,
            staff: None,
        };
    }
    if spec == "owner" {
        return Sessions {
            owner: Some(Actor::owner("owner-1")),
            staff: None,
        };
    }
    if let Some(staff) = spec.strip_prefix("owner+") {
        return Sessions {
            owner: Some(Actor::owner("owner-1")),
            staff: staff_of(staff),
        };
    }
    Sessions {
        owner: None,
        staff: staff_of(spec),
    }
}

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { sessions_of(&spec) })
    }
}

const STAFF_ROLES: [&str; 9] = [
    "FRONT",
    "BACKEND",
    "MANAGER",
    "RATES",
    "MONITOR",
    "FINANCE",
    "SYSTEM",
    "DOCUMENTS",
    "DOCUMENT_REVIEW",
];

/// Every caller kind: nobody, the owner, each staff role, and the owner and a
/// staff member in one browser.
fn every_caller() -> Vec<String> {
    let mut out = vec![String::new(), "owner".to_string()];
    for role in STAFF_ROLES {
        out.push(format!("staff:{role}:XC-B01"));
        out.push(format!("owner+staff:{role}:XC-B01"));
    }
    out
}

/// The actor the policy is asked about on `path`, by the rule of `rpc.rs`.
fn actor_on(path: EntryPath, spec: &str) -> Option<Actor> {
    let s = sessions_of(spec);
    match path {
        EntryPath::StaffRpc => s.staff,
        _ => s.owner.or(s.staff),
    }
}

// ------------------------------------------------------------ test views

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct Echo {
    branch: String,
    who: String,
}

/// Counts its loads; answers the branch and the caller it was loaded for.
struct Counting {
    loads: AtomicUsize,
}

impl BranchView for Counting {
    const NAME: &'static str = "test-count";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Filter = NoFilter;
    type Data = Echo;

    async fn load(&self, branch: &BranchId, who: &Actor, _filter: &NoFilter) -> AppResult<Echo> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        Ok(Echo {
            branch: branch.as_str().to_owned(),
            who: who.id().to_owned(),
        })
    }
}

struct SaveRate;

impl BranchView for SaveRate {
    const NAME: &'static str = "test-save-rate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSaveRate");
    type Filter = NoFilter;
    type Data = Echo;

    async fn load(&self, branch: &BranchId, who: &Actor, _filter: &NoFilter) -> AppResult<Echo> {
        Ok(Echo {
            branch: branch.as_str().to_owned(),
            who: who.id().to_owned(),
        })
    }
}

struct DeskConfig;

impl BranchView for DeskConfig {
    const NAME: &'static str = "test-desk-config";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::StaffRpc, "adminDeskConfig");
    type Filter = NoFilter;
    type Data = Echo;

    async fn load(&self, branch: &BranchId, who: &Actor, _filter: &NoFilter) -> AppResult<Echo> {
        Ok(Echo {
            branch: branch.as_str().to_owned(),
            who: who.id().to_owned(),
        })
    }
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct Row {
    id: String,
}

/// Records the `before` and `limit` of every page it was asked for.
struct Paging {
    asked: Mutex<Vec<(Option<RowId>, u16)>>,
}

impl PagedView for Paging {
    const NAME: &'static str = "test-paged";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Filter = NoFilter;
    type Row = Row;

    async fn page(
        &self,
        _branch: &BranchId,
        _who: &Actor,
        _filter: &NoFilter,
        before: Option<RowId>,
        limit: u16,
    ) -> AppResult<Vec<Row>> {
        self.asked
            .lock()
            .expect("not poisoned")
            .push((before, limit));
        Ok(vec![Row { id: "r2".into() }, Row { id: "r1".into() }])
    }
}

struct Views {
    counting: Arc<Counting>,
    paging: Arc<Paging>,
}

impl Views {
    fn new() -> Self {
        Self {
            counting: Arc::new(Counting {
                loads: AtomicUsize::new(0),
            }),
            paging: Arc::new(Paging {
                asked: Mutex::new(Vec::new()),
            }),
        }
    }

    fn loads(&self) -> usize {
        self.counting.loads.load(Ordering::SeqCst)
    }
}

#[derive(Debug)]
struct Answer {
    status: StatusCode,
    cache_control: Option<String>,
    body: Value,
}

/// `GET uri` as `who`, on the whole application (`bootstrap::configure`) with
/// the test views registered beside it.
async fn get(db: &TestDb, views: &Views, who: &str, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let counting = views.counting.clone();
    let paging = views.paging.clone();
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| {
                ViewEndpoint::register(cfg, counting);
                ViewEndpoint::register(cfg, Arc::new(SaveRate));
                ViewEndpoint::register(cfg, Arc::new(DeskConfig));
                ViewEndpoint::register_paged(cfg, paging);
                bootstrap::configure(cfg, state);
            })
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::get()
        .uri(uri)
        .insert_header((header::HOST, "xc.test"));
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let cache_control = resp
        .headers()
        .get(header::CACHE_CONTROL)
        .map(|v| v.to_str().expect("text").to_owned());
    let bytes = atest::read_body(resp).await;
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "GET {uri} as {who:?}: a JSON body ({e}): {:?}",
            String::from_utf8_lossy(&bytes)
        )
    });
    Answer {
        status,
        cache_control,
        body,
    }
}

fn is_unchanged(a: &Answer, version: i64, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: status; body {}", a.body);
    assert_eq!(
        a.body,
        serde_json::json!({ "version": version, "changed": false }),
        "{what}: exactly the version and changed: false, no data"
    );
}

fn is_changed(a: &Answer, version: i64, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: status; body {}", a.body);
    assert_eq!(a.body["version"], version, "{what}: version in {}", a.body);
    assert_eq!(a.body["changed"], true, "{what}: changed in {}", a.body);
    assert!(
        a.body.get("data").is_some_and(|d| !d.is_null()),
        "{what}: data in {}",
        a.body
    );
}

/// The policy's refusal, as a v3 refusal (§4.3): the policy's status, its
/// sentence byte for byte, a camelCase code, and no field.
fn is_refusal(a: &Answer, status: u16, message: &str, what: &str) {
    assert_eq!(a.status.as_u16(), status, "{what}: status; body {}", a.body);
    let error = &a.body["error"];
    assert_eq!(error["message"], message, "{what}: message in {}", a.body);
    let code = error["code"].as_str().unwrap_or("");
    assert!(
        !code.is_empty()
            && code.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && code.chars().all(|c| c.is_ascii_alphanumeric()),
        "{what}: a camelCase code, got {code:?}"
    );
    assert!(
        error.get("field").is_none(),
        "{what}: no field in {}",
        a.body
    );
    assert!(
        a.body.get("data").is_none(),
        "{what}: no data in {}",
        a.body
    );
}

// ------------------------------------------------------------ the version

#[actix_web::test]
async fn branch_versions_reads_the_row_branch_tx_writes() {
    let db = TestDb::start().await;
    let versions = BranchVersions::new(db.pool.clone());
    let b01 = BranchId::new("XC-B01");
    assert_eq!(versions.current(&b01).await.expect("read"), 0, "no row yet");
    db.write("XC-B01").await;
    assert_eq!(versions.current(&b01).await.expect("read"), 1);
    db.write("XC-B01").await;
    db.write("XC-B02").await;
    assert_eq!(versions.current(&b01).await.expect("read"), 2);
    assert_eq!(
        versions
            .current(&BranchId::new("XC-B02"))
            .await
            .expect("read"),
        1
    );
    db.finish().await;
}

#[actix_web::test]
async fn an_equal_since_answers_unchanged_without_loading_the_view() {
    let db = TestDb::start().await;
    let views = Views::new();
    db.write("XC-B01").await;

    let first = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-count?branch=XC-B01",
    )
    .await;
    is_changed(&first, 1, "the first read");
    assert_eq!(views.loads(), 1);

    for _ in 0..3 {
        let again = get(
            &db,
            &views,
            "owner",
            "/api/v3/views/test-count?branch=XC-B01&since=1",
        )
        .await;
        is_unchanged(&again, 1, "since = the current version");
    }
    assert_eq!(views.loads(), 1, "load is not called while unchanged");
    db.finish().await;
}

#[actix_web::test]
async fn a_write_through_branch_tx_makes_the_same_since_changed() {
    let db = TestDb::start().await;
    let views = Views::new();
    db.write("XC-B01").await;
    let uri = "/api/v3/views/test-count?branch=XC-B01&since=1";
    is_unchanged(&get(&db, &views, "owner", uri).await, 1, "before the write");

    db.write("XC-B01").await;
    let after = get(&db, &views, "owner", uri).await;
    is_changed(&after, 2, "after one write");
    assert_eq!(after.body["data"]["branch"], "XC-B01");
    assert_eq!(views.loads(), 1);

    // a write on another branch does not move this one
    db.write("XC-B02").await;
    let other = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-count?branch=XC-B01&since=2",
    )
    .await;
    is_unchanged(&other, 2, "after a write on XC-B02");
    db.finish().await;
}

#[actix_web::test]
async fn no_since_always_answers_with_data() {
    let db = TestDb::start().await;
    let views = Views::new();
    db.write("XC-B01").await;
    for n in 1..=3 {
        let a = get(
            &db,
            &views,
            "owner",
            "/api/v3/views/test-count?branch=XC-B01",
        )
        .await;
        is_changed(&a, 1, "no since");
        assert_eq!(views.loads(), n);
    }
    db.finish().await;
}

#[actix_web::test]
async fn a_branch_never_written_is_version_zero() {
    let db = TestDb::start().await;
    let views = Views::new();
    let a = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-count?branch=XC-NEW",
    )
    .await;
    is_changed(&a, 0, "a branch with no branch_versions row");
    let b = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-count?branch=XC-NEW&since=0",
    )
    .await;
    is_unchanged(&b, 0, "since=0 on that branch");
    db.finish().await;
}

#[actix_web::test]
async fn the_owner_without_a_branch_is_answered_for_main() {
    let db = TestDb::start().await;
    let views = Views::new();
    for uri in [
        "/api/v3/views/test-count",
        "/api/v3/views/test-count?branch=",
    ] {
        let a = get(&db, &views, "owner", uri).await;
        is_changed(&a, 0, uri);
        assert_eq!(a.body["data"]["branch"], "MAIN", "{uri}");
    }
    db.finish().await;
}

#[actix_web::test]
async fn a_staff_caller_naming_another_branch_is_answered_for_their_own() {
    let db = TestDb::start().await;
    let views = Views::new();
    db.write("XC-B01").await;
    for _ in 0..3 {
        db.write("XC-B02").await;
    }
    let a = get(
        &db,
        &views,
        "staff:FRONT:XC-B01",
        "/api/v3/views/test-count?branch=XC-B02",
    )
    .await;
    is_changed(&a, 1, "FRONT of XC-B01 naming XC-B02");
    assert_eq!(a.body["data"]["branch"], "XC-B01");
    assert_eq!(a.body["data"]["who"], "employee-7");

    // and its since is compared with its own branch's version
    let b = get(
        &db,
        &views,
        "staff:FRONT:XC-B01",
        "/api/v3/views/test-count?branch=XC-B02&since=1",
    )
    .await;
    is_unchanged(&b, 1, "since of XC-B01");
    db.finish().await;
}

// ------------------------------------------------------------ who may read

#[actix_web::test]
async fn a_role_outside_the_views_row_gets_the_policys_refusal() {
    let db = TestDb::start().await;
    let views = Views::new();
    // (StaffRpc, adminDeskConfig): FRONT only (policy R8)
    let ok = get(
        &db,
        &views,
        "staff:FRONT:XC-B01",
        "/api/v3/views/test-desk-config",
    )
    .await;
    is_changed(&ok, 0, "FRONT");
    let refused = get(
        &db,
        &views,
        "staff:BACKEND:XC-B01",
        "/api/v3/views/test-desk-config",
    )
    .await;
    is_refusal(&refused, 400, "ใช้ได้เฉพาะ Front", "BACKEND");
    let nobody = get(&db, &views, "", "/api/v3/views/test-count").await;
    is_refusal(&nobody, 401, "กรุณาเข้าสู่ระบบ", "nobody on Rpc");
    assert_eq!(views.loads(), 0, "a refused caller loads nothing");
    db.finish().await;
}

/// §9: a view whose INHERITS is (Rpc, "adminSaveRate") refuses exactly the
/// callers `Policy::allow` refuses for that pair, and so for the staff path.
#[actix_web::test]
async fn a_view_refuses_exactly_whom_the_policy_refuses_for_its_pair() {
    let db = TestDb::start().await;
    let views = Views::new();
    let mut allowed = 0;
    let mut refused = 0;
    for (view, path, action) in [
        ("test-save-rate", EntryPath::Rpc, "adminSaveRate"),
        ("test-desk-config", EntryPath::StaffRpc, "adminDeskConfig"),
    ] {
        for who in every_caller() {
            let a = get(&db, &views, &who, &format!("/api/v3/views/{view}")).await;
            let what = format!("{view} as {who:?}");
            let actor = actor_on(path, &who);
            match Policy::allow(actor.as_ref(), path, &Action::new(action)) {
                Ok(()) => {
                    allowed += 1;
                    is_changed(&a, 0, &what);
                    assert_eq!(
                        a.body["data"]["who"],
                        actor.expect("allowed means signed in").id(),
                        "{what}: answered for the actor of its entry path"
                    );
                }
                Err(e) => {
                    refused += 1;
                    is_refusal(&a, e.status(), &e.to_string(), &what);
                }
            }
        }
    }
    assert!(allowed >= 4, "allowed: {allowed}");
    assert!(refused >= 10, "refused: {refused}");
    db.finish().await;
}

// ------------------------------------------------------------ bad requests

#[actix_web::test]
async fn an_ill_formed_branch_since_or_limit_is_refused_as_a_bad_request() {
    let db = TestDb::start().await;
    let views = Views::new();
    for uri in [
        "/api/v3/views/test-count?branch=XC_B01",
        "/api/v3/views/test-count?branch=bad%20name",
        "/api/v3/views/test-count?since=abc",
        "/api/v3/views/test-count?since=-1",
        "/api/v3/views/test-paged?limit=0",
        "/api/v3/views/test-paged?limit=101",
        "/api/v3/views/test-paged?limit=x",
    ] {
        let a = get(&db, &views, "owner", uri).await;
        is_refusal(&a, 400, "คำขอไม่ถูกต้อง", uri);
    }
    assert_eq!(views.loads(), 0);
    assert!(views.paging.asked.lock().expect("lock").is_empty());
    db.finish().await;
}

#[actix_web::test]
async fn every_view_answer_is_no_store() {
    let db = TestDb::start().await;
    let views = Views::new();
    for (who, uri) in [
        ("owner", "/api/v3/views/test-count"),
        ("owner", "/api/v3/views/test-count?since=0"),
        ("", "/api/v3/views/test-count"),
        ("owner", "/api/v3/views/test-paged"),
    ] {
        let a = get(&db, &views, who, uri).await;
        assert_eq!(
            a.cache_control.as_deref(),
            Some("no-store"),
            "{uri} as {who:?}"
        );
    }
    db.finish().await;
}

// ------------------------------------------------------------ paged views

#[actix_web::test]
async fn a_paged_view_is_asked_for_before_and_limit_and_answers_its_rows() {
    let db = TestDb::start().await;
    let views = Views::new();
    db.write("MAIN").await;
    let a = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-paged?before=r9&limit=2",
    )
    .await;
    is_changed(&a, 1, "an older page");
    assert_eq!(
        a.body["data"],
        serde_json::json!([{ "id": "r2" }, { "id": "r1" }])
    );
    let b = get(&db, &views, "owner", "/api/v3/views/test-paged?limit=100").await;
    is_changed(&b, 1, "the first page, limit 100");
    let c = get(&db, &views, "owner", "/api/v3/views/test-paged").await;
    is_changed(&c, 1, "the first page, no limit");
    let asked = views.paging.asked.lock().expect("lock").clone();
    assert_eq!(asked[0], (Some("r9".to_string()), 2));
    assert_eq!(asked[1], (None, 100));
    assert!(asked[2].0.is_none());
    assert!(
        (1..=100).contains(&asked[2].1),
        "a default limit within 1..=100: {}",
        asked[2].1
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_paged_first_page_with_an_equal_since_is_unchanged() {
    let db = TestDb::start().await;
    let views = Views::new();
    db.write("MAIN").await;
    let a = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-paged?since=1&limit=20",
    )
    .await;
    is_unchanged(&a, 1, "first page, since = version");
    assert!(views.paging.asked.lock().expect("lock").is_empty());
    db.finish().await;
}

// ------------------------------------------------------------ the me view

#[actix_web::test]
async fn the_me_view_is_served_by_the_route_table_with_no_other_edit() {
    let db = TestDb::start().await;
    let views = Views::new();

    let owner = get(&db, &views, "owner", "/api/v3/views/me").await;
    let owner_version = owner.body["version"].as_i64().unwrap_or(0);
    assert_ne!(
        owner_version, 0,
        "me as the owner: its own version, not the branch's 0 (#492)"
    );
    is_changed(&owner, owner_version, "me as the owner");
    assert_eq!(
        owner.body["data"],
        serde_json::json!({ "kind": "owner", "id": "owner-1", "role": "OWNER", "branch": "MAIN", "name": null, "username": null })
    );

    let named = get(&db, &views, "owner", "/api/v3/views/me?branch=XC-B02").await;
    assert_eq!(named.body["data"]["branch"], "XC-B02");

    let staff = get(
        &db,
        &views,
        "staff:RATES:XC-B01",
        "/api/v3/views/me?branch=XC-B02",
    )
    .await;
    let staff_version = staff.body["version"].as_i64().unwrap_or(0);
    assert_ne!(
        staff_version, 0,
        "me as RATES staff: its own version, not the branch's 0 (#492)"
    );
    is_changed(&staff, staff_version, "me as RATES staff");
    assert_eq!(
        staff.body["data"],
        serde_json::json!({ "kind": "staff", "id": "employee-7", "role": "RATES", "branch": "XC-B01", "name": null, "username": null })
    );

    let nobody = get(&db, &views, "", "/api/v3/views/me").await;
    is_refusal(&nobody, 401, "กรุณาเข้าสู่ระบบ", "me as nobody");
    db.finish().await;
}
