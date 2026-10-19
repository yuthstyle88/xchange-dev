//! Specification tests for V2-cash-profit (#510; docs/design/xchange-v3-web.md
//! §12.20.5, rows P5 and P6 of §12.20.1): the compute action `profit_summary`,
//! `POST /api/v3/compute/profit_summary`, which takes eight typed figures and
//! answers the realized profit, the revaluation difference and the withdrawable
//! amount, reading nothing.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! The behaviour contract is v2's `summarize` (`xchange-v2-source@4d0bdbb`,
//! `src/devices-public/profit-summary-core.js:2-8`), on exact decimals:
//!
//! 1. each of `realized`, `foreignValue`, `foreignCost`, `netAssets`, `capital`,
//!    `reserve`, `cash`, `cashMinimum` present, not `null`, not `""`, not
//!    whitespace-only, a `typed_decimal`, and its cents fit — else
//!    `400 profitIncomplete`;
//! 2. then none of the six (`foreignValue` … `cashMinimum`) below zero — else
//!    `400 profitNegative`; `realized` and `netAssets` may be negative;
//! 3. `c(k) = round_cents(k)`; `realized = fixed2(c(realized))`,
//!    `unrealized = fixed2(c(foreignValue) − c(foreignCost))`,
//!    `withdrawable = fixed2(max(0, min(c(realized),
//!    c(netAssets) − c(capital) − c(reserve), c(cash) − c(cashMinimum))))`.
//!
//! `field` is the failing key when exactly one key fails that step, and absent
//! when several do (§4.3).
//!
//! Every request goes through the server's whole route table
//! (`bootstrap::configure`, which runs `configure_v3` over the generated
//! `ACTIONS`), so these tests also show that the action is found by the build
//! and reachable.
//!
//! Left to the implementer (ticket #510 §9): a figure sent as a JSON number
//! (`400 invalidRequest`) and `branch_versions` unchanged by a call.
//!
//! The tests need a real PostgreSQL (the endpoint checks the branch before the
//! action runs): `TEST_DATABASE_URL` must name an existing database whose name
//! ends in `_test`. They fail without it; they do not skip. Each test works in
//! a schema of its own.

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
use serde_json::{Map, Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as AccessActor, Role as AccessRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{fixed2, round_cents, typed_decimal};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http::v3::ACTIONS;
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const URI: &str = "/api/v3/compute/profit_summary";

/// v2's sentences (`profit-summary-core.js:4`, `:5`), byte for byte.
const INCOMPLETE: &str = "กรอกข้อมูลกำไรและเงินสำรองให้ครบก่อนคำนวณ";
const NEGATIVE: &str = "ต้นทุน ทุน และเงินสำรองต้องไม่ติดลบ";

/// The eight wire keys, in v2's order (`profit-summary-core.js:3`).
const KEYS: [&str; 8] = [
    "realized",
    "foreignValue",
    "foreignCost",
    "netAssets",
    "capital",
    "reserve",
    "cash",
    "cashMinimum",
];

/// The six that must not be below zero (`profit-summary-core.js:5`).
const NON_NEGATIVE: [&str; 6] = [
    "foreignValue",
    "foreignCost",
    "capital",
    "reserve",
    "cash",
    "cashMinimum",
];

/// The largest figure whose cents fit the 96-bit decimal: its cents are
/// 79,228,162,514,264,337,593,543,950,335.
const EDGE: &str = "792281625142643375935439503.35";

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
                 The profit summary tests need a real PostgreSQL and do not skip. Point it \
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
        let schema = unique("v3profit");
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
struct HeaderCallers;

fn staff_caller_of(spec: &str) -> AccessActor {
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role_name = parts.next().expect("a role");
    let roles: &[AccessRole] = match role_name {
        "FRONT" => &[AccessRole::Teller],
        "BACKEND" | "MANAGER" => &[AccessRole::BranchManager],
        "RATES" => &[AccessRole::RateManager],
        "FINANCE" => &[AccessRole::Finance],
        "SYSTEM" => &[AccessRole::Admin],
        "MONITOR" | "DOCUMENTS" | "DOCUMENT_REVIEW" => &[AccessRole::VerificationReviewer],
        other => panic!("unknown role {other:?}"),
    };
    let branch = parts.next().expect("a branch");
    test_callers::staff("employee-7", roles, BranchId::new(branch))
}

fn callers_of(spec: &str) -> Callers {
    if spec.is_empty() {
        return test_callers::nobody();
    }
    if spec == "owner" {
        return test_callers::callers(Some(test_callers::owner("owner-1")), None);
    }
    if let Some(staff) = spec.strip_prefix("owner+") {
        return test_callers::callers(
            Some(test_callers::owner("owner-1")),
            Some(staff_caller_of(staff)),
        );
    }
    test_callers::callers(None, Some(staff_caller_of(spec)))
}

fn staff_of(spec: &str) -> Option<Actor> {
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = Role::parse(parts.next().expect("a role")).expect("a known role");
    let branch = parts.next().expect("a branch");
    Actor::staff("employee-7", role, BranchId::new(branch))
}

fn legacy_actor_of(spec: &str) -> Option<Actor> {
    if spec.is_empty() {
        return None;
    }
    if spec == "owner" || spec.starts_with("owner+") {
        return Some(Actor::owner("owner-1"));
    }
    staff_of(spec)
}

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { callers_of(&spec) })
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

// ------------------------------------------------------------ requests

/// `method URI` as `who` from the same origin, with the marker header and a
/// JSON content type — what `ApiClient.act` sends.
struct Call {
    method: Method,
    who: String,
    body: Vec<u8>,
}

impl Call {
    fn post(who: &str, body: &Value) -> Self {
        Self {
            method: Method::POST,
            who: who.to_owned(),
            body: body.to_string().into_bytes(),
        }
    }

    fn owner(body: &Value) -> Self {
        Self::post("owner", body)
    }
}

struct Answer {
    status: StatusCode,
    content_type: Option<String>,
    cache_control: Option<String>,
    text: String,
    body: Value,
}

/// Sends every call, in order, to one instance of the whole application.
async fn send_all(db: &TestDb, calls: Vec<Call>) -> Vec<Answer> {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut out = Vec::with_capacity(calls.len());
    for call in calls {
        let mut req = atest::TestRequest::default()
            .method(call.method.clone())
            .uri(URI)
            .insert_header((header::HOST, HOST))
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"));
        if !call.who.is_empty() {
            req = req.insert_header((AS, call.who.as_str()));
        }
        let resp = atest::call_service(&app, req.set_payload(call.body).to_request()).await;
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
        out.push(Answer {
            status,
            content_type,
            cache_control,
            text,
            body,
        });
    }
    out
}

// ------------------------------------------------------------ bodies

/// Design §12.20.5's worked example: `realized 1000`, `foreignValue 5000`,
/// `foreignCost 4200`, `netAssets 20000`, `capital 15000`, `reserve 2000`,
/// `cash 8000`, `cashMinimum 5000`.
fn worked() -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in [
        ("realized", "1000"),
        ("foreignValue", "5000"),
        ("foreignCost", "4200"),
        ("netAssets", "20000"),
        ("capital", "15000"),
        ("reserve", "2000"),
        ("cash", "8000"),
        ("cashMinimum", "5000"),
    ] {
        m.insert(k.to_owned(), json!(v));
    }
    m
}

/// The worked example with some keys set to the given JSON values.
fn worked_with(overrides: &[(&str, Value)]) -> Value {
    let mut m = worked();
    for (k, v) in overrides {
        m.insert((*k).to_owned(), v.clone());
    }
    Value::Object(m)
}

/// The worked example with some keys set to the given strings.
fn worked_str(overrides: &[(&str, &str)]) -> Value {
    let o: Vec<(&str, Value)> = overrides.iter().map(|(k, v)| (*k, json!(v))).collect();
    worked_with(&o)
}

/// The worked example without `key` at all.
fn worked_without(keys: &[&str]) -> Value {
    let mut m = worked();
    for k in keys {
        m.remove(*k);
    }
    Value::Object(m)
}

fn summary(realized: &str, unrealized: &str, withdrawable: &str) -> Value {
    json!({ "realized": realized, "unrealized": unrealized, "withdrawable": withdrawable })
}

// ------------------------------------------------------------ assertions

/// `200`, exactly `{realized, unrealized, withdrawable}`, JSON, `no-store`.
fn is_summary(a: &Answer, want: Value, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: status; body {}", a.text);
    assert_eq!(a.body, want, "{what}: exactly the summary; body {}", a.text);
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

fn is_incomplete(a: &Answer, field: Option<&str>, what: &str) {
    is_refused(a, 400, "profitIncomplete", field, INCOMPLETE, what);
}

fn is_negative(a: &Answer, field: Option<&str>, what: &str) {
    is_refused(a, 400, "profitNegative", field, NEGATIVE, what);
}

/// Every pair of distinct keys from `keys`.
fn pairs<'a>(keys: &[&'a str]) -> Vec<(&'a str, &'a str)> {
    let mut out = Vec::new();
    for (i, a) in keys.iter().enumerate() {
        for b in &keys[i + 1..] {
            out.push((*a, *b));
        }
    }
    out
}

// ------------------------------------------------------------ discovery and reach

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

/// §12.20.3 *Discovery*: the action is one file in `http/v3/actions/`, listed
/// in the generated `ACTIONS`, and `configure_v3` routes it — `POST` only.
#[actix_web::test]
async fn the_action_is_in_actions_and_reachable_by_post_at_compute_profit_summary() {
    let names: Vec<&str> = ACTIONS.iter().map(|a| a.name).collect();
    assert!(
        names.contains(&"profit_summary"),
        "ACTIONS lists profit_summary; it lists {names:?}"
    );

    let db = TestDb::start().await;
    let mut get = Call::owner(&Value::Object(worked()));
    get.method = Method::GET;
    let answers = send_all(&db, vec![Call::owner(&Value::Object(worked())), get]).await;
    is_summary(
        &answers[0],
        summary("1000.00", "800.00", "1000.00"),
        "POST /api/v3/compute/profit_summary",
    );
    assert!(
        matches!(answers[1].status.as_u16(), 404 | 405),
        "GET is not routed: status {}; body {}",
        answers[1].status,
        answers[1].text
    );
    db.finish().await;
}

/// §5.1: the action's types are exported to `profit_summary.ts` with v2's eight
/// keys, each optional (absent or `null` is "not filled in"), and the three
/// answers; no key named `branch` (the endpoint's reserved key).
#[test]
fn the_exported_types_carry_the_eight_optional_keys_and_the_three_answers() {
    let entry = ACTIONS
        .iter()
        .find(|a| a.name == "profit_summary")
        .expect("ACTIONS lists profit_summary");
    let dir = std::env::temp_dir().join(unique("v3profit_types"));
    std::fs::create_dir_all(&dir).expect("create a temporary directory");
    (entry.export_types)(&dir).expect("profit_summary exports its types");
    let ts = std::fs::read_to_string(dir.join("profit_summary.ts"))
        .expect("profit_summary.ts is written");
    for key in KEYS {
        assert!(
            ts.contains(&format!("{key}?:")),
            "the request key {key} is optional in profit_summary.ts:\n{ts}"
        );
    }
    for key in ["realized", "unrealized", "withdrawable"] {
        assert!(
            ts.contains(&format!("{key}:")),
            "the answer key {key} is in profit_summary.ts:\n{ts}"
        );
    }
    assert!(
        !ts.contains("branch"),
        "no request key named branch (§12.20.3 step 4):\n{ts}"
    );
    std::fs::remove_dir_all(&dir).expect("remove the temporary directory");
}

// ------------------------------------------------------------ who may call

/// CA4: `INHERITS = (Rpc, "adminSnapshot")` — every caller the policy allows
/// for that pair gets the same summary, whatever branch it names (the action
/// reads nothing, CA6); every other caller gets the policy's refusal.
#[actix_web::test]
async fn whoever_the_policy_allows_for_admin_snapshot_gets_the_same_summary() {
    let db = TestDb::start().await;
    let mut calls = Vec::new();
    let mut cases = Vec::new();
    for who in every_caller() {
        for body in [
            Value::Object(worked()),
            worked_with(&[("branch", json!("XC-B02"))]),
        ] {
            calls.push(Call::post(&who, &body));
            cases.push((who.clone(), body.get("branch").is_some()));
        }
    }
    let answers = send_all(&db, calls).await;
    let mut allowed = 0;
    let mut refused = 0;
    for ((who, named), a) in cases.iter().zip(&answers) {
        let what = format!("as {who:?}, naming a branch: {named}");
        let actor = legacy_actor_of(who);
        // OR-4b (#767): the route table's rule, naming XC-B02 where the body does (D-4: staff of
        // XC-B01 are refused it)
        match blueprint_rule(
            actor.as_ref(),
            EntryPath::Rpc,
            "adminSnapshot",
            named.then_some("XC-B02"),
        ) {
            Ok(()) => {
                allowed += 1;
                is_summary(a, summary("1000.00", "800.00", "1000.00"), &what);
            }
            Err(e) => {
                refused += 1;
                let status = e.status();
                let message = e.to_string();
                let code = V3Error::from(e).code;
                is_refused(a, status, code, None, &message, &what);
            }
        }
    }
    assert!(allowed >= 20, "allowed: {allowed}");
    assert!(refused >= 2, "refused: {refused}");
    db.finish().await;
}

// ------------------------------------------------------------ the answer

/// §12.20.5's worked examples, and unknown keys are ignored.
#[actix_web::test]
async fn the_worked_examples_answer_fixed2_strings() {
    let db = TestDb::start().await;
    let cases = [
        (
            "worked example",
            Value::Object(worked()),
            summary("1000.00", "800.00", "1000.00"),
        ),
        (
            "netAssets 16000",
            worked_str(&[("netAssets", "16000")]),
            summary("1000.00", "800.00", "0.00"),
        ),
        (
            "foreignCost 5200",
            worked_str(&[("foreignCost", "5200")]),
            summary("1000.00", "-200.00", "1000.00"),
        ),
        (
            "with two decimals",
            worked_str(&[
                ("realized", "1000.00"),
                ("foreignValue", "5000.00"),
                ("foreignCost", "4200.00"),
            ]),
            summary("1000.00", "800.00", "1000.00"),
        ),
        (
            "an unknown key is ignored",
            worked_with(&[("note", json!("x")), ("extra", json!(5))]),
            summary("1000.00", "800.00", "1000.00"),
        ),
    ];
    let calls = cases.iter().map(|(_, b, _)| Call::owner(b)).collect();
    let answers = send_all(&db, calls).await;
    for ((what, _, want), a) in cases.iter().zip(&answers) {
        is_summary(a, want.clone(), what);
    }
    db.finish().await;
}

/// P5 (3): `withdrawable = max(0, min(c(realized), c(netAssets) − c(capital) −
/// c(reserve), c(cash) − c(cashMinimum)))` — each term can be the least, each
/// of its figures counts, and below zero is `"0.00"`.
#[actix_web::test]
async fn withdrawable_is_the_least_of_its_three_terms_and_never_below_zero() {
    let db = TestDb::start().await;
    let cases: Vec<(&str, Value, Value)> = vec![
        (
            "realized is the least",
            worked_str(&[("realized", "999.99")]),
            summary("999.99", "800.00", "999.99"),
        ),
        (
            "netAssets − capital − reserve is the least",
            worked_str(&[("netAssets", "17500")]),
            summary("1000.00", "800.00", "500.00"),
        ),
        (
            "capital counts",
            worked_str(&[("netAssets", "17500"), ("capital", "15400")]),
            summary("1000.00", "800.00", "100.00"),
        ),
        (
            "reserve counts",
            worked_str(&[("netAssets", "17500"), ("reserve", "2300")]),
            summary("1000.00", "800.00", "200.00"),
        ),
        (
            "cash − cashMinimum is the least",
            worked_str(&[("cash", "5300")]),
            summary("1000.00", "800.00", "300.00"),
        ),
        (
            "cashMinimum counts",
            worked_str(&[("cash", "5300"), ("cashMinimum", "5250")]),
            summary("1000.00", "800.00", "50.00"),
        ),
        (
            "the netAssets term below zero",
            worked_str(&[("netAssets", "16000")]),
            summary("1000.00", "800.00", "0.00"),
        ),
        (
            "the cash term below zero",
            worked_str(&[("cash", "4000")]),
            summary("1000.00", "800.00", "0.00"),
        ),
        (
            "cash equal to cashMinimum",
            worked_str(&[("cash", "5000")]),
            summary("1000.00", "800.00", "0.00"),
        ),
        (
            "realized below zero",
            worked_str(&[("realized", "-50")]),
            summary("-50.00", "800.00", "0.00"),
        ),
        (
            "netAssets below zero",
            worked_str(&[("netAssets", "-5000")]),
            summary("1000.00", "800.00", "0.00"),
        ),
        (
            "realized and netAssets below zero together",
            worked_str(&[("realized", "-1000"), ("netAssets", "-5000")]),
            summary("-1000.00", "800.00", "0.00"),
        ),
        (
            "all zero",
            worked_str(&[
                ("realized", "0"),
                ("foreignValue", "0"),
                ("foreignCost", "0"),
                ("netAssets", "0"),
                ("capital", "0"),
                ("reserve", "0"),
                ("cash", "0"),
                ("cashMinimum", "0"),
            ]),
            summary("0.00", "0.00", "0.00"),
        ),
    ];
    let calls = cases.iter().map(|(_, b, _)| Call::owner(b)).collect();
    let answers = send_all(&db, calls).await;
    for ((what, _, want), a) in cases.iter().zip(&answers) {
        is_summary(a, want.clone(), what);
    }
    db.finish().await;
}

/// P5 (3): every figure is rounded to cents on its own, before any subtraction
/// or comparison — v2's `cents(k)` is applied per key.
#[actix_web::test]
async fn each_figure_is_rounded_to_cents_before_it_is_combined() {
    let db = TestDb::start().await;
    let cases: Vec<(&str, Value, Value)> = vec![
        (
            // 1 − 0 cents; the difference first would be 0.001 → 0.00
            "unrealized: foreignValue 0.005, foreignCost 0.004",
            worked_str(&[("foreignValue", "0.005"), ("foreignCost", "0.004")]),
            summary("1000.00", "0.01", "1000.00"),
        ),
        (
            // 0 − 1 cents; the difference first would be −0.001 → 0.00
            "unrealized: foreignValue 0.004, foreignCost 0.005",
            worked_str(&[("foreignValue", "0.004"), ("foreignCost", "0.005")]),
            summary("1000.00", "-0.01", "1000.00"),
        ),
        (
            // 1001 − 500 − 0 cents; the sum first would be 4.997 → 5.00
            "withdrawable: netAssets 10.005, capital 5.004, reserve 0.004",
            worked_str(&[
                ("netAssets", "10.005"),
                ("capital", "5.004"),
                ("reserve", "0.004"),
            ]),
            summary("1000.00", "800.00", "5.01"),
        ),
        (
            // 301 − 100 cents; the difference first would be 2.001 → 2.00
            "withdrawable: cash 3.005, cashMinimum 1.004",
            worked_str(&[("cash", "3.005"), ("cashMinimum", "1.004")]),
            summary("1000.00", "800.00", "2.01"),
        ),
        (
            // min over cents: realized 2.005 → 201 cents, cash term 201 cents
            "withdrawable: realized 2.005 against a cash term of 2.01",
            worked_str(&[("realized", "2.005"), ("cash", "5002.01")]),
            summary("2.01", "800.00", "2.01"),
        ),
    ];
    let calls = cases.iter().map(|(_, b, _)| Call::owner(b)).collect();
    let answers = send_all(&db, calls).await;
    for ((what, _, want), a) in cases.iter().zip(&answers) {
        is_summary(a, want.clone(), what);
    }
    db.finish().await;
}

/// CA7: v2's `Math.round(x × 100)` on the exact decimal — `floor(x × 100 + ½)`,
/// a half toward +∞ — through the kernel's `round_cents` and `fixed2`. The
/// midpoints are chosen where that rule, half-away-from-zero, half-even,
/// truncation and v2's binary floats give different answers.
#[actix_web::test]
async fn figures_round_half_toward_plus_infinity_on_the_exact_decimal() {
    let db = TestDb::start().await;
    // (typed, cents as fixed2) — comments: half-away / half-even / float v2
    let table: [(&str, &str); 26] = [
        ("1.005", "1.01"),   // away 1.01, even 1.00, float 1.00
        ("1.015", "1.02"),   // away 1.02, even 1.02, float 1.01
        ("1.025", "1.03"),   // away 1.03, even 1.02, float 1.02
        ("2.675", "2.68"),   // float 2.67
        ("0.005", "0.01"),   // even 0.00
        ("1.0049", "1.00"),  // half-up on the third place only would give 1.01
        ("1.0051", "1.01"),  // truncation 1.00
        ("1.009", "1.01"),   // truncation 1.00
        ("-1.005", "-1.00"), // away -1.01, even -1.00
        ("-1.015", "-1.01"), // away -1.02, even -1.02
        ("-1.025", "-1.02"), // away -1.03, even -1.02
        ("-0.015", "-0.01"), // away -0.02, even -0.02
        ("-0.025", "-0.02"), // away -0.03
        ("-1.0049", "-1.00"),
        ("-1.0051", "-1.01"), // truncation -1.00
        ("-1.009", "-1.01"),  // truncation -1.00
        (".5", "0.50"),
        ("-.5", "-0.50"),
        ("5", "5.00"),
        ("007", "7.00"),
        ("0.10", "0.10"),
        ("1234567.891", "1234567.89"),
        ("1234567.895", "1234567.90"),
        ("-1234567.895", "-1234567.89"),
        (
            "100000000000000000000000.004",
            "100000000000000000000000.00",
        ),
        (EDGE, EDGE),
    ];
    let mut calls = Vec::new();
    for (typed, _) in table {
        // realized carries the sign; netAssets and cash are wide so that
        // withdrawable is c(realized) when it is not below zero.
        calls.push(Call::owner(&worked_str(&[
            ("realized", typed),
            ("netAssets", EDGE),
            ("capital", "0"),
            ("reserve", "0"),
            ("cash", EDGE),
            ("cashMinimum", "0"),
        ])));
    }
    for (typed, _) in table.iter().filter(|(t, _)| !t.starts_with('-')) {
        // the same rule for a figure of the six: foreignValue against 0
        calls.push(Call::owner(&worked_str(&[
            ("foreignValue", typed),
            ("foreignCost", "0"),
        ])));
    }
    let answers = send_all(&db, calls).await;
    let mut answers = answers.iter();
    for (typed, cents) in table {
        let a = answers.next().expect("an answer per realized row");
        let kernel =
            fixed2(round_cents(typed_decimal(typed).expect("typed")).expect("the cents fit"));
        assert_eq!(
            kernel, cents,
            "the table agrees with the kernel's round_cents/fixed2 for {typed}"
        );
        let withdrawable = if cents.starts_with('-') {
            "0.00"
        } else {
            cents
        };
        is_summary(
            a,
            summary(cents, "800.00", withdrawable),
            &format!("realized {typed}"),
        );
    }
    for (typed, cents) in table.iter().filter(|(t, _)| !t.starts_with('-')) {
        let a = answers.next().expect("an answer per foreignValue row");
        is_summary(
            a,
            summary("1000.00", cents, "1000.00"),
            &format!("foreignValue {typed}, foreignCost 0"),
        );
    }
    db.finish().await;
}

/// §12.20.7 *No negative zero*: v3 never answers `"-0.00"`; and `"-0"` in one
/// of the six is not below zero (v2's `Number("-0") < 0` is false).
#[actix_web::test]
async fn minus_zero_is_neither_answered_nor_refused() {
    let db = TestDb::start().await;
    let mut cases: Vec<(String, Value, Value)> = Vec::new();
    for typed in ["-0.004", "-0.005", "-0", "-0.00", "-.0", "-0.0049999"] {
        cases.push((
            format!("realized {typed}"),
            worked_str(&[("realized", typed)]),
            summary("0.00", "800.00", "0.00"),
        ));
    }
    cases.push((
        "unrealized 1.004 − 1.001".to_owned(),
        worked_str(&[("foreignValue", "1.004"), ("foreignCost", "1.001")]),
        summary("1000.00", "0.00", "1000.00"),
    ));
    // "-0" and "-0.00" in each of the six: allowed, and read as zero
    for typed in ["-0", "-0.00"] {
        for (key, want) in [
            ("foreignValue", summary("1000.00", "-4200.00", "1000.00")),
            ("foreignCost", summary("1000.00", "5000.00", "1000.00")),
            ("capital", summary("1000.00", "800.00", "1000.00")),
            ("reserve", summary("1000.00", "800.00", "1000.00")),
            ("cash", summary("1000.00", "800.00", "0.00")),
            ("cashMinimum", summary("1000.00", "800.00", "1000.00")),
        ] {
            cases.push((format!("{key} {typed}"), worked_str(&[(key, typed)]), want));
        }
    }
    let calls = cases.iter().map(|(_, b, _)| Call::owner(b)).collect();
    let answers = send_all(&db, calls).await;
    for ((what, _, want), a) in cases.iter().zip(&answers) {
        is_summary(a, want.clone(), what);
        assert!(
            !a.text.contains("-0.00"),
            "{what}: no minus zero in {}",
            a.text
        );
    }
    db.finish().await;
}

/// §12.20.7 *Huge figures*: the largest figures whose cents fit compute,
/// including terms far past the 96-bit decimal once combined.
#[actix_web::test]
async fn figures_at_the_96_bit_edge_compute_without_overflow() {
    let db = TestDb::start().await;
    let minus_edge = format!("-{EDGE}");
    let cases: Vec<(&str, Value, Value)> = vec![
        (
            "realized at the edge",
            worked_str(&[("realized", EDGE)]),
            summary(EDGE, "800.00", "3000.00"),
        ),
        (
            "realized at minus the edge",
            worked_str(&[("realized", &minus_edge)]),
            summary(&minus_edge, "800.00", "0.00"),
        ),
        (
            "foreignValue at the edge, foreignCost 0",
            worked_str(&[("foreignValue", EDGE), ("foreignCost", "0")]),
            summary("1000.00", EDGE, "1000.00"),
        ),
        (
            "foreignValue 0, foreignCost at the edge",
            worked_str(&[("foreignValue", "0"), ("foreignCost", EDGE)]),
            summary("1000.00", &minus_edge, "1000.00"),
        ),
        (
            "netAssets at minus the edge, capital and reserve at the edge",
            worked_str(&[
                ("netAssets", &minus_edge),
                ("capital", EDGE),
                ("reserve", EDGE),
            ]),
            summary("1000.00", "800.00", "0.00"),
        ),
        (
            "cash 0, cashMinimum at the edge",
            worked_str(&[("cash", "0"), ("cashMinimum", EDGE)]),
            summary("1000.00", "800.00", "0.00"),
        ),
        (
            "every term at the edge",
            worked_str(&[
                ("realized", EDGE),
                ("netAssets", EDGE),
                ("capital", "0"),
                ("reserve", "0"),
                ("cash", EDGE),
                ("cashMinimum", "0"),
            ]),
            summary(EDGE, "800.00", EDGE),
        ),
    ];
    let calls = cases.iter().map(|(_, b, _)| Call::owner(b)).collect();
    let answers = send_all(&db, calls).await;
    for ((what, _, want), a) in cases.iter().zip(&answers) {
        is_summary(a, want.clone(), what);
    }
    db.finish().await;
}

// ------------------------------------------------------------ profitIncomplete

/// What step 1 refuses as a single key's value, as JSON.
fn not_filled_in() -> Vec<(&'static str, Value)> {
    let mut out = vec![("null", Value::Null)];
    for text in [
        "",
        " ",
        "   ",
        "\t",
        "\n",
        "\u{00a0}",
        "abc",
        "1e3",
        "1E3",
        "+1",
        " 1",
        "1 ",
        "1,000",
        "1_000",
        "Infinity",
        "-Infinity",
        "NaN",
        "-",
        ".",
        "-.",
        "5.",
        "1.2.3",
        "--1",
        "0x10",
        "๑๐",
        // its cents are one past the 96-bit decimal
        "792281625142643375935439504",
        "-792281625142643375935439504",
        "1000000000000000000000000000",
        // not a 96-bit decimal at all
        "792281625142643375935439503.36",
        "79228162514264337593543950336",
        "1.00000000000000000000000000001",
    ] {
        out.push((text, json!(text)));
    }
    out
}

/// P5 (1): each key absent, `null`, empty, whitespace-only (decided on #510),
/// not a typed decimal, or with cents that do not fit is `profitIncomplete`
/// with that key as `field`.
#[actix_web::test]
async fn one_key_not_filled_in_is_profit_incomplete_naming_that_key() {
    let db = TestDb::start().await;
    let mut calls = Vec::new();
    let mut cases = Vec::new();
    for key in KEYS {
        calls.push(Call::owner(&worked_without(&[key])));
        cases.push((key, "absent".to_owned()));
        for (label, value) in not_filled_in() {
            calls.push(Call::owner(&worked_with(&[(key, value)])));
            cases.push((key, format!("{label:?}")));
        }
    }
    let answers = send_all(&db, calls).await;
    for ((key, label), a) in cases.iter().zip(&answers) {
        is_incomplete(a, Some(key), &format!("{key} {label}"));
    }
    db.finish().await;
}

/// §4.3: two or more keys failing step 1 are refused without a field.
#[actix_web::test]
async fn two_or_more_keys_not_filled_in_are_profit_incomplete_without_a_field() {
    let db = TestDb::start().await;
    let mut calls = Vec::new();
    let mut cases = Vec::new();
    for (a, b) in pairs(&KEYS) {
        calls.push(Call::owner(&worked_without(&[a, b])));
        cases.push(format!("{a} and {b} absent"));
        calls.push(Call::owner(&worked_str(&[(a, ""), (b, " ")])));
        cases.push(format!("{a} empty, {b} a space"));
        calls.push(Call::owner(&worked_with(&[
            (a, Value::Null),
            (b, json!("1e3")),
        ])));
        cases.push(format!("{a} null, {b} 1e3"));
    }
    calls.push(Call::owner(&json!({})));
    cases.push("an empty object".to_owned());
    calls.push(Call::owner(&worked_without(&KEYS[1..])));
    cases.push("only realized".to_owned());
    let answers = send_all(&db, calls).await;
    for (what, a) in cases.iter().zip(&answers) {
        is_incomplete(a, None, what);
    }
    db.finish().await;
}

/// P5 order: step 1 runs over all eight before step 2 looks at any sign — a key
/// not filled in wins over a negative one wherever either stands in v2's order.
#[actix_web::test]
async fn not_filled_in_is_checked_over_all_eight_before_any_negative() {
    let db = TestDb::start().await;
    let mut calls = Vec::new();
    let mut cases = Vec::new();
    for empty in KEYS {
        for negative in NON_NEGATIVE.iter().filter(|n| **n != empty) {
            calls.push(Call::owner(&worked_str(&[(empty, ""), (negative, "-1")])));
            cases.push((empty, *negative));
        }
    }
    let answers = send_all(&db, calls).await;
    for ((empty, negative), a) in cases.iter().zip(&answers) {
        is_incomplete(
            a,
            Some(empty),
            &format!("{empty} empty, {negative} negative"),
        );
    }
    db.finish().await;
}

// ------------------------------------------------------------ profitNegative

/// P5 (2): one of the six below zero is `profitNegative` naming it — the sign
/// of the typed figure, before rounding (v2's `Number(p[k]) < 0`): `-0.004`
/// rounds to zero cents and is still refused.
#[actix_web::test]
async fn one_of_the_six_below_zero_is_profit_negative_naming_that_key() {
    let db = TestDb::start().await;
    let mut calls = Vec::new();
    let mut cases = Vec::new();
    for key in NON_NEGATIVE {
        for typed in [
            "-1",
            "-0.01",
            "-0.004",
            "-.001",
            "-1000000",
            "-792281625142643375935439503.35",
        ] {
            calls.push(Call::owner(&worked_str(&[(key, typed)])));
            cases.push((key, typed));
        }
    }
    let answers = send_all(&db, calls).await;
    for ((key, typed), a) in cases.iter().zip(&answers) {
        is_negative(a, Some(key), &format!("{key} {typed}"));
    }
    db.finish().await;
}

/// §4.3: two or more of the six below zero are refused without a field.
#[actix_web::test]
async fn two_or_more_of_the_six_below_zero_are_profit_negative_without_a_field() {
    let db = TestDb::start().await;
    let mut calls = Vec::new();
    let mut cases = Vec::new();
    for (a, b) in pairs(&NON_NEGATIVE) {
        calls.push(Call::owner(&worked_str(&[(a, "-1"), (b, "-0.004")])));
        cases.push(format!("{a} and {b} negative"));
    }
    let all: Vec<(&str, &str)> = NON_NEGATIVE.iter().map(|k| (*k, "-5")).collect();
    calls.push(Call::owner(&worked_str(&all)));
    cases.push("all six negative".to_owned());
    let answers = send_all(&db, calls).await;
    for (what, a) in cases.iter().zip(&answers) {
        is_negative(a, None, what);
    }
    db.finish().await;
}

/// `realized` and `netAssets` may be negative (`profit-summary.js:10`): a
/// negative there is not counted as a second failing key either.
#[actix_web::test]
async fn realized_and_net_assets_below_zero_are_not_profit_negative() {
    let db = TestDb::start().await;
    let cases: Vec<(&str, Value)> = vec![
        (
            "realized and netAssets negative, capital negative",
            worked_str(&[("realized", "-1"), ("netAssets", "-1"), ("capital", "-1")]),
        ),
        (
            "realized negative, cashMinimum negative",
            worked_str(&[("realized", "-0.004"), ("cashMinimum", "-2")]),
        ),
    ];
    let calls = cases.iter().map(|(_, b)| Call::owner(b)).collect();
    let answers = send_all(&db, calls).await;
    is_negative(&answers[0], Some("capital"), cases[0].0);
    is_negative(&answers[1], Some("cashMinimum"), cases[1].0);
    db.finish().await;
}

// ------------------------------------------------------------ implementer tests (§9)

/// Implementer test (ticket #510 §9): a figure sent as a JSON number is
/// refused as `400 invalidRequest` with no field.
#[actix_web::test]
async fn a_figure_sent_as_a_json_number_is_refused_as_invalid_request() {
    let db = TestDb::start().await;
    let mut calls = Vec::new();
    let mut cases = Vec::new();
    for key in KEYS {
        for num in [json!(1000), json!(1000.5)] {
            calls.push(Call::owner(&worked_with(&[(key, num.clone())])));
            cases.push((key, num));
        }
    }
    let answers = send_all(&db, calls).await;
    for ((key, num), a) in cases.iter().zip(&answers) {
        is_refused(
            a,
            400,
            "invalidRequest",
            None,
            "คำขอไม่ถูกต้อง",
            &format!("{key} sent as JSON number {num}"),
        );
    }
    db.finish().await;
}

/// Implementer test (ticket #510 §9): `branch_versions` is unchanged by a compute call.
#[actix_web::test]
async fn branch_versions_are_unchanged_by_a_call() {
    let db = TestDb::start().await;
    sqlx::query(
        "INSERT INTO branch_versions (branch_id, version) VALUES ('XC-B01', 3), ('XC-B02', 7)",
    )
    .execute(&db.pool)
    .await
    .expect("insert initial branch versions");

    let rows_before: Vec<(String, i64)> = sqlx::query_as::<_, (String, i64)>(
        "SELECT branch_id, version FROM branch_versions ORDER BY branch_id",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read branch_versions before");

    let answers = send_all(&db, vec![Call::owner(&Value::Object(worked()))]).await;
    is_summary(
        &answers[0],
        summary("1000.00", "800.00", "1000.00"),
        "successful profit_summary call",
    );

    let rows_after: Vec<(String, i64)> = sqlx::query_as::<_, (String, i64)>(
        "SELECT branch_id, version FROM branch_versions ORDER BY branch_id",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read branch_versions after");

    assert_eq!(
        rows_before, rows_after,
        "branch_versions rows and versions must be unchanged"
    );

    db.finish().await;
}
