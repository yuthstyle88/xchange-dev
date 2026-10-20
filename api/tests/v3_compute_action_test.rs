//! Specification tests for V0-D (#508; docs/design/xchange-v3-web.md §12.20.3, §12.20.9):
//! the compute-only action seam — a `POST` that reads what it must, computes,
//! answers and moves nothing — and the exact-decimal helpers every compute
//! action shares.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! - `presentation::http::v3::action::{ComputeAction, ComputeEndpoint}`:
//!   `ComputeEndpoint::register(cfg, Arc<A>)` serves only
//!   `POST /api/v3/compute/{A::NAME}` and runs, in this order: sign-in and
//!   `Policy::allow(INHERITS)`; `same_origin` (`400 originRejected`); the body
//!   (at most 16,384 bytes, a JSON object; else `400 invalidRequest`, no
//!   field); the reserved key `branch` (`400 invalidRequest`, `field: "branch"`);
//!   `Policy::branch` then `require_branch` (`400 noSuchBranch`); the body as
//!   `A::Request` (`400 invalidRequest`, no field); `A::compute`. The answer is
//!   the action's DTO itself, `no-store`.
//! - `xc_kernel::money::{typed_decimal, round_cents, fixed2, fixed6}`: v2's
//!   `Math.round` rule on exact decimals (§12.20.3, CA7).
//!
//! The test actions below live in this file only; the first real actions come
//! with #509 and #510.
//!
//! The endpoint tests need a real PostgreSQL: `TEST_DATABASE_URL` must name an
//! existing database whose name ends in `_test`. They fail without it; they do
//! not skip. Each test works in a schema of its own. The money tests need no
//! database.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use ts_rs::TS;
use xc_auth::domain::access::Actor as AccessActor;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{fixed2, fixed6, round_cents, typed_decimal};
use xc_rate::domain::validation::NO_SUCH_BRANCH;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http::v3::action::{ComputeAction, ComputeEndpoint};
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::pages::{SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const LIMIT: usize = 16_384;
const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
const SERVER_ERROR: &str = "เซิร์ฟเวอร์ขัดข้อง";

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
                 The v3 compute action tests need a real PostgreSQL and do not skip. Point it \
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
        let schema = unique("v3compute");
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
    /// version still rises (#231). Used to give branches a non-zero version.
    async fn write(&self, branch: &str) {
        BranchTx::new(Transactor::new(self.pool.clone()))
            .run(&BranchId::new(branch), |_tx| {
                Box::pin(async move { AppResult::Ok(()) })
            })
            .await
            .expect("a BranchTx run commits");
    }

    /// Every `branch_versions` row, in branch order.
    async fn version_rows(&self) -> Vec<(String, i64)> {
        sqlx::query_as::<_, (String, i64)>(
            "SELECT branch_id, version FROM branch_versions ORDER BY branch_id",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read branch_versions")
    }

    /// `BranchVersions::current` for each branch this file names.
    async fn current_versions(&self) -> Vec<(&'static str, i64)> {
        let versions = BranchVersions::new(self.pool.clone());
        let mut out = Vec::new();
        for b in ["MAIN", "XC-B01", "XC-B02", "XC-NEW"] {
            let v = versions
                .current(&BranchId::new(b))
                .await
                .expect("read a branch version");
            out.push((b, v));
        }
        out
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

// ------------------------------------------------------------ test actions

/// What every test action takes. It has no field named `branch` (that key is
/// the endpoint's, §12.20.3 step 4).
#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
struct AmountRequest {
    amount: String,
    note: Option<String>,
}

/// What the echoing test actions answer: the branch and the caller they were
/// computed for, and the amount they were sent.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct Echo {
    branch: String,
    who: String,
    amount: String,
}

/// Counts its computes and echoes; one type, three `INHERITS` pairs.
struct Echoing<const K: u8> {
    calls: Arc<AtomicUsize>,
}

impl<const K: u8> Echoing<K> {
    async fn echo(&self, branch: &BranchId, who: &AccessActor, request: AmountRequest) -> Echo {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _ = request.note;
        Echo {
            branch: branch.as_str().to_owned(),
            who: who.user_id.clone(),
            amount: request.amount,
        }
    }
}

/// `(Rpc, "adminSnapshot")`: the cash menu's row (CA4) — the owner and every staff role.
type EchoSnapshot = Echoing<0>;
/// `(Rpc, "rateDraft")` (re-expected for RE-7 #604: was `adminSaveRate`): a narrower row of the same path.
type EchoSaveRate = Echoing<1>;
/// `(StaffRpc, "adminDeskConfig")`: FRONT only, staff session only.
type EchoDeskConfig = Echoing<2>;

impl ComputeAction for EchoSnapshot {
    const NAME: &'static str = "test-echo";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Request = AmountRequest;
    type Answer = Echo;

    async fn compute(
        &self,
        branch: &BranchId,
        who: &AccessActor,
        request: AmountRequest,
    ) -> Result<Echo, V3Error> {
        Ok(self.echo(branch, who, request).await)
    }
}

impl ComputeAction for EchoSaveRate {
    const NAME: &'static str = "test-save-rate";
    // re-expected for RE-7 #604 by the ticket author: `adminSaveRate` is retired (design §9);
    // `rateDraft` has the same rows of `Policy` (the owner, and RATES)
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    type Request = AmountRequest;
    type Answer = Echo;

    async fn compute(
        &self,
        branch: &BranchId,
        who: &AccessActor,
        request: AmountRequest,
    ) -> Result<Echo, V3Error> {
        Ok(self.echo(branch, who, request).await)
    }
}

impl ComputeAction for EchoDeskConfig {
    const NAME: &'static str = "test-desk-config";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::StaffRpc, "adminDeskConfig");
    type Request = AmountRequest;
    type Answer = Echo;

    async fn compute(
        &self,
        branch: &BranchId,
        who: &AccessActor,
        request: AmountRequest,
    ) -> Result<Echo, V3Error> {
        Ok(self.echo(branch, who, request).await)
    }
}

const REFUSED_WITH_FIELD: &str = "ยอดเงินไม่ถูกต้อง (ทดสอบ)";
const REFUSED_WITHOUT_FIELD: &str = "คำนวณไม่ได้ (ทดสอบ)";

/// Refuses by its amount: `field` → 409 with a field, `plain` → 422 without,
/// `server` → 500 serverError. Anything else computes.
struct Refusing {
    calls: Arc<AtomicUsize>,
}

impl ComputeAction for Refusing {
    const NAME: &'static str = "test-refuse";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Request = AmountRequest;
    type Answer = Echo;

    async fn compute(
        &self,
        branch: &BranchId,
        who: &AccessActor,
        request: AmountRequest,
    ) -> Result<Echo, V3Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match request.amount.as_str() {
            "field" => Err(V3Error::new(
                "testRefused",
                Some("amount"),
                REFUSED_WITH_FIELD,
                StatusCode::CONFLICT,
            )),
            "plain" => Err(V3Error::new(
                "testRefusedPlain",
                None,
                REFUSED_WITHOUT_FIELD,
                StatusCode::UNPROCESSABLE_ENTITY,
            )),
            "server" => Err(V3Error::new(
                "serverError",
                None,
                SERVER_ERROR,
                StatusCode::INTERNAL_SERVER_ERROR,
            )),
            _ => Ok(Echo {
                branch: branch.as_str().to_owned(),
                who: who.user_id.clone(),
                amount: request.amount,
            }),
        }
    }
}

/// The test actions and one counter of every compute they ran.
struct Actions {
    calls: Arc<AtomicUsize>,
}

impl Actions {
    fn new() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

// ------------------------------------------------------------ requests

/// A request to the whole application (`bootstrap::configure`) with the test
/// actions registered beside it. `Host` is always `xc.test`.
struct Call {
    method: Method,
    uri: String,
    who: String,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl Call {
    /// `POST uri` as `who` from the same origin, with the marker header and a
    /// JSON content type — what `ApiClient.act` sends.
    fn post(who: &str, uri: &str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            method: Method::POST,
            uri: uri.to_owned(),
            who: who.to_owned(),
            headers: vec![
                ("x-xchange-request", "1".to_owned()),
                ("origin", SAME_ORIGIN.to_owned()),
                ("content-type", "application/json".to_owned()),
            ],
            body: body.into(),
        }
    }

    fn json(who: &str, uri: &str, body: Value) -> Self {
        Self::post(who, uri, body.to_string())
    }

    fn method(mut self, method: Method) -> Self {
        self.method = method;
        self
    }

    fn without(mut self, name: &str) -> Self {
        self.headers.retain(|(n, _)| *n != name);
        self
    }

    fn with(mut self, name: &'static str, value: &str) -> Self {
        self.headers.retain(|(n, _)| *n != name);
        self.headers.push((name, value.to_owned()));
        self
    }
}

#[derive(Debug)]
struct Answer {
    status: StatusCode,
    content_type: Option<String>,
    cache_control: Option<String>,
    text: String,
    body: Value,
}

async fn send(db: &TestDb, actions: &Actions, call: Call) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let calls = actions.calls.clone();
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| {
                ComputeEndpoint::register(
                    cfg,
                    Arc::new(EchoSnapshot {
                        calls: calls.clone(),
                    }),
                );
                ComputeEndpoint::register(
                    cfg,
                    Arc::new(EchoSaveRate {
                        calls: calls.clone(),
                    }),
                );
                ComputeEndpoint::register(
                    cfg,
                    Arc::new(EchoDeskConfig {
                        calls: calls.clone(),
                    }),
                );
                ComputeEndpoint::register(cfg, Arc::new(Refusing { calls }));
                bootstrap::configure(cfg, state);
            })
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(call.method.clone())
        .uri(&call.uri)
        .insert_header((header::HOST, HOST));
    if !call.who.is_empty() {
        req = req.insert_header((AS, call.who.as_str()));
    }
    for (name, value) in &call.headers {
        req = req.insert_header((*name, value.as_str()));
    }
    let resp = atest::call_service(&app, req.set_payload(call.body.clone()).to_request()).await;
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().expect("text").to_owned());
    let cache_control = resp
        .headers()
        .get(header::CACHE_CONTROL)
        .map(|v| v.to_str().expect("text").to_owned());
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer {
        status,
        content_type,
        cache_control,
        text,
        body,
    }
}

const ECHO: &str = "/api/v3/compute/test-echo";
const REFUSE: &str = "/api/v3/compute/test-refuse";
const SAVE_RATE: &str = "/api/v3/compute/test-save-rate";
const DESK_CONFIG: &str = "/api/v3/compute/test-desk-config";

/// `200`, the DTO as the whole body (no `version`, no `changed`), JSON, `no-store`.
fn is_answer(a: &Answer, want: Value, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: status; body {}", a.text);
    assert_eq!(a.body, want, "{what}: exactly the action's answer");
    assert!(
        a.content_type
            .as_deref()
            .is_some_and(|c| c.starts_with("application/json")),
        "{what}: content type {:?}",
        a.content_type
    );
    assert_eq!(a.cache_control.as_deref(), Some("no-store"), "{what}");
}

fn echo(branch: &str, who: &str, amount: &str) -> Value {
    json!({ "branch": branch, "who": who, "amount": amount })
}

/// §4.3's refusal body, exactly: `{error: {code, field?, message}}`, `no-store`.
fn is_refused(a: &Answer, status: u16, code: &str, field: Option<&str>, message: &str, what: &str) {
    assert_eq!(a.status.as_u16(), status, "{what}: status; body {}", a.text);
    let want = match field {
        Some(f) => json!({ "error": { "code": code, "field": f, "message": message } }),
        None => json!({ "error": { "code": code, "message": message } }),
    };
    assert_eq!(a.body, want, "{what}: the refusal body");
    assert_eq!(a.cache_control.as_deref(), Some("no-store"), "{what}");
}

fn is_invalid_request(a: &Answer, field: Option<&str>, what: &str) {
    is_refused(a, 400, "invalidRequest", field, BAD_REQUEST, what);
}

fn is_origin_rejected(a: &Answer, what: &str) {
    is_refused(a, 400, "originRejected", None, "ORIGIN_REJECTED", what);
}

/// A JSON object of exactly `len` bytes that the test actions accept once the
/// size is allowed: `{"amount":"1"<extra>,"pad":"xxx…"}`.
fn object_of_len(len: usize, extra: &str) -> Vec<u8> {
    let head = format!("{{\"amount\":\"1\"{extra},\"pad\":\"");
    let tail = "\"}";
    assert!(len >= head.len() + tail.len(), "{len} is too short");
    let pad = "x".repeat(len - head.len() - tail.len());
    let body = format!("{head}{pad}{tail}");
    assert_eq!(body.len(), len);
    serde_json::from_str::<Value>(&body).expect("a JSON object");
    body.into_bytes()
}

// ------------------------------------------------------------ the answer

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
async fn an_accepted_call_answers_the_dto_itself_as_no_store_json() {
    let db = TestDb::start().await;
    let actions = Actions::new();
    db.write("XC-B01").await;

    let a = send(
        &db,
        &actions,
        Call::json(
            "owner",
            ECHO,
            json!({ "amount": "12.50", "branch": "XC-B01", "note": "n", "extra": [1, { "deep": true }] }),
        ),
    )
    .await;
    is_answer(&a, echo("XC-B01", "owner-1", "12.50"), "owner on XC-B01");
    assert!(
        a.body.get("version").is_none() && a.body.get("changed").is_none(),
        "no version and no changed (CA2): {}",
        a.text
    );
    assert_eq!(actions.calls(), 1, "computed once");
    db.finish().await;
}

#[actix_web::test]
async fn a_call_never_moves_any_branch_version() {
    let db = TestDb::start().await;
    let actions = Actions::new();
    db.write("XC-B01").await;
    db.write("XC-B02").await;
    db.write("XC-B02").await;
    let rows_before = db.version_rows().await;
    let current_before = db.current_versions().await;
    assert_eq!(
        rows_before,
        vec![("XC-B01".to_string(), 1), ("XC-B02".to_string(), 2)],
        "the seeded versions"
    );

    let mut answered = 0;
    for _ in 0..3 {
        for call in [
            Call::json("owner", ECHO, json!({ "amount": "1" })),
            Call::json("owner", ECHO, json!({ "amount": "1", "branch": "XC-B01" })),
            Call::json("owner", ECHO, json!({ "amount": "1", "branch": "XC-NEW" })),
            // OR-4b (D-4): staff name their own branch (another is refused, below)
            Call::json(
                "staff:FRONT:XC-B01",
                ECHO,
                json!({ "amount": "1", "branch": "XC-B01" }),
            ),
            Call::json("staff:FRONT:XC-B02", DESK_CONFIG, json!({ "amount": "1" })),
            Call::json(
                "owner",
                REFUSE,
                json!({ "amount": "ok", "branch": "XC-B02" }),
            ),
        ] {
            let a = send(&db, &actions, call).await;
            assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
            answered += 1;
        }
        for amount in ["field", "plain", "server"] {
            let a = send(
                &db,
                &actions,
                Call::json(
                    "owner",
                    REFUSE,
                    json!({ "amount": amount, "branch": "XC-B01" }),
                ),
            )
            .await;
            assert_ne!(a.status, StatusCode::OK, "{amount}: refused");
        }
    }
    // OR-4b (D-4): a staff caller naming another branch is refused before compute, and moves no
    // version either
    let a = send(
        &db,
        &actions,
        Call::json(
            "staff:FRONT:XC-B01",
            ECHO,
            json!({ "amount": "1", "branch": "XC-B02" }),
        ),
    )
    .await;
    is_out_of_scope(&a, "FRONT of XC-B01 naming XC-B02");
    assert_eq!(answered, 18);
    assert_eq!(actions.calls(), 27, "every call above reached compute");

    assert_eq!(
        db.version_rows().await,
        rows_before,
        "branch_versions after the calls: no version moved, no row added (CA3)"
    );
    assert_eq!(
        db.current_versions().await,
        current_before,
        "BranchVersions::current after the calls"
    );
    db.finish().await;
}

// ------------------------------------------------------------ who may call

/// §12.20.3 step 1, CA4: an action whose INHERITS is a pair refuses_exactly the
/// callers `Policy::authorize` / `RoleGrants::seed` refuses for that pair, with the policy's refusal.
#[actix_web::test]
async fn the_endpoint_refuses_exactly_whom_the_policy_refuses_for_the_actions_pair() {
    use xc_auth::domain::access::RoleGrants;
    use xc_auth::domain::access::v3_roles::{actor_from_v3, blueprint_roles};
    use xc_auth::domain::policy::entities::policy::Need;

    let db = TestDb::start().await;
    let actions = Actions::new();
    let mut allowed = 0;
    let mut refused = 0;
    for (uri, path, action) in [
        (ECHO, EntryPath::Rpc, "adminSnapshot"),
        (SAVE_RATE, EntryPath::Rpc, "rateDraft"), // RE-7 #604: was adminSaveRate
        (DESK_CONFIG, EntryPath::StaffRpc, "adminDeskConfig"),
    ] {
        let act = Action::new(action);
        let Some(Need::Permission(perm)) = Policy::need(path, &act) else {
            panic!("({path:?}, {action:?}) needs a permission");
        };
        for who in every_caller() {
            let a = send(
                &db,
                &actions,
                Call::json(&who, uri, json!({ "amount": "7" })),
            )
            .await;
            let what = format!("{uri} as {who:?}");
            let actor = actor_on(path, &who);
            let holds = actor.as_ref().is_some_and(|ac| {
                blueprint_roles(ac.role())
                    .iter()
                    .any(|&r| RoleGrants::seed(r).contains(&perm))
            });
            let user = actor.as_ref().map(|ac| {
                actor_from_v3(
                    ac,
                    xchange::presentation::pages::SEEDED_XC_ORG_ID,
                    chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
                )
            });
            let auth = Policy::authorize(user.as_ref(), path, &act, None, chrono::Utc::now());
            assert_eq!(
                auth.is_ok(),
                holds,
                "{what}: RoleGrants::seed matches Policy::authorize"
            );
            match auth {
                Ok(_) => {
                    allowed += 1;
                    let actor = actor.expect("allowed means signed in");
                    let branch = Policy::branch(Some(&actor), None);
                    is_answer(
                        &a,
                        echo(branch.as_str(), actor.id(), "7"),
                        &format!("{what}: answered for the actor of its entry path"),
                    );
                }
                Err(e) => {
                    refused += 1;
                    let status = e.status();
                    let message = e.to_string();
                    let code = V3Error::from(e).code;
                    is_refused(&a, status, code, None, &message, &what);
                }
            }
        }
    }
    assert_eq!(
        actions.calls(),
        allowed,
        "only the allowed callers computed"
    );
    assert!(allowed >= 20, "allowed: {allowed}");
    assert!(refused >= 20, "refused: {refused}");
    db.finish().await;
}

// ------------------------------------------------------------ the origin

#[actix_web::test]
async fn a_missing_marker_or_a_foreign_origin_is_origin_rejected() {
    let db = TestDb::start().await;
    let actions = Actions::new();
    let body = json!({ "amount": "1" });
    let cases = [
        (
            "no marker",
            Call::json("owner", ECHO, body.clone()).without("x-xchange-request"),
        ),
        (
            "marker 2",
            Call::json("owner", ECHO, body.clone()).with("x-xchange-request", "2"),
        ),
        (
            "marker true",
            Call::json("owner", ECHO, body.clone()).with("x-xchange-request", "true"),
        ),
        (
            "no origin",
            Call::json("owner", ECHO, body.clone()).without("origin"),
        ),
        (
            "foreign origin",
            Call::json("owner", ECHO, body.clone()).with("origin", "http://evil.test"),
        ),
        (
            "other scheme",
            Call::json("owner", ECHO, body.clone()).with("origin", "https://xc.test"),
        ),
        (
            "other port",
            Call::json("owner", ECHO, body.clone()).with("origin", "http://xc.test:8080"),
        ),
        (
            "null origin",
            Call::json("owner", ECHO, body.clone()).with("origin", "null"),
        ),
        (
            "neither",
            Call::json("owner", ECHO, body.clone())
                .without("origin")
                .without("x-xchange-request"),
        ),
        (
            "staff, foreign origin",
            Call::json("staff:FRONT:XC-B01", ECHO, body.clone()).with("origin", "http://evil.test"),
        ),
        (
            "staff path, no marker",
            Call::json("staff:FRONT:XC-B01", DESK_CONFIG, body.clone())
                .without("x-xchange-request"),
        ),
    ];
    for (what, call) in cases {
        let a = send(&db, &actions, call).await;
        is_origin_rejected(&a, what);
    }
    assert_eq!(actions.calls(), 0, "a rejected origin computes nothing");
    db.finish().await;
}

// ------------------------------------------------------------ the body

#[actix_web::test]
async fn a_body_that_is_not_a_json_object_is_an_invalid_request_without_a_field() {
    let db = TestDb::start().await;
    let actions = Actions::new();
    let bodies: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("text", b"not json".to_vec()),
        ("cut short", br#"{"amount":"1""#.to_vec()),
        ("two objects", br#"{"amount":"1"}{"amount":"2"}"#.to_vec()),
        ("an array", br#"[]"#.to_vec()),
        ("an array of the object", br#"[{"amount":"1"}]"#.to_vec()),
        // serde reads a struct from an array by position: this one would fit
        // the request, so only the object rule refuses it
        ("an array that fits the request", br#"["1", null]"#.to_vec()),
        ("a string", br#""amount""#.to_vec()),
        ("a number", b"5".to_vec()),
        ("null", b"null".to_vec()),
        ("true", b"true".to_vec()),
        (
            "not UTF-8",
            vec![b'{', b'"', 0xff, 0xfe, b'"', b':', b'1', b'}'],
        ),
    ];
    for (what, body) in bodies {
        let a = send(&db, &actions, Call::post("owner", ECHO, body)).await;
        is_invalid_request(&a, None, what);
    }
    assert_eq!(actions.calls(), 0);
    db.finish().await;
}

/// The limit is 16,384 bytes and its refusal is §4.3's `400 invalidRequest`,
/// not actix's default `413` — also for a body past actix's own default limits.
/// (The body of exactly 16,384 bytes is the implementer's test, §9.)
#[actix_web::test]
async fn a_body_over_16384_bytes_is_an_invalid_request_not_413() {
    let db = TestDb::start().await;
    let actions = Actions::new();
    for len in [LIMIT + 1, LIMIT + 1000, 64 * 1024, 300 * 1024, 1024 * 1024] {
        let a = send(
            &db,
            &actions,
            Call::post("owner", ECHO, object_of_len(len, "")),
        )
        .await;
        is_invalid_request(&a, None, &format!("{len} bytes"));
    }
    assert_eq!(actions.calls(), 0, "an oversized body computes nothing");

    let a = send(
        &db,
        &actions,
        Call::post("owner", ECHO, object_of_len(16_000, "")),
    )
    .await;
    is_answer(
        &a,
        echo("MAIN", "owner-1", "1"),
        "16,000 bytes, under the limit",
    );
    assert_eq!(actions.calls(), 1);
    db.finish().await;
}

/// The table lists no `Content-Type` refusal, so none is added.
#[actix_web::test]
async fn a_body_is_read_whatever_its_content_type() {
    let db = TestDb::start().await;
    let actions = Actions::new();
    let body = json!({ "amount": "3" });
    for (what, call) in [
        (
            "no content type",
            Call::json("owner", ECHO, body.clone()).without("content-type"),
        ),
        (
            "text/plain",
            Call::json("owner", ECHO, body.clone()).with("content-type", "text/plain"),
        ),
        (
            "json with a charset",
            Call::json("owner", ECHO, body.clone())
                .with("content-type", "application/json; charset=utf-8"),
        ),
    ] {
        let a = send(&db, &actions, call).await;
        is_answer(&a, echo("MAIN", "owner-1", "3"), what);
    }
    assert_eq!(actions.calls(), 3);
    db.finish().await;
}

// ------------------------------------------------------------ the branch

#[actix_web::test]
async fn the_reserved_key_branch_names_the_branch_like_the_view_query() {
    let db = TestDb::start().await;
    let actions = Actions::new();
    for (what, body, branch) in [
        ("absent", json!({ "amount": "1" }), "MAIN"),
        ("null", json!({ "amount": "1", "branch": null }), "MAIN"),
        ("empty", json!({ "amount": "1", "branch": "" }), "MAIN"),
        (
            "XC-B02",
            json!({ "amount": "1", "branch": "XC-B02" }),
            "XC-B02",
        ),
        (
            "XC-NEW",
            json!({ "amount": "1", "branch": "XC-NEW" }),
            "XC-NEW",
        ),
    ] {
        let a = send(&db, &actions, Call::json("owner", ECHO, body)).await;
        is_answer(
            &a,
            echo(branch, "owner-1", "1"),
            &format!("owner, branch {what}"),
        );
    }
    assert_eq!(actions.calls(), 5);
    db.finish().await;
}

/// §12.20.3 step 4. (A JSON number is the implementer's test, §9.)
#[actix_web::test]
async fn an_ill_formed_branch_is_an_invalid_request_on_the_field_branch() {
    let db = TestDb::start().await;
    let actions = Actions::new();
    let long = "A".repeat(71);
    for (what, branch) in [
        ("an underscore", json!("XC_B01")),
        ("a space inside", json!("bad name")),
        ("a trailing space", json!("XC-B01 ")),
        ("a leading space", json!(" XC-B01")),
        ("a space alone", json!(" ")),
        ("71 characters", json!(long)),
        ("non-ASCII", json!("สาขา")),
        ("true", json!(true)),
        ("an object", json!({ "id": "XC-B01" })),
        ("an array", json!(["XC-B01"])),
    ] {
        for who in ["owner", "staff:FRONT:XC-B01"] {
            let a = send(
                &db,
                &actions,
                Call::json(who, ECHO, json!({ "amount": "1", "branch": branch })),
            )
            .await;
            is_invalid_request(&a, Some("branch"), &format!("{who}: branch {what}"));
        }
    }
    assert_eq!(actions.calls(), 0);
    db.finish().await;
}

#[actix_web::test]
async fn a_staff_caller_naming_another_branch_computes_on_their_own() {
    let db = TestDb::start().await;
    let actions = Actions::new();
    // OR-4b (D-4): naming another branch (known, unknown or MAIN) is refused before compute;
    // naming none or its own, it computes on its own
    for (what, body) in [
        (
            "naming XC-B02",
            json!({ "amount": "2", "branch": "XC-B02" }),
        ),
        (
            "naming an unknown branch",
            json!({ "amount": "2", "branch": "XC-NOPE" }),
        ),
        ("naming MAIN", json!({ "amount": "2", "branch": "MAIN" })),
    ] {
        let a = send(&db, &actions, Call::json("staff:FRONT:XC-B01", ECHO, body)).await;
        is_out_of_scope(&a, &format!("FRONT of XC-B01 {what}"));
    }
    for (what, body) in [
        ("naming none", json!({ "amount": "2" })),
        (
            "naming its own",
            json!({ "amount": "2", "branch": "XC-B01" }),
        ),
    ] {
        let a = send(&db, &actions, Call::json("staff:FRONT:XC-B01", ECHO, body)).await;
        is_answer(
            &a,
            echo("XC-B01", "employee-7", "2"),
            &format!("FRONT of XC-B01 {what}"),
        );
    }
    assert_eq!(actions.calls(), 2);
    db.finish().await;
}

#[actix_web::test]
async fn an_unknown_branch_named_by_the_owner_is_no_such_branch() {
    let db = TestDb::start().await;
    let actions = Actions::new();
    for branch in ["XC-NOPE", "XC-B03"] {
        let a = send(
            &db,
            &actions,
            Call::json("owner", ECHO, json!({ "amount": "1", "branch": branch })),
        )
        .await;
        is_refused(
            &a,
            400,
            "noSuchBranch",
            Some("branch"),
            NO_SUCH_BRANCH,
            branch,
        );
    }
    assert_eq!(actions.calls(), 0);
    db.finish().await;
}

// ------------------------------------------------------------ the request

#[actix_web::test]
async fn a_body_that_does_not_fit_the_request_is_an_invalid_request_without_a_field() {
    let db = TestDb::start().await;
    let actions = Actions::new();
    for (what, body) in [
        ("no amount", json!({})),
        ("only a branch", json!({ "branch": "XC-B01" })),
        ("a number amount", json!({ "amount": 5 })),
        ("a null amount", json!({ "amount": null })),
        ("an object amount", json!({ "amount": { "v": "1" } })),
        ("a number note", json!({ "amount": "1", "note": 7 })),
    ] {
        let a = send(&db, &actions, Call::json("owner", ECHO, body)).await;
        is_invalid_request(&a, None, what);
    }
    assert_eq!(actions.calls(), 0);

    let a = send(
        &db,
        &actions,
        Call::json(
            "owner",
            ECHO,
            json!({ "amount": "4", "unknown": { "deep": [1, 2] }, "branch": "" }),
        ),
    )
    .await;
    is_answer(
        &a,
        echo("MAIN", "owner-1", "4"),
        "unknown keys, branch included, are ignored",
    );
    assert_eq!(actions.calls(), 1);
    db.finish().await;
}

#[actix_web::test]
async fn an_actions_refusal_passes_through_with_its_status_code_field_and_message() {
    let db = TestDb::start().await;
    let actions = Actions::new();
    let field = send(
        &db,
        &actions,
        Call::json("owner", REFUSE, json!({ "amount": "field" })),
    )
    .await;
    is_refused(
        &field,
        409,
        "testRefused",
        Some("amount"),
        REFUSED_WITH_FIELD,
        "with a field",
    );
    let plain = send(
        &db,
        &actions,
        Call::json("owner", REFUSE, json!({ "amount": "plain" })),
    )
    .await;
    is_refused(
        &plain,
        422,
        "testRefusedPlain",
        None,
        REFUSED_WITHOUT_FIELD,
        "without a field",
    );
    let server = send(
        &db,
        &actions,
        Call::json("owner", REFUSE, json!({ "amount": "server" })),
    )
    .await;
    is_refused(
        &server,
        500,
        "serverError",
        None,
        SERVER_ERROR,
        "serverError",
    );
    assert_eq!(actions.calls(), 3);
    db.finish().await;
}

// ------------------------------------------------------------ the order

/// §12.20.3: sign-in and policy → origin → body → `branch` → `Policy::branch`
/// and `require_branch` → `A::Request` → compute. Each row breaks two steps at
/// once and expects the earlier one's refusal.
#[actix_web::test]
async fn the_checks_run_in_the_designs_order() {
    let db = TestDb::start().await;
    let actions = Actions::new();

    // 1 before 2: nobody, foreign origin, a body that is not JSON
    let a = send(
        &db,
        &actions,
        Call::post("", ECHO, b"not json".to_vec()).with("origin", "http://evil.test"),
    )
    .await;
    is_refused(
        &a,
        401,
        "signIn",
        None,
        "กรุณาเข้าสู่ระบบ",
        "nobody, foreign origin",
    );

    // 1 before 2: a role outside the row, no marker, an oversized body
    assert!(
        !xc_auth::domain::access::RoleGrants::seed(xc_auth::domain::access::Role::Finance)
            .contains(&xc_auth::domain::access::Permission::ExchangeTransact),
        "§11.2: FINANCE does not hold ExchangeTransact"
    );
    let finance = actor_on(EntryPath::StaffRpc, "staff:FINANCE:XC-B01").map(|ac| {
        xc_auth::domain::access::v3_roles::actor_from_v3(
            &ac,
            xchange::presentation::pages::SEEDED_XC_ORG_ID,
            chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
        )
    });
    let refusal = Policy::authorize(
        finance.as_ref(),
        EntryPath::StaffRpc,
        &Action::new("adminDeskConfig"),
        None,
        chrono::Utc::now(),
    )
    .expect_err("FINANCE is outside adminDeskConfig's row");
    let a = send(
        &db,
        &actions,
        Call::post(
            "staff:FINANCE:XC-B01",
            DESK_CONFIG,
            object_of_len(LIMIT + 1, ""),
        )
        .without("x-xchange-request"),
    )
    .await;
    let (status, message) = (refusal.status(), refusal.to_string());
    is_refused(
        &a,
        status,
        V3Error::from(refusal).code,
        None,
        &message,
        "FINANCE, no marker",
    );

    // 2 before 3: foreign origin, a body that is not JSON / oversized
    let a = send(
        &db,
        &actions,
        Call::post("owner", ECHO, b"not json".to_vec()).with("origin", "http://evil.test"),
    )
    .await;
    is_origin_rejected(&a, "foreign origin, not JSON");
    let a = send(
        &db,
        &actions,
        Call::post("owner", ECHO, object_of_len(LIMIT + 1, "")).without("x-xchange-request"),
    )
    .await;
    is_origin_rejected(&a, "no marker, oversized");

    // 3 before 4: oversized, with an ill-formed branch inside
    let a = send(
        &db,
        &actions,
        Call::post(
            "owner",
            ECHO,
            object_of_len(LIMIT + 1, r#","branch":"XC_B01""#),
        ),
    )
    .await;
    is_invalid_request(&a, None, "oversized with an ill-formed branch");

    // 4 before 5 and 6: an ill-formed branch, a request that does not fit
    let a = send(
        &db,
        &actions,
        Call::json("owner", ECHO, json!({ "branch": "XC_B01", "amount": 5 })),
    )
    .await;
    is_invalid_request(&a, Some("branch"), "ill-formed branch, bad request");

    // 5 before 6: an unknown branch, a request that does not fit
    let a = send(
        &db,
        &actions,
        Call::json("owner", ECHO, json!({ "branch": "XC-NOPE", "amount": 5 })),
    )
    .await;
    is_refused(
        &a,
        400,
        "noSuchBranch",
        Some("branch"),
        NO_SUCH_BRANCH,
        "unknown branch, bad request",
    );

    // the grant's branch before require_branch. OR-4b (D-4): staff naming an unknown branch are
    // refused as out of scope there, before 6
    let a = send(
        &db,
        &actions,
        Call::json(
            "staff:FRONT:XC-B01",
            ECHO,
            json!({ "branch": "XC-NOPE", "amount": 5 }),
        ),
    )
    .await;
    is_out_of_scope(&a, "staff naming an unknown branch, bad request");

    assert_eq!(actions.calls(), 0, "no refused request reached compute");

    // 7 last: everything fits, the action refuses
    let a = send(
        &db,
        &actions,
        Call::json(
            "owner",
            REFUSE,
            json!({ "branch": "XC-B01", "amount": "field" }),
        ),
    )
    .await;
    is_refused(
        &a,
        409,
        "testRefused",
        Some("amount"),
        REFUSED_WITH_FIELD,
        "compute last",
    );
    assert_eq!(actions.calls(), 1);
    db.finish().await;
}

// ------------------------------------------------------------ the route

/// CA2: only `POST /api/v3/compute/{NAME}` is routed.
#[actix_web::test]
async fn only_post_on_the_compute_area_is_routed() {
    let db = TestDb::start().await;
    let actions = Actions::new();
    for method in [Method::GET, Method::PUT, Method::DELETE, Method::PATCH] {
        let a = send(
            &db,
            &actions,
            Call::json("owner", ECHO, json!({ "amount": "1" })).method(method.clone()),
        )
        .await;
        assert!(
            matches!(a.status.as_u16(), 404 | 405),
            "{method} {ECHO}: not routed, got {} {}",
            a.status,
            a.text
        );
    }
    for uri in [
        "/api/v3/views/test-echo",
        "/api/v3/test-echo",
        "/api/v3/compute/test-echo/x",
    ] {
        let a = send(
            &db,
            &actions,
            Call::json("owner", uri, json!({ "amount": "1" })),
        )
        .await;
        assert!(
            matches!(a.status.as_u16(), 404 | 405),
            "POST {uri}: not routed, got {} {}",
            a.status,
            a.text
        );
    }
    assert_eq!(actions.calls(), 0);
    db.finish().await;
}

// ------------------------------------------------------------ typed money (CA7)

fn d(text: &str) -> Decimal {
    Decimal::from_str_exact(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

/// What a browser `type=number` input sends, without the exponent form.
#[test]
fn typed_decimal_reads_an_optional_minus_and_digits_with_an_optional_fraction() {
    for (text, want) in [
        ("0", "0"),
        ("7", "7"),
        ("12.5", "12.5"),
        ("0012.50", "12.5"),
        ("1.005", "1.005"),
        ("-12.34", "-12.34"),
        ("-0", "0"),
        ("-0.00", "0"),
        (".5", "0.5"),
        ("-.5", "-0.5"),
        ("0.000001", "0.000001"),
        ("1000000", "1000000"),
        (
            "0.0000000000000000000000000001",
            "0.0000000000000000000000000001",
        ),
        (
            "79228162514264337593543950335",
            "79228162514264337593543950335",
        ),
        (
            "-79228162514264337593543950335",
            "-79228162514264337593543950335",
        ),
    ] {
        assert_eq!(
            typed_decimal(text),
            Some(d(want)),
            "typed_decimal({text:?})"
        );
    }
}

#[test]
fn typed_decimal_refuses_signs_spaces_exponents_grouping_and_half_numbers() {
    for text in [
        "",
        " ",
        "-",
        ".",
        "-.",
        "5.",
        "-5.",
        "+1",
        "+.5",
        "--1",
        "-+1",
        "+-1",
        "1e3",
        "1E3",
        "1e-3",
        " 1",
        "1 ",
        " 1 ",
        "1 000",
        "1,000",
        "1_000",
        "1.2.3",
        "..5",
        "0x10",
        "NaN",
        "Infinity",
        "-Infinity",
        "1.5\n",
        "\t1",
        "١٢",
        "๑",
        "−1",
        "1-",
        "- 1",
    ] {
        assert_eq!(typed_decimal(text), None, "typed_decimal({text:?})");
    }
}

#[test]
fn typed_decimal_is_none_past_the_96_bit_decimal() {
    for text in [
        "79228162514264337593543950336",
        "-79228162514264337593543950336",
        "100000000000000000000000000000",
        "99999999999999999999999999999999999",
    ] {
        assert_eq!(typed_decimal(text), None, "typed_decimal({text:?})");
    }
}

/// v2's `Math.round(x × 100)` on the exact decimal: `floor(x × 100 + ½)`, a
/// half toward +∞.
#[test]
fn round_cents_rounds_a_half_toward_plus_infinity() {
    for (x, cents) in [
        ("0", 0),
        ("0.005", 1),
        ("-0.005", 0),
        ("-0.015", -1),
        ("0.015", 2),
        ("0.025", 3),
        ("-0.025", -2),
        ("0.004", 0),
        ("-0.004", 0),
        ("0.0049999", 0),
        ("-0.0050001", -1),
        ("1.005", 101),
        ("1.234", 123),
        ("-1.235", -123),
        ("-1.236", -124),
        ("12.345", 1235),
        ("-12.345", -1234),
        ("2.5", 250),
        ("-2.5", -250),
        ("-0.0000000000000000000000000001", 0),
        ("100000000000000000000.005", 10_000_000_000_000_000_000_001),
        (
            "792281625142643375935439503.35",
            79_228_162_514_264_337_593_543_950_335,
        ),
        (
            "-792281625142643375935439503.35",
            -79_228_162_514_264_337_593_543_950_335,
        ),
    ] {
        assert_eq!(round_cents(d(x)), Some(cents), "round_cents({x})");
    }
}

/// The cents must fit the 96-bit decimal again (`x × 100` overflows it).
#[test]
fn round_cents_is_none_when_the_cents_do_not_fit_the_96_bit_decimal() {
    for x in [
        d("792281625142643375935439503.4"),
        d("-792281625142643375935439503.4"),
        d("1000000000000000000000000000"),
        Decimal::MAX,
        Decimal::MIN,
    ] {
        assert_eq!(round_cents(x), None, "round_cents({x})");
    }
}

/// §4.2's decimal string: `-` only below zero, no grouping, two places.
#[test]
fn fixed2_writes_cents_as_a_plain_two_place_string_never_minus_zero() {
    for (cents, want) in [
        (0_i128, "0.00"),
        (-0, "0.00"),
        (5, "0.05"),
        (-5, "-0.05"),
        (-10, "-0.10"),
        (99, "0.99"),
        (-99, "-0.99"),
        (100, "1.00"),
        (-100, "-1.00"),
        (123_450, "1234.50"),
        (-123_450, "-1234.50"),
        (100_000_000, "1000000.00"),
        (
            79_228_162_514_264_337_593_543_950_335,
            "792281625142643375935439503.35",
        ),
        (i128::MAX, "1701411834604692317316873037158841057.27"),
        (i128::MIN, "-1701411834604692317316873037158841057.28"),
    ] {
        assert_eq!(fixed2(cents), want, "fixed2({cents})");
    }
}

/// Typed text → cents → text: what a calculator shows for what was typed.
#[test]
fn a_negative_amount_that_rounds_to_zero_is_written_without_a_sign() {
    for text in ["-0", "-0.00", "-0.004", "-0.005", "-.001"] {
        let x = typed_decimal(text).unwrap_or_else(|| panic!("{text} is typed"));
        let cents = round_cents(x).unwrap_or_else(|| panic!("{text} fits"));
        assert_eq!(fixed2(cents), "0.00", "{text}");
    }
    let cents = round_cents(typed_decimal("-0.015").expect("typed")).expect("fits");
    assert_eq!(fixed2(cents), "-0.01");
    let cents = round_cents(typed_decimal("1.005").expect("typed")).expect("fits");
    assert_eq!(
        fixed2(cents),
        "1.01",
        "exact decimals, not v2's float 1.00 (§12.20.7)"
    );
}

/// `toFixed(6)` by the same half-toward-+∞ rule.
#[test]
fn fixed6_rounds_to_six_places_a_half_toward_plus_infinity() {
    for (x, want) in [
        ("32.123456789", "32.123457"),
        ("2", "2.000000"),
        ("0.1", "0.100000"),
        ("35.5", "35.500000"),
        ("0.000001", "0.000001"),
        ("0.0000005", "0.000001"),
        ("0.0000004999", "0.000000"),
        ("1.0000005", "1.000001"),
        ("36.1234565", "36.123457"),
        ("36.1234575", "36.123458"),
        ("-1.0000005", "-1.000000"),
        ("-1.0000015", "-1.000001"),
        ("-1.0000016", "-1.000002"),
        ("-2.5", "-2.500000"),
        ("1234567.1234565", "1234567.123457"),
        ("1000000", "1000000.000000"),
        ("79228162514264.1234565", "79228162514264.123457"),
    ] {
        assert_eq!(fixed6(d(x)), want, "fixed6({x})");
    }
}
