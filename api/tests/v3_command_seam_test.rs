//! Ticket #597 (CM-1) — the v3 command seam: `CommandAction` + `CommandEndpoint`, and
//! the idempotency store it calls around `run`. Written before the
//! implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` (#547, #548, #549), §6 "The command seam
//! (shared, first used here)", §5.1 `idempotency_records`, §5.6 "Concurrency and
//! idempotency", §9 CM-1 row and "Up-front tests per ticket" (CM-1).
//!
//! What §6 fixes, and these tests pin:
//! - `CommandEndpoint::register` serves `METHOD ROUTE`, the route the command
//!   declares — not a route of its own making, and no other method on it.
//! - `x-xchange-request: 1` is required, as on the compute seam (`same_origin`,
//!   `400 originRejected`).
//! - A missing `Idempotency-Key` is `400 validationFailed`, field
//!   `idempotencyKey`, and `run` is not called.
//! - Policy runs through `authenticate_and_authorize` (`view.rs:331`): a command
//!   refuses exactly whom a view with the same `INHERITS` refuses, with the same
//!   status and body.
//! - `idempotency_records` is keyed `(org_id, operation, key)`: the same key and
//!   the same body replay the stored answer and `run` is called once; the same
//!   key and a different body are `idempotencyConflict`; one key used by two
//!   operations does not collide.
//!
//! Signatures this test assumes (choices where §6 leaves the exact form open):
//! - `xchange::presentation::http::v3::command::{CommandAction, CommandEndpoint}`;
//!   `CommandAction` has exactly §6's items (`NAME`, `INHERITS`, `Input`,
//!   `Output`, `ROUTE`, `METHOD: actix_web::http::Method`, `run`), and `run` may
//!   be implemented as an `async fn`. `run(&self, tx: &mut sqlx::Transaction<'_,
//!   Postgres>, branch, who, input, key)`: the endpoint opens the transaction
//!   (`BranchTx::run` of the branch) and `finish_in` writes the answer in it
//!   (#691, the lane controller's ruling X on #597).
//! - `CommandEndpoint::register(cfg: &mut web::ServiceConfig, command: Arc<C>)`,
//!   the same shape as `ComputeEndpoint::register`. The endpoint reaches its
//!   `PgIdempotencyStore` through the application's `AppState` (its pool), the
//!   way `require_branch` reaches the database; the test passes no store.
//! - `IdempotencyKey` lives in `xchange::infrastructure::idempotency::store`
//!   (beside the `IdempotencyStore` port) and has `as_str(&self) -> &str`; `run`
//!   receives the key exactly as the `Idempotency-Key` header sent it.
//! - A path parameter of `ROUTE` (`{id}`) reaches `run` as the `Input` field of
//!   the same name: the endpoint puts the path parameters into the body object
//!   before reading it as `Input`. The request hash therefore covers the path
//!   parameters too, so the same key on another `{id}` is a conflict, never a
//!   replay of the other object's answer.
//! - `operation` in `idempotency_records` is the command's `NAME`.
//! - An accepted command answers `200` with the `Output` DTO itself as the whole
//!   body, JSON, `no-store` (as the compute seam answers). A replay answers the
//!   same status and the same body.
//! - `idempotencyConflict` is `409`. The message of `validationFailed` and of
//!   `idempotencyConflict` is not pinned (any non-empty text).
//! - An empty `Idempotency-Key` value counts as missing.
//! - Policy and the origin rule run before the key is claimed: a refused call
//!   writes no `idempotency_records` row.
//! - With no branch named, the command runs on `Policy::branch(Some(&who), None)`.
//!
//! The tests need a real PostgreSQL: `TEST_DATABASE_URL` must name an existing
//! database whose name ends in `_test`. They fail without it; they do not skip.
//! Each test works in a schema of its own.

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
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::idempotency::store::IdempotencyKey;
use xchange::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::pages::{SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const KEY: &str = "idempotency-key";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

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
                 The v3 command seam tests need a real PostgreSQL and do not skip. Point it \
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
        let schema = unique("v3command");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B02', 'B02')")
            .execute(&pool)
            .await
            .expect("insert the branches this file names into branches");
        Self { pool, schema }
    }

    /// `(operation, key)` of every `idempotency_records` row, in that order.
    async fn records(&self) -> Vec<(String, String)> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT operation, key FROM idempotency_records ORDER BY operation, key",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read idempotency_records (migration 3 creates it)")
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

/// The actor the policy is asked about on `path`, by the rule of `view.rs:331`.
fn actor_on(path: EntryPath, spec: &str) -> Option<Actor> {
    let s = sessions_of(spec);
    match path {
        EntryPath::StaffRpc => s.staff,
        _ => s.owner.or(s.staff),
    }
}

// ------------------------------------------------------------ test commands

/// What every test command takes. `id` is the route's `{id}`, put into the
/// body object by the endpoint; the client sends only `amount`.
#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
struct DraftInput {
    id: String,
    amount: String,
}

/// What every test command answers: which command ran, on what, for whom,
/// under which key, and the how-many-th run of any test command it was (so a
/// second run can never pass for a replay).
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct Done {
    operation: String,
    id: String,
    amount: String,
    branch: String,
    who: String,
    key: String,
    run: usize,
}

/// Counts its runs and answers `Done`; one type, four commands.
struct Recording<const K: u8> {
    calls: Arc<AtomicUsize>,
}

impl<const K: u8> Recording<K> {
    fn record(
        &self,
        operation: &str,
        branch: &BranchId,
        who: &AccessActor,
        input: DraftInput,
        key: &IdempotencyKey,
    ) -> Done {
        let run = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        Done {
            operation: operation.to_owned(),
            id: input.id,
            amount: input.amount,
            branch: branch.as_str().to_owned(),
            who: who.user_id.clone(),
            key: key.as_str().to_owned(),
            run,
        }
    }
}

/// `POST /api/test-drafts/{id}/submit`, `(Rpc, "adminSnapshot")`: the owner and
/// every staff role — the same row as the real view `admin_branches`.
type SubmitDraft = Recording<0>;
/// `PUT /api/test-drafts/{id}`, `(Rpc, "rateDraft")` (re-expected for RE-7 #604: was `adminSaveRate`): a narrower row.
type SaveDraft = Recording<1>;
/// `POST /api/test-desk/{id}`, `(StaffRpc, "adminDeskConfig")`: staff session only.
type SaveDesk = Recording<2>;
/// `POST /api/test-employees/{id}`, `(OwnerRoute, "list")`: the same row as the
/// real view `admin_employees`.
type SaveEmployee = Recording<3>;

const SUBMIT_NAME: &str = "test-draft-submit";
const SAVE_NAME: &str = "test-draft-save";

impl CommandAction for SubmitDraft {
    const NAME: &'static str = SUBMIT_NAME;
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Input = DraftInput;
    type Output = Done;
    const ROUTE: &'static str = "/api/test-drafts/{id}/submit";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &AccessActor,
        input: DraftInput,
        key: IdempotencyKey,
    ) -> Result<Done, V3Error> {
        Ok(self.record(Self::NAME, branch, who, input, &key))
    }
}

impl CommandAction for SaveDraft {
    const NAME: &'static str = SAVE_NAME;
    // re-expected for RE-7 #604 by the ticket author: `adminSaveRate` is retired (design §9);
    // `rateDraft` has the same rows of `Policy` (the owner, and RATES)
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    type Input = DraftInput;
    type Output = Done;
    const ROUTE: &'static str = "/api/test-drafts/{id}";
    const METHOD: Method = Method::PUT;

    async fn run(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &AccessActor,
        input: DraftInput,
        key: IdempotencyKey,
    ) -> Result<Done, V3Error> {
        Ok(self.record(Self::NAME, branch, who, input, &key))
    }
}

impl CommandAction for SaveDesk {
    const NAME: &'static str = "test-desk-save";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::StaffRpc, "adminDeskConfig");
    type Input = DraftInput;
    type Output = Done;
    const ROUTE: &'static str = "/api/test-desk/{id}";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &AccessActor,
        input: DraftInput,
        key: IdempotencyKey,
    ) -> Result<Done, V3Error> {
        Ok(self.record(Self::NAME, branch, who, input, &key))
    }
}

impl CommandAction for SaveEmployee {
    const NAME: &'static str = "test-employee-save";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::OwnerRoute, "list");
    type Input = DraftInput;
    type Output = Done;
    const ROUTE: &'static str = "/api/test-employees/{id}";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &AccessActor,
        input: DraftInput,
        key: IdempotencyKey,
    ) -> Result<Done, V3Error> {
        Ok(self.record(Self::NAME, branch, who, input, &key))
    }
}

/// One counter of every run any test command made.
struct Commands {
    calls: Arc<AtomicUsize>,
}

impl Commands {
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
/// commands registered beside it. `Host` is always `xc.test`.
struct Call {
    method: Method,
    uri: String,
    who: String,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl Call {
    /// `method uri` as `who` from the same origin, with the marker header, a
    /// JSON content type, the JSON `body` and the `Idempotency-Key` `key` —
    /// what the client's `command(method, path, body, key)` sends.
    fn command(who: &str, method: Method, uri: &str, body: Value, key: &str) -> Self {
        Self {
            method,
            uri: uri.to_owned(),
            who: who.to_owned(),
            headers: vec![
                ("x-xchange-request", "1".to_owned()),
                ("origin", SAME_ORIGIN.to_owned()),
                ("content-type", "application/json".to_owned()),
                (KEY, key.to_owned()),
            ],
            body: body.to_string().into_bytes(),
        }
    }

    /// `POST uri` with `{"amount": amount}` under `key`.
    fn post(who: &str, uri: &str, amount: &str, key: &str) -> Self {
        Self::command(who, Method::POST, uri, json!({ "amount": amount }), key)
    }

    /// A `GET` of a view, as the pages read one: no body, no key, no marker.
    fn view(who: &str, name: &str) -> Self {
        Self {
            method: Method::GET,
            uri: format!("/api/v3/views/{name}"),
            who: who.to_owned(),
            headers: Vec::new(),
            body: Vec::new(),
        }
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

async fn send(db: &TestDb, commands: &Commands, call: Call) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let calls = commands.calls.clone();
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| {
                CommandEndpoint::register(
                    cfg,
                    Arc::new(SubmitDraft {
                        calls: calls.clone(),
                    }),
                );
                CommandEndpoint::register(
                    cfg,
                    Arc::new(SaveDraft {
                        calls: calls.clone(),
                    }),
                );
                CommandEndpoint::register(
                    cfg,
                    Arc::new(SaveDesk {
                        calls: calls.clone(),
                    }),
                );
                CommandEndpoint::register(cfg, Arc::new(SaveEmployee { calls }));
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

const SUBMIT_D1: &str = "/api/test-drafts/d-1/submit";
const SUBMIT_D2: &str = "/api/test-drafts/d-2/submit";
const DRAFT_D1: &str = "/api/test-drafts/d-1";
const DESK_D1: &str = "/api/test-desk/d-1";
const EMPLOYEE_D1: &str = "/api/test-employees/d-1";

/// What a test command answers for these arguments.
fn done(
    operation: &str,
    id: &str,
    amount: &str,
    branch: &str,
    who: &str,
    key: &str,
    run: usize,
) -> Value {
    json!({
        "operation": operation,
        "id": id,
        "amount": amount,
        "branch": branch,
        "who": who,
        "key": key,
        "run": run,
    })
}

/// `200`, the `Output` DTO as the whole body, JSON, `no-store`.
fn is_answer(a: &Answer, want: &Value, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: status; body {}", a.text);
    assert_eq!(&a.body, want, "{what}: exactly the command's answer");
    assert!(
        a.content_type
            .as_deref()
            .is_some_and(|c| c.starts_with("application/json")),
        "{what}: content type {:?}",
        a.content_type
    );
    assert_eq!(a.cache_control.as_deref(), Some("no-store"), "{what}");
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

/// A refusal whose message these tests do not pin: status, code, field (or
/// none), a non-empty message, nothing else, `no-store`.
fn is_refused_with_any_message(
    a: &Answer,
    status: u16,
    code: &str,
    field: Option<&str>,
    what: &str,
) {
    assert_eq!(a.status.as_u16(), status, "{what}: status; body {}", a.text);
    let error = a.body["error"]
        .as_object()
        .unwrap_or_else(|| panic!("{what}: an `error` object; body {}", a.text));
    assert_eq!(error.get("code"), Some(&json!(code)), "{what}: code");
    assert_eq!(
        error.get("field"),
        field.map(|f| json!(f)).as_ref(),
        "{what}: field"
    );
    assert!(
        error
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|m| !m.is_empty()),
        "{what}: a message; body {}",
        a.text
    );
    assert_eq!(
        error.len(),
        if field.is_some() { 3 } else { 2 },
        "{what}: no other keys"
    );
    assert_eq!(a.cache_control.as_deref(), Some("no-store"), "{what}");
}

fn is_origin_rejected(a: &Answer, what: &str) {
    is_refused(a, 400, "originRejected", None, "ORIGIN_REJECTED", what);
}

fn is_conflict(a: &Answer, what: &str) {
    is_refused_with_any_message(a, 409, "idempotencyConflict", None, what);
}

/// The owner's answer for `SubmitDraft` on `MAIN`.
fn owner_submit(id: &str, amount: &str, key: &str, run: usize) -> Value {
    done(SUBMIT_NAME, id, amount, "MAIN", "owner-1", key, run)
}

// ------------------------------------------------------------ the key

/// OR-4b (#767): the rule a route asks once it is on `Policy::authorize` (design §4.2): the caller
/// as B1 maps them (§3.3), naming `named` (D-4: a branch outside the caller's scope is refused).
fn blueprint_rule(
    actor: Option<&xc_auth::domain::policy::value_objects::actor::Actor>,
    path: xc_auth::domain::policy::value_objects::entry_path::EntryPath,
    action: &str,
    named: Option<&str>,
) -> Result<(), xc_auth::domain::policy::value_objects::policy_error::PolicyError> {
    use xc_auth::domain::policy::entities::policy::{Need, Policy};
    let act = xc_auth::domain::policy::value_objects::action::Action::new(action);
    if Policy::need(path, &act) == Some(Need::Custody) {
        // #582's rows stay on the v2 rule
        return Policy::allow(actor, path, &act);
    }
    let user = actor.map(|a| {
        xc_auth::domain::access::v3_roles::actor_from_v3(
            a,
            "XC",
            chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
        )
    });
    Policy::authorize(
        user.as_ref(),
        path,
        &act,
        named.map(xc_kernel::ids::BranchId::new).as_ref(),
        chrono::Utc::now(),
    )
    .map(|_| ())
}

/// §6: a missing `Idempotency-Key` is `400 validationFailed`, field
/// `idempotencyKey`; `run` is not called and nothing is stored.
#[actix_web::test]
async fn a_missing_idempotency_key_is_validation_failed_on_the_field_idempotency_key() {
    let db = TestDb::start().await;
    let commands = Commands::new();
    let cases = [
        (
            "no key, POST command",
            Call::post("owner", SUBMIT_D1, "5", "x").without(KEY),
        ),
        (
            "empty key, POST command",
            Call::post("owner", SUBMIT_D1, "5", "x").with(KEY, ""),
        ),
        (
            "no key, PUT command",
            Call::command(
                "owner",
                Method::PUT,
                DRAFT_D1,
                json!({ "amount": "5" }),
                "x",
            )
            .without(KEY),
        ),
        (
            "no key, staff on a staff command",
            Call::post("staff:FRONT:XC-B01", DESK_D1, "5", "x").without(KEY),
        ),
    ];
    for (what, call) in cases {
        let a = send(&db, &commands, call).await;
        is_refused_with_any_message(&a, 400, "validationFailed", Some("idempotencyKey"), what);
    }
    assert_eq!(commands.calls(), 0, "a call without a key runs nothing");
    assert_eq!(db.records().await, Vec::new(), "and stores nothing");

    // The same call with a key runs, and run receives that key.
    let a = send(
        &db,
        &commands,
        Call::post("owner", SUBMIT_D1, "5", "k-present"),
    )
    .await;
    is_answer(&a, &owner_submit("d-1", "5", "k-present", 1), "with a key");
    assert_eq!(commands.calls(), 1);
    db.finish().await;
}

// ------------------------------------------------------------ the origin

/// §6: `x-xchange-request: 1` is required, refused as the compute seam refuses
/// it (`same_origin`, `400 originRejected`). Nothing runs, nothing is stored.
#[actix_web::test]
async fn a_missing_marker_or_a_foreign_origin_is_refused_like_the_compute_seam() {
    let db = TestDb::start().await;
    let commands = Commands::new();
    let cases = [
        (
            "no marker",
            Call::post("owner", SUBMIT_D1, "5", "k-1").without("x-xchange-request"),
        ),
        (
            "marker 2",
            Call::post("owner", SUBMIT_D1, "5", "k-2").with("x-xchange-request", "2"),
        ),
        (
            "marker true",
            Call::post("owner", SUBMIT_D1, "5", "k-3").with("x-xchange-request", "true"),
        ),
        (
            "no origin",
            Call::post("owner", SUBMIT_D1, "5", "k-4").without("origin"),
        ),
        (
            "foreign origin",
            Call::post("owner", SUBMIT_D1, "5", "k-5").with("origin", "http://evil.test"),
        ),
        (
            "PUT command, no marker",
            Call::command(
                "owner",
                Method::PUT,
                DRAFT_D1,
                json!({ "amount": "5" }),
                "k-6",
            )
            .without("x-xchange-request"),
        ),
        (
            "staff command, no marker",
            Call::post("staff:FRONT:XC-B01", DESK_D1, "5", "k-7").without("x-xchange-request"),
        ),
    ];
    for (what, call) in cases {
        let a = send(&db, &commands, call).await;
        is_origin_rejected(&a, what);
    }
    assert_eq!(commands.calls(), 0, "a rejected origin runs nothing");
    assert_eq!(
        db.records().await,
        Vec::new(),
        "a rejected origin claims no key"
    );

    // The key of a rejected call is still free: the same key, sent properly, runs.
    let a = send(&db, &commands, Call::post("owner", SUBMIT_D1, "5", "k-1")).await;
    is_answer(&a, &owner_submit("d-1", "5", "k-1", 1), "k-1 sent properly");
    db.finish().await;
}

// ------------------------------------------------------------ who may call

/// §6: policy runs through `authenticate_and_authorize` (`view.rs:331`). A
/// command refuses exactly whom `Policy::allow` refuses for its `INHERITS`, with
/// the refusal the view core answers — and, where a real view has the same
/// `INHERITS`, with byte-for-byte that view's status and body.
/// `(route, method, the command's policy pair, a real view with the same pair)`.
type PolicyRow = (
    &'static str,
    Method,
    EntryPath,
    &'static str,
    &'static str,
    Option<&'static str>,
);

#[actix_web::test]
async fn a_command_refuses_exactly_whom_the_view_core_refuses_for_the_same_inherits() {
    let db = TestDb::start().await;
    let commands = Commands::new();
    let mut allowed = 0;
    let mut refused = 0;
    let mut compared_with_a_view = 0;
    // (route, method, the command's pair, a real view with the same pair)
    let rows: [PolicyRow; 4] = [
        (
            SUBMIT_D1,
            Method::POST,
            EntryPath::Rpc,
            "adminSnapshot",
            SUBMIT_NAME,
            Some("admin_branches"),
        ),
        (
            DRAFT_D1,
            Method::PUT,
            EntryPath::Rpc,
            // re-expected for RE-7 #604: was `adminSaveRate`, retired (design §9)
            "rateDraft",
            SAVE_NAME,
            None,
        ),
        (
            DESK_D1,
            Method::POST,
            EntryPath::StaffRpc,
            "adminDeskConfig",
            "test-desk-save",
            None,
        ),
        (
            EMPLOYEE_D1,
            Method::POST,
            EntryPath::OwnerRoute,
            "list",
            "test-employee-save",
            Some("admin_employees"),
        ),
    ];
    for (uri, method, path, action, name, view) in rows {
        for who in every_caller() {
            let key = unique("policy");
            let a = send(
                &db,
                &commands,
                Call::command(&who, method.clone(), uri, json!({ "amount": "7" }), &key),
            )
            .await;
            let what = format!("{method} {uri} as {who:?}");
            let actor = actor_on(path, &who);
            // OR-4b (#767): the route table's rule (C4: SYSTEM, ADMIN under B1, saves employees)
            match blueprint_rule(actor.as_ref(), path, action, None) {
                Ok(()) => {
                    allowed += 1;
                    let actor = actor.expect("allowed means signed in");
                    let branch = Policy::branch(Some(&actor), None);
                    assert_eq!(a.status, StatusCode::OK, "{what}: allowed; body {}", a.text);
                    assert_eq!(a.body["operation"], json!(name), "{what}");
                    assert_eq!(a.body["branch"], json!(branch.as_str()), "{what}: branch");
                    assert_eq!(
                        a.body["who"],
                        json!(actor.id()),
                        "{what}: the actor of its entry path"
                    );
                    assert_eq!(a.body["key"], json!(key), "{what}: the key run received");
                }
                Err(e) => {
                    refused += 1;
                    let status = e.status();
                    let message = e.to_string();
                    let code = V3Error::from(e).code;
                    is_refused(&a, status, code, None, &message, &what);
                    if let Some(view) = view {
                        let v = send(&db, &commands, Call::view(&who, view)).await;
                        assert_eq!(
                            (a.status, &a.body),
                            (v.status, &v.body),
                            "{what}: the same refusal as GET /api/v3/views/{view}"
                        );
                        compared_with_a_view += 1;
                    }
                }
            }
        }
    }
    assert_eq!(commands.calls(), allowed, "only the allowed callers ran");
    assert!(allowed >= 20, "allowed: {allowed}");
    assert!(refused >= 20, "refused: {refused}");
    assert!(
        compared_with_a_view >= 2,
        "refusals compared with a real view: {compared_with_a_view}"
    );
    assert_eq!(
        db.records().await.len(),
        allowed,
        "a refused caller claims no key; every allowed call stored one"
    );
    db.finish().await;
}

// ------------------------------------------------------------ replay and conflict

/// §5.6: the same key with the same body replays the stored answer; `run` is
/// called once however often the call is repeated.
#[actix_web::test]
async fn the_same_key_and_the_same_body_replay_the_stored_answer_and_run_once() {
    let db = TestDb::start().await;
    let commands = Commands::new();
    let first = send(
        &db,
        &commands,
        Call::post("owner", SUBMIT_D1, "5", "k-replay"),
    )
    .await;
    let want = owner_submit("d-1", "5", "k-replay", 1);
    is_answer(&first, &want, "first call");
    assert_eq!(commands.calls(), 1);

    for n in 2..=4 {
        let again = send(
            &db,
            &commands,
            Call::post("owner", SUBMIT_D1, "5", "k-replay"),
        )
        .await;
        is_answer(
            &again,
            &want,
            &format!("call {n}: the stored answer, run 1"),
        );
        assert_eq!(again.text, first.text, "call {n}: the same bytes");
    }
    assert_eq!(commands.calls(), 1, "run was called once");
    assert_eq!(
        db.records().await,
        vec![(SUBMIT_NAME.to_string(), "k-replay".to_string())],
        "one record, keyed by the command's NAME and the key"
    );

    // Another key is another command: it runs.
    let other = send(
        &db,
        &commands,
        Call::post("owner", SUBMIT_D1, "5", "k-other"),
    )
    .await;
    is_answer(
        &other,
        &owner_submit("d-1", "5", "k-other", 2),
        "another key",
    );
    assert_eq!(commands.calls(), 2);
    db.finish().await;
}

/// §5.6, §07: the same key with a different payload is refused
/// `idempotencyConflict`, `run` is not called again, and the stored answer is
/// kept. Another `{id}` on the route is a different payload too.
#[actix_web::test]
async fn the_same_key_and_a_different_body_is_idempotency_conflict_and_does_not_run() {
    let db = TestDb::start().await;
    let commands = Commands::new();
    let first = send(
        &db,
        &commands,
        Call::post("owner", SUBMIT_D1, "5", "k-conflict"),
    )
    .await;
    let want = owner_submit("d-1", "5", "k-conflict", 1);
    is_answer(&first, &want, "first call");

    let other_amount = send(
        &db,
        &commands,
        Call::post("owner", SUBMIT_D1, "6", "k-conflict"),
    )
    .await;
    is_conflict(&other_amount, "same key, another amount");

    let other_id = send(
        &db,
        &commands,
        Call::post("owner", SUBMIT_D2, "5", "k-conflict"),
    )
    .await;
    is_conflict(&other_id, "same key, same body, another {id}");

    let extra_field = send(
        &db,
        &commands,
        Call::command(
            "owner",
            Method::POST,
            SUBMIT_D1,
            json!({ "amount": "5", "note": "x" }),
            "k-conflict",
        ),
    )
    .await;
    is_conflict(&extra_field, "same key, a body with another field");
    assert_eq!(commands.calls(), 1, "no conflicting call ran");

    // The conflicts overwrote nothing: the first call still replays.
    let again = send(
        &db,
        &commands,
        Call::post("owner", SUBMIT_D1, "5", "k-conflict"),
    )
    .await;
    is_answer(&again, &want, "the first body again");
    assert_eq!(commands.calls(), 1);
    assert_eq!(
        db.records().await,
        vec![(SUBMIT_NAME.to_string(), "k-conflict".to_string())],
        "still one record"
    );
    db.finish().await;
}

/// §6: the store is keyed `(org_id, operation, key)` — one key used by two
/// commands is two records, each running once and replaying its own answer.
#[actix_web::test]
async fn one_key_used_by_two_operations_does_not_collide() {
    let db = TestDb::start().await;
    let commands = Commands::new();
    let save = |amount: &str| {
        Call::command(
            "owner",
            Method::PUT,
            DRAFT_D1,
            json!({ "amount": amount }),
            "k-shared",
        )
    };

    let submitted = send(
        &db,
        &commands,
        Call::post("owner", SUBMIT_D1, "5", "k-shared"),
    )
    .await;
    let want_submit = owner_submit("d-1", "5", "k-shared", 1);
    is_answer(&submitted, &want_submit, "submit under k-shared");

    // Same key, same body, same {id}: another operation, so it runs — neither a
    // replay of the submit nor a conflict with it.
    let saved = send(&db, &commands, save("5")).await;
    let want_save = done(SAVE_NAME, "d-1", "5", "MAIN", "owner-1", "k-shared", 2);
    is_answer(&saved, &want_save, "save under k-shared");
    assert_eq!(commands.calls(), 2);

    // Each replays its own answer.
    let a = send(
        &db,
        &commands,
        Call::post("owner", SUBMIT_D1, "5", "k-shared"),
    )
    .await;
    is_answer(&a, &want_submit, "submit replayed");
    let a = send(&db, &commands, save("5")).await;
    is_answer(&a, &want_save, "save replayed");
    assert_eq!(commands.calls(), 2, "both replays ran nothing");

    // And a conflict on one is not a conflict on the other.
    let a = send(&db, &commands, save("9")).await;
    is_conflict(&a, "save, another amount");
    let a = send(
        &db,
        &commands,
        Call::post("owner", SUBMIT_D1, "5", "k-shared"),
    )
    .await;
    is_answer(&a, &want_submit, "submit still replays");
    assert_eq!(commands.calls(), 2);

    assert_eq!(
        db.records().await,
        vec![
            (SAVE_NAME.to_string(), "k-shared".to_string()),
            (SUBMIT_NAME.to_string(), "k-shared".to_string()),
        ],
        "two records, one per operation"
    );
    db.finish().await;
}

// ------------------------------------------------------------ the route

/// §6: `CommandEndpoint::register` serves `METHOD ROUTE` — the command's own
/// route with its path parameter, at its declared method, and nothing else.
#[actix_web::test]
async fn a_command_is_served_at_its_declared_method_and_route_only() {
    let db = TestDb::start().await;
    let commands = Commands::new();

    let a = send(&db, &commands, Call::post("owner", SUBMIT_D1, "5", "r-1")).await;
    is_answer(&a, &owner_submit("d-1", "5", "r-1", 1), "POST submit d-1");
    let a = send(
        &db,
        &commands,
        Call::post("owner", "/api/test-drafts/X-42/submit", "5", "r-2"),
    )
    .await;
    is_answer(
        &a,
        &owner_submit("X-42", "5", "r-2", 2),
        "POST submit X-42: the path parameter reaches run",
    );
    assert!(
        Policy::allow(
            Some(&Actor::owner("owner-1")),
            EntryPath::Rpc,
            &Action::new("rateDraft")
        )
        .is_ok(),
        "precondition: the owner may call rateDraft (RE-7 #604: was adminSaveRate)"
    );
    let a = send(
        &db,
        &commands,
        Call::command(
            "owner",
            Method::PUT,
            DRAFT_D1,
            json!({ "amount": "5" }),
            "r-3",
        ),
    )
    .await;
    is_answer(
        &a,
        &done(SAVE_NAME, "d-1", "5", "MAIN", "owner-1", "r-3", 3),
        "PUT draft d-1: a METHOD other than POST is honoured",
    );
    assert_eq!(commands.calls(), 3);

    let mut n = 0;
    let mut not_routed = |a: &Answer, what: String| {
        assert!(
            matches!(a.status.as_u16(), 404 | 405),
            "{what}: not routed, got {} {}",
            a.status,
            a.text
        );
        n += 1;
    };
    for method in [Method::GET, Method::PUT, Method::DELETE, Method::PATCH] {
        let a = send(
            &db,
            &commands,
            Call::post("owner", SUBMIT_D1, "5", &unique("r")).method(method.clone()),
        )
        .await;
        not_routed(&a, format!("{method} {SUBMIT_D1}"));
    }
    for method in [Method::GET, Method::POST, Method::DELETE, Method::PATCH] {
        let a = send(
            &db,
            &commands,
            Call::post("owner", DRAFT_D1, "5", &unique("r")).method(method.clone()),
        )
        .await;
        not_routed(&a, format!("{method} {DRAFT_D1}"));
    }
    for uri in [
        "/api/test-drafts/submit",
        "/api/test-drafts/d-1/submit/x",
        "/api/v3/compute/test-draft-submit",
        "/api/v3/commands/test-draft-submit",
        "/api/v3/test-draft-submit",
    ] {
        let a = send(&db, &commands, Call::post("owner", uri, "5", &unique("r"))).await;
        not_routed(&a, format!("POST {uri}"));
    }
    assert_eq!(n, 13);
    assert_eq!(commands.calls(), 3, "no unrouted call ran");
    db.finish().await;
}
