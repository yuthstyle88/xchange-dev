//! Specification tests for #500: the v3 view core refuses a branch that is not in
//! `branches`, once, for every view (docs/design/xchange-v3-web.md §5.1, §12.1).
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! - An owner naming a well-formed branch id that is not in `branches` gets
//!   `400 {"error": {"code": "noSuchBranch", "field": "branch", "message":
//!   "ไม่พบสาขาที่เลือก กรุณารีเฟรช"}}`, `no-store`, from every view; `load`,
//!   `page` and `own_version` are not called (B1).
//! - The check runs in the core after `Policy::branch` and before the filter,
//!   the version and the `since` shortcut; `since` never skips it (B2).
//! - Every answer for an existing branch is unchanged (B3); an ill-formed
//!   branch, a signed-out caller and the staff forcing of `Policy::branch` are
//!   unchanged and come first (B4).
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
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use ts_rs::TS;
use xc_auth::domain::access::Actor as AccessActor;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http::v3::VIEWS;
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::http::v3::view::{
    BranchView, PagedView, RowId, ViewEndpoint, ViewFilter,
};
use xchange::presentation::pages::{SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";

/// A well-formed branch id that is in no `branches` row.
const GONE: &str = "XC-GONE";
/// A branch this file inserts into `branches`.
const B01: &str = "XC-B01";
/// The compat routes' sentence for this mistake, byte for byte
/// (`xc_rate::domain::validation::NO_SUCH_BRANCH`).
const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";

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
                 The v3 unknown-branch tests need a real PostgreSQL and do not skip. Point \
                 it at an existing database whose name ends in `_test`, e.g.\n\
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
        let schema = unique("v3unknown");
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
        // The migrations insert MAIN; this file also uses XC-B01. XC-GONE is never inserted.
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01')")
            .execute(&pool)
            .await
            .expect("insert XC-B01 into branches");
        Self { pool, schema }
    }

    /// One committed `BranchTx` run on `branch` that writes nothing: the
    /// version rises, and a `branch_versions` row exists for it — whether or
    /// not the branch is in `branches`.
    async fn write(&self, branch: &str) {
        BranchTx::new(Transactor::new(self.pool.clone()))
            .run(&BranchId::new(branch), |_tx| {
                Box::pin(async move { AppResult::Ok(()) })
            })
            .await
            .expect("a BranchTx run commits");
    }

    async fn branch_rows(&self, id: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM branches WHERE id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("count branches")
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
/// `staff:<ROLE>:<branch>`, or absent.
struct HeaderSessions;

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
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = Role::parse(parts.next().expect("a role")).expect("a known role");
    let branch = parts.next().expect("a branch");
    Sessions {
        owner: None,
        staff: Actor::staff("employee-7", role, BranchId::new(branch)),
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

// ------------------------------------------------------------ test views

/// A filter with one typed key `n`: `n=abc` does not parse (`invalidRequest`,
/// field `n`), and `n=13` parses but `check` refuses it.
#[derive(Debug, Default, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
struct ProbeFilter {
    n: Option<u32>,
}

impl ViewFilter for ProbeFilter {
    fn check(&self) -> Result<(), V3Error> {
        if self.n == Some(13) {
            return Err(V3Error::new(
                "probeRefused",
                Some("n"),
                "probe refused",
                StatusCode::BAD_REQUEST,
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct Echo {
    branch: String,
}

/// A versioned branch view that counts its loads.
struct Counting {
    loads: AtomicUsize,
}

impl BranchView for Counting {
    const NAME: &'static str = "test-ub-count";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Filter = ProbeFilter;
    type Data = Echo;

    async fn load(
        &self,
        branch: &BranchId,
        _who: &AccessActor,
        _f: &ProbeFilter,
    ) -> AppResult<Echo> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        Ok(Echo {
            branch: branch.as_str().to_owned(),
        })
    }
}

/// The version this view answers instead of the branch's (design §12.16).
const OWN_VERSION: i64 = 7;

/// A branch view with its own version; counts both `own_version` and `load`.
struct OwnVersioned {
    asked: AtomicUsize,
    loads: AtomicUsize,
}

impl BranchView for OwnVersioned {
    const NAME: &'static str = "test-ub-own";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Filter = ProbeFilter;
    type Data = Echo;

    async fn own_version(&self, _branch: &BranchId, _who: &AccessActor) -> AppResult<Option<i64>> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        Ok(Some(OWN_VERSION))
    }

    async fn load(
        &self,
        branch: &BranchId,
        _who: &AccessActor,
        _f: &ProbeFilter,
    ) -> AppResult<Echo> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        Ok(Echo {
            branch: branch.as_str().to_owned(),
        })
    }
}

/// A `VERSIONED = false` branch view that counts its loads.
struct Unversioned {
    loads: AtomicUsize,
}

impl BranchView for Unversioned {
    const NAME: &'static str = "test-ub-unversioned";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    const VERSIONED: bool = false;
    type Filter = ProbeFilter;
    type Data = Echo;

    async fn load(
        &self,
        branch: &BranchId,
        _who: &AccessActor,
        _f: &ProbeFilter,
    ) -> AppResult<Echo> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        Ok(Echo {
            branch: branch.as_str().to_owned(),
        })
    }
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct Row {
    branch: String,
}

/// A paged view that records the `before` of every page it was asked for.
struct Paging {
    asked: Mutex<Vec<Option<RowId>>>,
}

impl PagedView for Paging {
    const NAME: &'static str = "test-ub-paged";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Filter = ProbeFilter;
    type Row = Row;

    async fn page(
        &self,
        branch: &BranchId,
        _who: &AccessActor,
        _f: &ProbeFilter,
        before: Option<RowId>,
        _limit: u16,
    ) -> AppResult<Vec<Row>> {
        self.asked.lock().expect("not poisoned").push(before);
        Ok(vec![Row {
            branch: branch.as_str().to_owned(),
        }])
    }
}

struct Views {
    counting: Arc<Counting>,
    own: Arc<OwnVersioned>,
    unversioned: Arc<Unversioned>,
    paging: Arc<Paging>,
}

impl Views {
    fn new() -> Self {
        Self {
            counting: Arc::new(Counting {
                loads: AtomicUsize::new(0),
            }),
            own: Arc::new(OwnVersioned {
                asked: AtomicUsize::new(0),
                loads: AtomicUsize::new(0),
            }),
            unversioned: Arc::new(Unversioned {
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

    fn own_asked(&self) -> usize {
        self.own.asked.load(Ordering::SeqCst)
    }

    fn own_loads(&self) -> usize {
        self.own.loads.load(Ordering::SeqCst)
    }

    fn unversioned_loads(&self) -> usize {
        self.unversioned.loads.load(Ordering::SeqCst)
    }

    fn pages(&self) -> usize {
        self.paging.asked.lock().expect("lock").len()
    }
}

struct Answer {
    status: StatusCode,
    cache_control: Option<String>,
    body: Value,
}

/// `GET uri` as `who`, on the whole application (`bootstrap::configure`, with
/// every generated view) and the test views registered beside it.
async fn get(db: &TestDb, views: &Views, who: &str, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let counting = views.counting.clone();
    let own = views.own.clone();
    let unversioned = views.unversioned.clone();
    let paging = views.paging.clone();
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| {
                ViewEndpoint::register(cfg, counting);
                ViewEndpoint::register(cfg, own);
                ViewEndpoint::register(cfg, unversioned);
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

/// B1: exactly the refusal, nothing else in the body, and `no-store`.
fn is_no_such_branch(a: &Answer, what: &str) {
    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "{what}: 400 for a branch not in `branches`; body {}",
        a.body
    );
    assert_eq!(
        a.body,
        json!({ "error": { "code": "noSuchBranch", "field": "branch", "message": NO_SUCH_BRANCH } }),
        "{what}: exactly the noSuchBranch refusal"
    );
    assert_eq!(
        a.cache_control.as_deref(),
        Some("no-store"),
        "{what}: no-store"
    );
}

fn is_unchanged(a: &Answer, version: i64, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: status; body {}", a.body);
    assert_eq!(
        a.body,
        json!({ "version": version, "changed": false }),
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

// ------------------------------------------------------------ a branch view

/// OR-4b (#767, D-4, design §2 and §02 "Server ตรวจ … branch scope"): an employee who names a
/// branch outside their scope is refused 403 `outOfScope`, where v3 answered their own branch.
fn is_out_of_scope(a: &Answer, what: &str) {
    assert_eq!(a.status.as_u16(), 403, "{what}: status; body {}", a.body);
    assert_eq!(
        a.body["error"]["code"],
        serde_json::json!("outOfScope"),
        "{what}: {}",
        a.body
    );
}

#[actix_web::test]
async fn a_branch_view_refuses_an_unknown_branch_without_loading() {
    let db = TestDb::start().await;
    let views = Views::new();
    assert_eq!(
        db.branch_rows(GONE).await,
        0,
        "XC-GONE is in no branches row"
    );

    for uri in [
        format!("/api/v3/views/test-ub-count?branch={GONE}"),
        format!("/api/v3/views/test-ub-count?since=3&branch={GONE}"),
    ] {
        let a = get(&db, &views, "owner", &uri).await;
        is_no_such_branch(&a, &uri);
    }
    assert_eq!(views.loads(), 0, "load is not called for an unknown branch");

    // B3: an existing branch is answered as before, so the view itself works
    db.write(B01).await;
    let known = get(
        &db,
        &views,
        "owner",
        &format!("/api/v3/views/test-ub-count?branch={B01}"),
    )
    .await;
    is_changed(&known, 1, "XC-B01");
    assert_eq!(known.body["data"]["branch"], B01);
    let main = get(&db, &views, "owner", "/api/v3/views/test-ub-count").await;
    is_changed(&main, 0, "no branch: the owner's MAIN");
    assert_eq!(main.body["data"]["branch"], "MAIN");
    assert_eq!(views.loads(), 2);
    db.finish().await;
}

// ------------------------------------------------------------ a paged view

#[actix_web::test]
async fn a_paged_view_refuses_an_unknown_branch_with_and_without_before() {
    let db = TestDb::start().await;
    let views = Views::new();
    for uri in [
        format!("/api/v3/views/test-ub-paged?branch={GONE}"),
        format!("/api/v3/views/test-ub-paged?branch={GONE}&limit=5"),
        format!("/api/v3/views/test-ub-paged?branch={GONE}&before=r9&limit=5"),
        format!("/api/v3/views/test-ub-paged?branch={GONE}&before=r9&since=0"),
    ] {
        let a = get(&db, &views, "owner", &uri).await;
        is_no_such_branch(&a, &uri);
    }
    assert_eq!(views.pages(), 0, "page is not called for an unknown branch");

    // B3: an existing branch is answered as before
    db.write(B01).await;
    let first = get(
        &db,
        &views,
        "owner",
        &format!("/api/v3/views/test-ub-paged?branch={B01}"),
    )
    .await;
    is_changed(&first, 1, "XC-B01, the first page");
    assert_eq!(first.body["data"], json!([{ "branch": B01 }]));
    let older = get(
        &db,
        &views,
        "owner",
        &format!("/api/v3/views/test-ub-paged?branch={B01}&before=r9&limit=5"),
    )
    .await;
    is_changed(&older, 1, "XC-B01, an older page");
    let same = get(
        &db,
        &views,
        "owner",
        &format!("/api/v3/views/test-ub-paged?branch={B01}&since=1"),
    )
    .await;
    is_unchanged(&same, 1, "XC-B01, the first page at its version");
    let asked = views.paging.asked.lock().expect("lock").clone();
    assert_eq!(asked, vec![None, Some("r9".to_string())]);
    db.finish().await;
}

// ------------------------------------------------------------ own_version

#[actix_web::test]
async fn a_view_with_its_own_version_refuses_an_unknown_branch_before_asking_for_it() {
    let db = TestDb::start().await;
    let views = Views::new();
    for uri in [
        format!("/api/v3/views/test-ub-own?branch={GONE}"),
        // `since` equal to what own_version would answer is still refused
        format!("/api/v3/views/test-ub-own?branch={GONE}&since={OWN_VERSION}"),
        format!("/api/v3/views/test-ub-own?branch={GONE}&since=0"),
    ] {
        let a = get(&db, &views, "owner", &uri).await;
        is_no_such_branch(&a, &uri);
    }
    assert_eq!(
        views.own_asked(),
        0,
        "own_version is not read for a branch that does not exist"
    );
    assert_eq!(views.own_loads(), 0, "load is not called");

    // B3: an existing branch is answered as before, own version and all
    let known = get(
        &db,
        &views,
        "owner",
        &format!("/api/v3/views/test-ub-own?branch={B01}"),
    )
    .await;
    is_changed(&known, OWN_VERSION, "XC-B01");
    let same = get(
        &db,
        &views,
        "owner",
        &format!("/api/v3/views/test-ub-own?branch={B01}&since={OWN_VERSION}"),
    )
    .await;
    is_unchanged(&same, OWN_VERSION, "XC-B01 at its own version");
    assert_eq!(views.own_asked(), 2);
    assert_eq!(views.own_loads(), 1);
    db.finish().await;
}

// ------------------------------------------------------------ VERSIONED = false

#[actix_web::test]
async fn an_unversioned_view_refuses_an_unknown_branch() {
    let db = TestDb::start().await;
    let views = Views::new();
    for uri in [
        format!("/api/v3/views/test-ub-unversioned?branch={GONE}"),
        format!("/api/v3/views/test-ub-unversioned?branch={GONE}&since=0"),
    ] {
        let a = get(&db, &views, "owner", &uri).await;
        is_no_such_branch(&a, &uri);
    }
    assert_eq!(views.unversioned_loads(), 0, "load is not called");

    // B3: an existing branch is loaded every time, `since` or not
    for uri in [
        format!("/api/v3/views/test-ub-unversioned?branch={B01}"),
        format!("/api/v3/views/test-ub-unversioned?branch={B01}&since=0"),
    ] {
        let a = get(&db, &views, "owner", &uri).await;
        is_changed(&a, 0, &uri);
    }
    assert_eq!(views.unversioned_loads(), 2);
    db.finish().await;
}

// ------------------------------------------------------------ since never skips the check

#[actix_web::test]
async fn since_at_an_unknown_branchs_version_is_refused_not_unchanged() {
    let db = TestDb::start().await;
    let views = Views::new();
    // an unknown branch has version 0 — `since=0` must not answer changed:false
    for uri in [
        format!("/api/v3/views/test-ub-count?branch={GONE}&since=0"),
        format!("/api/v3/views/test-ub-paged?branch={GONE}&since=0"),
    ] {
        let a = get(&db, &views, "owner", &uri).await;
        is_no_such_branch(&a, &uri);
    }

    // a branch_versions row is not a branch: BranchTx writes one for any id
    db.write(GONE).await;
    db.write(GONE).await;
    assert_eq!(db.branch_rows(GONE).await, 0, "still in no branches row");
    for uri in [
        format!("/api/v3/views/test-ub-count?branch={GONE}"),
        format!("/api/v3/views/test-ub-count?branch={GONE}&since=2"),
        format!("/api/v3/views/test-ub-count?branch={GONE}&since=0"),
        format!("/api/v3/views/test-ub-paged?branch={GONE}"),
        format!("/api/v3/views/test-ub-paged?branch={GONE}&since=2"),
    ] {
        let a = get(&db, &views, "owner", &uri).await;
        is_no_such_branch(&a, &uri);
    }
    assert_eq!(views.loads(), 0);
    assert_eq!(views.pages(), 0);

    // B3: on an existing branch `since` at its version is still unchanged
    db.write(B01).await;
    db.write(B01).await;
    for uri in [
        format!("/api/v3/views/test-ub-count?branch={B01}&since=2"),
        format!("/api/v3/views/test-ub-paged?branch={B01}&since=2"),
    ] {
        let a = get(&db, &views, "owner", &uri).await;
        is_unchanged(&a, 2, &uri);
    }
    assert_eq!(views.loads(), 0, "unchanged loads nothing");
    assert_eq!(views.pages(), 0, "unchanged pages nothing");
    db.finish().await;
}

// ------------------------------------------------------------ staff forcing comes first

#[actix_web::test]
async fn a_staff_caller_naming_an_unknown_branch_is_answered_for_their_own() {
    let db = TestDb::start().await;
    let views = Views::new();
    db.write(B01).await;
    let staff = format!("staff:FRONT:{B01}");
    // OR-4b (D-4): naming XC-GONE is refused and nothing is loaded; naming its own branch, it is
    // answered for it
    for view in ["test-ub-count", "test-ub-paged", "test-ub-own"] {
        let refused = get(
            &db,
            &views,
            &staff,
            &format!("/api/v3/views/{view}?branch={GONE}"),
        )
        .await;
        is_out_of_scope(&refused, view);
    }
    assert_eq!(views.loads(), 0, "a refused view loads nothing");

    let a = get(
        &db,
        &views,
        &staff,
        &format!("/api/v3/views/test-ub-count?branch={B01}"),
    )
    .await;
    is_changed(&a, 1, "FRONT of XC-B01 naming its own");
    assert_eq!(a.body["data"]["branch"], B01);

    let b = get(
        &db,
        &views,
        &staff,
        &format!("/api/v3/views/test-ub-count?branch={B01}&since=1"),
    )
    .await;
    is_unchanged(&b, 1, "since of XC-B01");

    let p = get(
        &db,
        &views,
        &staff,
        &format!("/api/v3/views/test-ub-paged?branch={B01}"),
    )
    .await;
    is_changed(&p, 1, "paged: FRONT of XC-B01 naming its own");
    assert_eq!(p.body["data"], json!([{ "branch": B01 }]));

    let o = get(
        &db,
        &views,
        &staff,
        &format!("/api/v3/views/test-ub-own?branch={B01}&since={OWN_VERSION}"),
    )
    .await;
    is_unchanged(
        &o,
        OWN_VERSION,
        "own version: FRONT of XC-B01 naming its own",
    );
    assert_eq!(views.loads(), 1);
    db.finish().await;
}

// ------------------------------------------------------------ the branch before the filter

#[actix_web::test]
async fn the_branch_is_checked_before_the_filter() {
    let db = TestDb::start().await;
    let views = Views::new();
    for view in [
        "test-ub-count",
        "test-ub-own",
        "test-ub-unversioned",
        "test-ub-paged",
    ] {
        // a filter that does not parse, and one that `check` refuses
        for filter in ["n=abc", "n=13"] {
            let unknown = get(
                &db,
                &views,
                "owner",
                &format!("/api/v3/views/{view}?{filter}&branch={GONE}"),
            )
            .await;
            is_no_such_branch(&unknown, &format!("{view} {filter} on XC-GONE"));
        }

        // B3: on an existing branch the filter is refused as before
        let bad = get(
            &db,
            &views,
            "owner",
            &format!("/api/v3/views/{view}?n=abc&branch={B01}"),
        )
        .await;
        assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{view}: {}", bad.body);
        assert_eq!(
            bad.body,
            json!({ "error": { "code": "invalidRequest", "field": "n", "message": "คำขอไม่ถูกต้อง" } }),
            "{view}: n=abc on XC-B01"
        );
        let refused = get(
            &db,
            &views,
            "owner",
            &format!("/api/v3/views/{view}?n=13&branch={B01}"),
        )
        .await;
        assert_eq!(
            refused.body,
            json!({ "error": { "code": "probeRefused", "field": "n", "message": "probe refused" } }),
            "{view}: n=13 on XC-B01"
        );
    }
    assert_eq!(views.loads(), 0);
    assert_eq!(views.own_asked(), 0);
    assert_eq!(views.own_loads(), 0);
    assert_eq!(views.unversioned_loads(), 0);
    assert_eq!(views.pages(), 0);
    db.finish().await;
}

// ------------------------------------------------------------ earlier refusals stay earlier

#[actix_web::test]
async fn an_ill_formed_branch_and_a_signed_out_caller_are_refused_as_before() {
    let db = TestDb::start().await;
    let views = Views::new();
    for view in ["test-ub-count", "test-ub-paged"] {
        let ill = get(
            &db,
            &views,
            "owner",
            &format!("/api/v3/views/{view}?branch=a%2Fb"),
        )
        .await;
        assert_eq!(ill.status, StatusCode::BAD_REQUEST, "{view}: {}", ill.body);
        assert_eq!(
            ill.body,
            json!({ "error": { "code": "invalidRequest", "message": "คำขอไม่ถูกต้อง" } }),
            "{view}: an ill-formed branch, no field"
        );

        let nobody = get(
            &db,
            &views,
            "",
            &format!("/api/v3/views/{view}?branch={GONE}"),
        )
        .await;
        assert_eq!(
            nobody.status,
            StatusCode::UNAUTHORIZED,
            "{view}: {}",
            nobody.body
        );
        assert_eq!(nobody.body["error"]["code"], "signIn", "{view}");
        assert_eq!(nobody.body["error"]["message"], "กรุณาเข้าสู่ระบบ", "{view}");
    }
    assert_eq!(views.loads(), 0);
    assert_eq!(views.pages(), 0);
    db.finish().await;
}

// ------------------------------------------------------------ branches unreadable

/// §8: when `branches` cannot be read the answer is `500 serverError`, never
/// `noSuchBranch` — a fault is not "no such branch". Sign-in in this file reads
/// no table, so hiding `branches` breaks nothing but the check.
#[actix_web::test]
async fn an_unreadable_branches_table_is_a_server_error_not_no_such_branch() {
    let db = TestDb::start().await;
    let views = Views::new();
    sqlx::query("ALTER TABLE branches RENAME TO hidden_branches")
        .execute(&db.pool)
        .await
        .expect("hide branches");
    for uri in [
        format!("/api/v3/views/test-ub-count?branch={B01}"),
        format!("/api/v3/views/test-ub-count?branch={GONE}"),
        format!("/api/v3/views/test-ub-paged?branch={B01}"),
        format!("/api/v3/views/test-ub-paged?branch={GONE}"),
    ] {
        let a = get(&db, &views, "owner", &uri).await;
        assert_eq!(
            a.status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{uri}: {}",
            a.body
        );
        assert_eq!(
            a.body,
            json!({ "error": { "code": "serverError", "message": "เซิร์ฟเวอร์ขัดข้อง" } }),
            "{uri}"
        );
    }
    assert_eq!(views.loads(), 0, "nothing is loaded when the check fails");
    assert_eq!(views.pages(), 0);
    sqlx::query("ALTER TABLE hidden_branches RENAME TO branches")
        .execute(&db.pool)
        .await
        .expect("restore branches");
    db.finish().await;
}

// ------------------------------------------------------------ every generated view

/// B1/B5: every view of the generated table answers an unknown branch the same
/// way, from the core. Each view is asked by the owner naming XC-GONE and by a
/// staff member of each role whose own branch is XC-GONE; an answer is either
/// the policy's refusal (before the check) or noSuchBranch — never 200, never
/// 500 — and every view gives noSuchBranch to at least one caller.
#[actix_web::test]
async fn every_generated_view_refuses_an_unknown_branch() {
    let db = TestDb::start().await;
    let views = Views::new();
    assert!(VIEWS.len() >= 11, "the generated views: {}", VIEWS.len());
    let mut callers = vec!["owner".to_string()];
    for role in STAFF_ROLES {
        callers.push(format!("staff:{role}:{GONE}"));
    }
    for entry in VIEWS {
        let mut refused_as_unknown = 0;
        for who in &callers {
            let uri = format!("/api/v3/views/{}?branch={GONE}", entry.name);
            let a = get(&db, &views, who, &uri).await;
            let what = format!("{} as {who}", entry.name);
            let code = a.body["error"]["code"].as_str().unwrap_or("").to_owned();
            if code == "noSuchBranch" {
                is_no_such_branch(&a, &what);
                refused_as_unknown += 1;
            } else {
                assert!(
                    a.status.is_client_error()
                        && !code.is_empty()
                        && code != "serverError"
                        && code != "invalidRequest",
                    "{what}: either the policy's refusal or noSuchBranch, got {} {}",
                    a.status,
                    a.body
                );
            }
        }
        assert!(
            refused_as_unknown >= 1,
            "{}: no caller was answered noSuchBranch",
            entry.name
        );
    }
    db.finish().await;
}
