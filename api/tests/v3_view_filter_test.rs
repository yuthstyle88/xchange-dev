//! Specification tests for V0-C (#475; docs/design/xchange-v3-web.md §5.1, §12.1,
//! §12.16): a view carries a filter, `BranchView::VERSIONED`, and
//! `BranchView::own_version`.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! - `presentation::http::v3::view::ViewFilter`: `DeserializeOwned + TS + Default`,
//!   with `check(&self) -> Result<(), V3Error>` (default `Ok(())`).
//! - `BranchView` / `PagedView` gain `type Filter: ViewFilter;`, and `load` / `page`
//!   take `filter: &Self::Filter` after `who`.
//! - `BranchView::VERSIONED` (default `true`) and `BranchView::own_version(&self)`
//!   (default `Ok(None)`).
//! - `presentation::dto::v3::common::{NoFilter, Period}`: `NoFilter` is the filter
//!   of a view without one; `Period { pub from: String, pub to: String }` with
//!   `check(&self) -> Result<(), V3Error>`.
//! - The endpoint, in this order: sign-in and policy → paging (`branch`, `since`,
//!   `before`, `limit`, reserved) → the filter from the other keys (first
//!   occurrence of a key counts; unknown keys ignored; a value that does not parse
//!   is `400 invalidRequest` naming its key) → `check` → branch → version → the
//!   `since` shortcut (never for `VERSIONED = false`; `own_version`'s `Some(v)`
//!   replaces the branch version) → `load` / `page`.
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
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
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
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::dto::v3::common::{NoFilter, Period};
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::http::v3::view::{
    BranchView, PagedView, RowId, ViewEndpoint, ViewFilter,
};
use xchange::presentation::pages::{SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const INVALID: &str = "คำขอไม่ถูกต้อง";
const PERIOD_ORDER: &str = "วันที่เริ่มต้นต้องไม่เกินวันที่สิ้นสุด";

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
                 The v3 view filter tests need a real PostgreSQL and do not skip. Point it \
                 at an existing database whose name ends in `_test`, e.g.\n\
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
        let schema = unique("v3filter");
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

    async fn version(&self, branch: &str) -> i64 {
        BranchVersions::new(self.pool.clone())
            .current(&BranchId::new(branch))
            .await
            .expect("read branch_versions")
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

// ------------------------------------------------------------ filters

/// A filter with each shape a view's filter has: a text, a number, a period.
/// `minAmount` is camelCase on the wire, so a refusal must name the wire key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
struct Probe {
    q: String,
    min_amount: Option<u32>,
    from: String,
    to: String,
}

impl ViewFilter for Probe {
    fn check(&self) -> Result<(), V3Error> {
        Period {
            from: self.from.clone(),
            to: self.to.clone(),
        }
        .check()
    }
}

/// A filter that would catch the four paging keys if the endpoint let them
/// through. No real filter may declare them (§12.1); this one exists to prove
/// they never arrive.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, TS)]
#[serde(default)]
struct Reserved {
    branch: Option<String>,
    since: Option<String>,
    before: Option<String>,
    limit: Option<String>,
    q: String,
}

impl ViewFilter for Reserved {}

// ------------------------------------------------------------ test views

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct Echo {
    branch: String,
    who: String,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct Row {
    id: String,
}

/// A branch view that records the branch and the filter of every load.
struct Seen<F> {
    asked: Mutex<Vec<(String, F)>>,
}

impl<F: Clone> Seen<F> {
    fn new() -> Self {
        Self {
            asked: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, branch: &BranchId, who: &Actor, filter: &F) -> Echo {
        self.asked
            .lock()
            .expect("not poisoned")
            .push((branch.as_str().to_owned(), filter.clone()));
        Echo {
            branch: branch.as_str().to_owned(),
            who: who.id().to_owned(),
        }
    }

    fn asked(&self) -> Vec<(String, F)> {
        self.asked.lock().expect("not poisoned").clone()
    }
}

impl BranchView for Seen<Probe> {
    const NAME: &'static str = "test-filtered";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Data = Echo;
    type Filter = Probe;

    async fn load(&self, branch: &BranchId, who: &Actor, filter: &Probe) -> AppResult<Echo> {
        Ok(self.record(branch, who, filter))
    }
}

impl BranchView for Seen<Reserved> {
    const NAME: &'static str = "test-reserved";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Data = Echo;
    type Filter = Reserved;

    async fn load(&self, branch: &BranchId, who: &Actor, filter: &Reserved) -> AppResult<Echo> {
        Ok(self.record(branch, who, filter))
    }
}

/// What a paged view was asked: the branch, the filter, `before` and `limit`.
type PageAsk<F> = (String, F, Option<RowId>, u16);

/// A paged view that records every page it was asked for.
struct Pages<F> {
    asked: Mutex<Vec<PageAsk<F>>>,
}

impl<F: Clone> Pages<F> {
    fn new() -> Self {
        Self {
            asked: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, branch: &BranchId, filter: &F, before: Option<RowId>, limit: u16) -> Vec<Row> {
        self.asked.lock().expect("not poisoned").push((
            branch.as_str().to_owned(),
            filter.clone(),
            before,
            limit,
        ));
        vec![Row { id: "r1".into() }]
    }

    fn asked(&self) -> Vec<PageAsk<F>> {
        self.asked.lock().expect("not poisoned").clone()
    }
}

impl PagedView for Pages<Probe> {
    const NAME: &'static str = "test-filtered-paged";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Row = Row;
    type Filter = Probe;

    async fn page(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Probe,
        before: Option<RowId>,
        limit: u16,
    ) -> AppResult<Vec<Row>> {
        Ok(self.record(branch, filter, before, limit))
    }
}

impl PagedView for Pages<Reserved> {
    const NAME: &'static str = "test-reserved-paged";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Row = Row;
    type Filter = Reserved;

    async fn page(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Reserved,
        before: Option<RowId>,
        limit: u16,
    ) -> AppResult<Vec<Row>> {
        Ok(self.record(branch, filter, before, limit))
    }
}

/// A view without a filter, counting its loads.
struct Plain {
    loads: AtomicUsize,
}

impl BranchView for Plain {
    const NAME: &'static str = "test-plain";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Data = Echo;
    type Filter = NoFilter;

    async fn load(&self, branch: &BranchId, who: &Actor, _filter: &NoFilter) -> AppResult<Echo> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        Ok(Echo {
            branch: branch.as_str().to_owned(),
            who: who.id().to_owned(),
        })
    }
}

/// A view a branch version cannot describe (§12.1 `VERSIONED = false`).
struct Unversioned {
    loads: AtomicUsize,
}

impl BranchView for Unversioned {
    const NAME: &'static str = "test-unversioned";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    const VERSIONED: bool = false;
    type Data = Echo;
    type Filter = NoFilter;

    async fn load(&self, branch: &BranchId, who: &Actor, _filter: &NoFilter) -> AppResult<Echo> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        Ok(Echo {
            branch: branch.as_str().to_owned(),
            who: who.id().to_owned(),
        })
    }
}

/// A view with a version of its own (§12.16), set by the test.
struct OwnVersion {
    version: AtomicI64,
    loads: AtomicUsize,
}

impl BranchView for OwnVersion {
    const NAME: &'static str = "test-own-version";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Data = Echo;
    type Filter = NoFilter;

    async fn own_version(&self) -> AppResult<Option<i64>> {
        Ok(Some(self.version.load(Ordering::SeqCst)))
    }

    async fn load(&self, branch: &BranchId, who: &Actor, _filter: &NoFilter) -> AppResult<Echo> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        Ok(Echo {
            branch: branch.as_str().to_owned(),
            who: who.id().to_owned(),
        })
    }
}

struct Views {
    filtered: Arc<Seen<Probe>>,
    reserved: Arc<Seen<Reserved>>,
    filtered_pages: Arc<Pages<Probe>>,
    reserved_pages: Arc<Pages<Reserved>>,
    plain: Arc<Plain>,
    unversioned: Arc<Unversioned>,
    own: Arc<OwnVersion>,
}

impl Views {
    fn new() -> Self {
        Self {
            filtered: Arc::new(Seen::new()),
            reserved: Arc::new(Seen::new()),
            filtered_pages: Arc::new(Pages::new()),
            reserved_pages: Arc::new(Pages::new()),
            plain: Arc::new(Plain {
                loads: AtomicUsize::new(0),
            }),
            unversioned: Arc::new(Unversioned {
                loads: AtomicUsize::new(0),
            }),
            own: Arc::new(OwnVersion {
                version: AtomicI64::new(7),
                loads: AtomicUsize::new(0),
            }),
        }
    }

    /// Nothing reached a view's `load` or `page`.
    fn nothing_loaded(&self, what: &str) {
        assert!(
            self.filtered.asked().is_empty(),
            "{what}: test-filtered loaded"
        );
        assert!(
            self.reserved.asked().is_empty(),
            "{what}: test-reserved loaded"
        );
        assert!(
            self.filtered_pages.asked().is_empty(),
            "{what}: test-filtered-paged paged"
        );
        assert!(
            self.reserved_pages.asked().is_empty(),
            "{what}: test-reserved-paged paged"
        );
        assert_eq!(
            self.plain.loads.load(Ordering::SeqCst),
            0,
            "{what}: test-plain"
        );
        assert_eq!(
            self.unversioned.loads.load(Ordering::SeqCst),
            0,
            "{what}: test-unversioned"
        );
        assert_eq!(
            self.own.loads.load(Ordering::SeqCst),
            0,
            "{what}: test-own-version"
        );
    }
}

#[derive(Debug)]
struct Answer {
    status: StatusCode,
    body: Value,
}

/// `GET uri` as `who`, on the whole application (`bootstrap::configure`) with
/// the test views registered beside it, its state on `pool`.
async fn get_on(pool: PgPool, views: &Views, who: &str, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), pool);
    let filtered = views.filtered.clone();
    let reserved = views.reserved.clone();
    let filtered_pages = views.filtered_pages.clone();
    let reserved_pages = views.reserved_pages.clone();
    let plain = views.plain.clone();
    let unversioned = views.unversioned.clone();
    let own = views.own.clone();
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| {
                ViewEndpoint::register(cfg, filtered);
                ViewEndpoint::register(cfg, reserved);
                ViewEndpoint::register_paged(cfg, filtered_pages);
                ViewEndpoint::register_paged(cfg, reserved_pages);
                ViewEndpoint::register(cfg, plain);
                ViewEndpoint::register(cfg, unversioned);
                ViewEndpoint::register(cfg, own);
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
    let bytes = atest::read_body(resp).await;
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "GET {uri} as {who:?}: a JSON body ({e}): {:?}",
            String::from_utf8_lossy(&bytes)
        )
    });
    Answer { status, body }
}

async fn get(db: &TestDb, views: &Views, who: &str, uri: &str) -> Answer {
    get_on(db.pool.clone(), views, who, uri).await
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

/// A v3 refusal (§4.3): exactly `{"error": {code, field?, message}}`.
fn is_refused(a: &Answer, status: u16, code: &str, field: Option<&str>, message: &str, what: &str) {
    assert_eq!(a.status.as_u16(), status, "{what}: status; body {}", a.body);
    let expected = match field {
        Some(field) => json!({ "error": { "code": code, "field": field, "message": message } }),
        None => json!({ "error": { "code": code, "message": message } }),
    };
    assert_eq!(a.body, expected, "{what}: the refusal");
}

fn probe(q: &str, min_amount: Option<u32>, from: &str, to: &str) -> Probe {
    Probe {
        q: q.to_owned(),
        min_amount,
        from: from.to_owned(),
        to: to.to_owned(),
    }
}

// ------------------------------------------------------------ the shared types

#[test]
fn no_filter_reads_any_map_and_checks_nothing() {
    let filter: NoFilter =
        serde_json::from_value(json!({ "foo": "1", "q": "a", "from": "x" })).expect("any keys");
    assert!(filter.check().is_ok());
    assert!(NoFilter::default().check().is_ok());
}

#[test]
fn period_reads_from_and_to_each_open_when_absent() {
    let both: Period =
        serde_json::from_value(json!({ "from": "2026-10-01", "to": "2026-10-08" })).expect("both");
    assert_eq!(both.from, "2026-10-01");
    assert_eq!(both.to, "2026-10-08");
    let left: Period = serde_json::from_value(json!({ "from": "2026-10-01" })).expect("from only");
    assert_eq!(left.from, "2026-10-01");
    assert_eq!(left.to, "");
    let none: Period = serde_json::from_value(json!({})).expect("neither");
    assert_eq!(none.from, "");
    assert_eq!(none.to, "");
}

fn period(from: &str, to: &str) -> Period {
    Period {
        from: from.to_owned(),
        to: to.to_owned(),
    }
}

#[test]
fn period_check_passes_open_sides_and_a_single_day() {
    for (from, to) in [
        ("", ""),
        ("2026-10-01", ""),
        ("", "2026-10-08"),
        ("2026-10-01", "2026-10-08"),
        ("2026-10-08", "2026-10-08"),
        ("2024-02-29", "2024-03-01"),
        ("2025-12-31", "2026-01-01"),
    ] {
        assert!(
            period(from, to).check().is_ok(),
            "from {from:?} to {to:?} is a valid period"
        );
    }
}

#[test]
fn period_check_refuses_from_after_to_naming_to() {
    for (from, to) in [("2026-10-09", "2026-10-08"), ("2027-01-01", "2026-12-31")] {
        let e = period(from, to)
            .check()
            .expect_err("from after to is refused");
        assert_eq!(e.code, "periodOrder", "{from}..{to}");
        assert_eq!(e.field, Some("to"), "{from}..{to}");
        assert_eq!(e.message, PERIOD_ORDER, "{from}..{to}");
        assert_eq!(e.status, StatusCode::BAD_REQUEST, "{from}..{to}");
    }
}

#[test]
fn period_check_refuses_a_date_that_is_not_a_calendar_yyyy_mm_dd_naming_its_side() {
    for bad in [
        "2026-02-30",
        "2026-02-29",
        "2026-13-01",
        "2026-00-10",
        "2026-1-5",
        "20261008",
        "2026/10/08",
        "08-10-2026",
        "2026-10-08T00:00:00",
        " 2026-10-08",
        "abc",
    ] {
        let e = period(bad, "").check().expect_err("a bad from");
        assert_eq!(
            (e.code, e.field, e.message.as_str(), e.status),
            (
                "invalidRequest",
                Some("from"),
                INVALID,
                StatusCode::BAD_REQUEST
            ),
            "from {bad:?}"
        );
        let e = period("", bad).check().expect_err("a bad to");
        assert_eq!(
            (e.code, e.field, e.message.as_str(), e.status),
            (
                "invalidRequest",
                Some("to"),
                INVALID,
                StatusCode::BAD_REQUEST
            ),
            "to {bad:?}"
        );
    }
}

// ------------------------------------------------------------ F1: no answer changes

#[actix_web::test]
async fn a_view_without_a_filter_answers_the_same_whatever_extra_keys_it_is_sent() {
    let db = TestDb::start().await;
    let views = Views::new();
    db.write("XC-B01").await;
    let bare = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-plain?branch=XC-B01",
    )
    .await;
    is_changed(&bare, 1, "no extra key");
    for uri in [
        "/api/v3/views/test-plain?branch=XC-B01&foo=1",
        "/api/v3/views/test-plain?branch=XC-B01&foo=1&foo=2&bar=",
        "/api/v3/views/test-plain?q=a&q=b&branch=XC-B01&from=2026-10-09&to=2026-10-08",
    ] {
        let a = get(&db, &views, "owner", uri).await;
        assert_eq!(a.status, StatusCode::OK, "{uri}: body {}", a.body);
        assert_eq!(
            a.body, bare.body,
            "{uri}: the same answer as without the keys"
        );
    }
    let unchanged = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-plain?branch=XC-B01&since=1&foo=1",
    )
    .await;
    is_unchanged(&unchanged, 1, "since = version with an extra key");

    let me = get(&db, &views, "owner", "/api/v3/views/me").await;
    is_changed(&me, 0, "me");
    let me_extra = get(&db, &views, "owner", "/api/v3/views/me?foo=1&q=x").await;
    assert_eq!(me_extra.status, StatusCode::OK, "me with extra keys");
    assert_eq!(
        me_extra.body, me.body,
        "me answers as before with extra keys"
    );
    db.finish().await;
}

// ------------------------------------------------------------ the filter reaches the view

#[actix_web::test]
async fn a_branch_view_is_loaded_with_the_filter_of_the_non_paging_keys() {
    let db = TestDb::start().await;
    let views = Views::new();
    let a = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-filtered?branch=XC-B01&q=abc&minAmount=5&from=2026-10-01&to=2026-10-08",
    )
    .await;
    is_changed(&a, 0, "every filter key");
    let b = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-filtered?branch=XC-B01",
    )
    .await;
    is_changed(&b, 0, "no filter key");
    let c = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-filtered?q=%E0%B8%AA%20%E0%B8%A1&branch=XC-B01",
    )
    .await;
    is_changed(&c, 0, "an encoded Thai text before the paging key");
    assert_eq!(
        views.filtered.asked(),
        vec![
            (
                "XC-B01".to_owned(),
                probe("abc", Some(5), "2026-10-01", "2026-10-08")
            ),
            ("XC-B01".to_owned(), Probe::default()),
            ("XC-B01".to_owned(), probe("ส ม", None, "", "")),
        ]
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_paged_view_is_asked_with_the_filter_beside_before_and_limit() {
    let db = TestDb::start().await;
    let views = Views::new();
    db.write("MAIN").await;
    let a = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-filtered-paged?q=x&before=r9&minAmount=0&limit=2&to=2026-10-08",
    )
    .await;
    is_changed(&a, 1, "an older page, filtered");
    assert_eq!(a.body["data"], json!([{ "id": "r1" }]));
    let b = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-filtered-paged?limit=100",
    )
    .await;
    is_changed(&b, 1, "the first page, no filter");
    assert_eq!(
        views.filtered_pages.asked(),
        vec![
            (
                "MAIN".to_owned(),
                probe("x", Some(0), "", "2026-10-08"),
                Some("r9".to_owned()),
                2
            ),
            ("MAIN".to_owned(), Probe::default(), None, 100),
        ]
    );
    db.finish().await;
}

#[actix_web::test]
async fn the_paging_keys_keep_their_meaning_and_never_reach_the_filter() {
    let db = TestDb::start().await;
    let views = Views::new();
    db.write("XC-B01").await;
    let a = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-reserved?branch=XC-B01&since=0&limit=3&before=r9&q=a",
    )
    .await;
    is_changed(&a, 1, "branch view, since behind the version");
    assert_eq!(a.body["data"]["branch"], "XC-B01");
    let b = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-reserved-paged?branch=XC-B01&since=0&limit=3&before=r9&q=a",
    )
    .await;
    is_changed(&b, 1, "paged view, an older page");
    let only_q = Reserved {
        q: "a".to_owned(),
        ..Reserved::default()
    };
    assert_eq!(
        views.reserved.asked(),
        vec![("XC-B01".to_owned(), only_q.clone())],
        "the branch view's filter holds no paging key"
    );
    assert_eq!(
        views.reserved_pages.asked(),
        vec![("XC-B01".to_owned(), only_q, Some("r9".to_owned()), 3)],
        "the paged view gets before and limit as paging, not in its filter"
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_repeated_filter_key_counts_its_first_value() {
    let db = TestDb::start().await;
    let views = Views::new();
    let a = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-filtered?q=a&minAmount=1&q=b&minAmount=x",
    )
    .await;
    is_changed(&a, 0, "branch view, q and minAmount twice");
    let b = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-filtered-paged?q=a&q=b&from=2026-10-01&from=2026-10-09&to=2026-10-02&limit=7",
    )
    .await;
    is_changed(&b, 0, "paged view, q and from twice");
    assert_eq!(
        views.filtered.asked(),
        vec![("MAIN".to_owned(), probe("a", Some(1), "", ""))]
    );
    assert_eq!(
        views.filtered_pages.asked(),
        vec![(
            "MAIN".to_owned(),
            probe("a", None, "2026-10-01", "2026-10-02"),
            None,
            7
        )]
    );
    db.finish().await;
}

#[actix_web::test]
async fn an_unknown_key_beside_a_filter_is_ignored() {
    let db = TestDb::start().await;
    let views = Views::new();
    let a = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-filtered?zzz=1&q=a&min_amount=x",
    )
    .await;
    is_changed(&a, 0, "branch view");
    let b = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-filtered-paged?q=a&zzz=1&limit=7",
    )
    .await;
    is_changed(&b, 0, "paged view");
    assert_eq!(
        views.filtered.asked(),
        vec![("MAIN".to_owned(), probe("a", None, "", ""))]
    );
    assert_eq!(
        views.filtered_pages.asked(),
        vec![("MAIN".to_owned(), probe("a", None, "", ""), None, 7)]
    );
    db.finish().await;
}

// ------------------------------------------------------------ refusals

#[actix_web::test]
async fn a_filter_value_that_does_not_parse_is_refused_naming_its_wire_key() {
    let db = TestDb::start().await;
    let views = Views::new();
    for uri in [
        "/api/v3/views/test-filtered?minAmount=abc",
        "/api/v3/views/test-filtered?q=a&minAmount=-1",
        "/api/v3/views/test-filtered-paged?minAmount=1.5&limit=10",
        "/api/v3/views/test-filtered-paged?before=r9&minAmount=99999999999",
    ] {
        let a = get(&db, &views, "owner", uri).await;
        is_refused(&a, 400, "invalidRequest", Some("minAmount"), INVALID, uri);
    }
    views.nothing_loaded("a filter that does not parse");
    db.finish().await;
}

#[actix_web::test]
async fn a_period_from_after_to_is_refused_naming_to() {
    let db = TestDb::start().await;
    let views = Views::new();
    for uri in [
        "/api/v3/views/test-filtered?from=2026-10-09&to=2026-10-08",
        "/api/v3/views/test-filtered-paged?to=2026-10-08&from=2026-10-09&before=r1",
    ] {
        let a = get(&db, &views, "owner", uri).await;
        is_refused(&a, 400, "periodOrder", Some("to"), PERIOD_ORDER, uri);
    }
    views.nothing_loaded("from after to");
    db.finish().await;
}

#[actix_web::test]
async fn a_period_with_an_impossible_date_is_refused_naming_its_side() {
    let db = TestDb::start().await;
    let views = Views::new();
    for (uri, field) in [
        ("/api/v3/views/test-filtered?from=2026-02-30", "from"),
        (
            "/api/v3/views/test-filtered?from=2026-10-01&to=2026-13-01",
            "to",
        ),
        ("/api/v3/views/test-filtered-paged?from=2026-1-5", "from"),
        ("/api/v3/views/test-filtered-paged?to=20261008", "to"),
    ] {
        let a = get(&db, &views, "owner", uri).await;
        is_refused(&a, 400, "invalidRequest", Some(field), INVALID, uri);
    }
    views.nothing_loaded("an impossible date");
    db.finish().await;
}

#[actix_web::test]
async fn a_period_open_on_one_side_or_one_day_long_is_answered() {
    let db = TestDb::start().await;
    let views = Views::new();
    for uri in [
        "/api/v3/views/test-filtered?from=2026-10-01",
        "/api/v3/views/test-filtered?from=2026-10-01&to=",
        "/api/v3/views/test-filtered?to=2026-10-08",
        "/api/v3/views/test-filtered?from=2026-10-08&to=2026-10-08",
    ] {
        let a = get(&db, &views, "owner", uri).await;
        is_changed(&a, 0, uri);
    }
    assert_eq!(
        views.filtered.asked(),
        vec![
            ("MAIN".to_owned(), probe("", None, "2026-10-01", "")),
            ("MAIN".to_owned(), probe("", None, "2026-10-01", "")),
            ("MAIN".to_owned(), probe("", None, "", "2026-10-08")),
            (
                "MAIN".to_owned(),
                probe("", None, "2026-10-08", "2026-10-08")
            ),
        ]
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_bad_filter_is_refused_even_when_since_is_the_current_version() {
    let db = TestDb::start().await;
    let views = Views::new();
    db.write("MAIN").await;
    for (uri, code, field, message) in [
        (
            "/api/v3/views/test-filtered?since=1&minAmount=abc",
            "invalidRequest",
            "minAmount",
            INVALID,
        ),
        (
            "/api/v3/views/test-filtered?since=1&from=2026-10-09&to=2026-10-08",
            "periodOrder",
            "to",
            PERIOD_ORDER,
        ),
        (
            "/api/v3/views/test-filtered?from=2026-02-30&since=1",
            "invalidRequest",
            "from",
            INVALID,
        ),
        (
            "/api/v3/views/test-filtered-paged?since=1&minAmount=abc",
            "invalidRequest",
            "minAmount",
            INVALID,
        ),
        (
            "/api/v3/views/test-filtered-paged?since=1&from=2026-10-09&to=2026-10-08",
            "periodOrder",
            "to",
            PERIOD_ORDER,
        ),
    ] {
        let a = get(&db, &views, "owner", uri).await;
        is_refused(&a, 400, code, Some(field), message, uri);
    }
    views.nothing_loaded("a bad filter with since current");

    // and a good filter with since current is the O(1) unchanged answer
    for uri in [
        "/api/v3/views/test-filtered?since=1&q=a&from=2026-10-01",
        "/api/v3/views/test-filtered-paged?since=1&q=a&minAmount=3",
    ] {
        let a = get(&db, &views, "owner", uri).await;
        is_unchanged(&a, 1, uri);
    }
    views.nothing_loaded("a valid filter with since current");
    db.finish().await;
}

#[actix_web::test]
async fn sign_in_and_paging_are_refused_before_the_filter_is_read() {
    let db = TestDb::start().await;
    let views = Views::new();
    for uri in [
        "/api/v3/views/test-filtered?minAmount=abc",
        "/api/v3/views/test-filtered-paged?from=2026-10-09&to=2026-10-08",
    ] {
        let a = get(&db, &views, "", uri).await;
        is_refused(&a, 401, "signIn", None, "กรุณาเข้าสู่ระบบ", uri);
    }
    for uri in [
        "/api/v3/views/test-filtered?since=-1&minAmount=abc",
        "/api/v3/views/test-filtered?branch=XC_B01&from=2026-10-09&to=2026-10-08",
        "/api/v3/views/test-filtered-paged?limit=0&minAmount=abc",
        "/api/v3/views/test-filtered-paged?limit=101&from=2026-02-30",
    ] {
        let a = get(&db, &views, "owner", uri).await;
        is_refused(&a, 400, "invalidRequest", None, INVALID, uri);
    }
    views.nothing_loaded("refused before the filter");
    db.finish().await;
}

// ------------------------------------------------------------ branch

#[actix_web::test]
async fn a_staff_caller_naming_another_branch_with_a_filter_gets_their_own_branch_filtered() {
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
        "/api/v3/views/test-filtered?branch=XC-B02&q=a&from=2026-10-01",
    )
    .await;
    is_changed(&a, 1, "FRONT of XC-B01 naming XC-B02, filtered");
    assert_eq!(a.body["data"]["branch"], "XC-B01");
    assert_eq!(a.body["data"]["who"], "employee-7");
    let b = get(
        &db,
        &views,
        "staff:FRONT:XC-B01",
        "/api/v3/views/test-filtered-paged?branch=XC-B02&minAmount=4&limit=7",
    )
    .await;
    is_changed(&b, 1, "paged, FRONT of XC-B01 naming XC-B02, filtered");
    let c = get(
        &db,
        &views,
        "staff:FRONT:XC-B01",
        "/api/v3/views/test-filtered?branch=XC-B02&q=a&since=1",
    )
    .await;
    is_unchanged(&c, 1, "since compared with XC-B01's version");
    assert_eq!(
        views.filtered.asked(),
        vec![("XC-B01".to_owned(), probe("a", None, "2026-10-01", ""))]
    );
    assert_eq!(
        views.filtered_pages.asked(),
        vec![("XC-B01".to_owned(), probe("", Some(4), "", ""), None, 7)]
    );
    db.finish().await;
}

// ------------------------------------------------------------ VERSIONED = false

#[actix_web::test]
async fn an_unversioned_view_answers_with_data_even_when_since_is_current() {
    let db = TestDb::start().await;
    let views = Views::new();
    db.write("XC-B01").await;
    for (n, uri) in [
        "/api/v3/views/test-unversioned?branch=XC-B01&since=1",
        "/api/v3/views/test-unversioned?branch=XC-B01&since=0",
        "/api/v3/views/test-unversioned?branch=XC-B01",
        "/api/v3/views/test-unversioned?since=1&branch=XC-B01",
    ]
    .into_iter()
    .enumerate()
    {
        let a = get(&db, &views, "owner", uri).await;
        is_changed(&a, 1, uri);
        assert_eq!(a.body["data"]["branch"], "XC-B01", "{uri}");
        assert_eq!(
            views.unversioned.loads.load(Ordering::SeqCst),
            n + 1,
            "{uri}: load is called every time"
        );
    }
    // a versioned view on the same branch still takes the shortcut
    let plain = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-plain?branch=XC-B01&since=1",
    )
    .await;
    is_unchanged(&plain, 1, "the default VERSIONED view");
    db.finish().await;
}

// ------------------------------------------------------------ own_version

#[actix_web::test]
async fn a_view_with_its_own_version_answers_and_compares_by_it() {
    let db = TestDb::start().await;
    let views = Views::new();
    db.write("MAIN").await;
    db.write("MAIN").await;
    assert_eq!(
        db.version("MAIN").await,
        2,
        "the branch version differs from 7"
    );

    let same = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-own-version?since=7",
    )
    .await;
    is_unchanged(&same, 7, "since = its own version");
    assert_eq!(
        views.own.loads.load(Ordering::SeqCst),
        0,
        "not loaded while unchanged"
    );

    let branch = get(
        &db,
        &views,
        "owner",
        "/api/v3/views/test-own-version?since=2",
    )
    .await;
    is_changed(&branch, 7, "since = the branch version, not its own");
    let none = get(&db, &views, "owner", "/api/v3/views/test-own-version").await;
    is_changed(&none, 7, "no since");
    assert_eq!(views.own.loads.load(Ordering::SeqCst), 2);
    db.finish().await;
}

#[actix_web::test]
async fn an_own_version_that_moves_makes_the_same_since_changed() {
    let db = TestDb::start().await;
    let views = Views::new();
    db.write("MAIN").await;
    let uri = "/api/v3/views/test-own-version?since=7";
    is_unchanged(&get(&db, &views, "owner", uri).await, 7, "before it moves");

    views.own.version.store(8, Ordering::SeqCst);
    let after = get(&db, &views, "owner", uri).await;
    is_changed(&after, 8, "after its own version moved");
    assert_eq!(after.body["data"]["branch"], "MAIN");
    assert_eq!(views.own.loads.load(Ordering::SeqCst), 1);
    assert_eq!(
        db.version("MAIN").await,
        1,
        "the branch version did not move"
    );
    is_unchanged(
        &get(
            &db,
            &views,
            "owner",
            "/api/v3/views/test-own-version?since=8",
        )
        .await,
        8,
        "since = the moved version",
    );
    db.finish().await;
}

#[actix_web::test]
async fn an_equal_own_version_answers_without_reading_branch_versions() {
    let views = Views::new();
    // The control: on a pool that cannot be read, a view on the branch version fails.
    let control = get_on(
        common::unreachable_pool(),
        &views,
        "owner",
        "/api/v3/views/test-plain?since=0",
    )
    .await;
    assert_eq!(
        control.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a view on the branch version cannot answer without the database: {}",
        control.body
    );

    let a = get_on(
        common::unreachable_pool(),
        &views,
        "owner",
        "/api/v3/views/test-own-version?since=7",
    )
    .await;
    is_unchanged(&a, 7, "own version with no branch_versions read");
    assert_eq!(views.own.loads.load(Ordering::SeqCst), 0);
}
