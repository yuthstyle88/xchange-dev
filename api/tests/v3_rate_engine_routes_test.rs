//! Ticket #598 (RE-1+RE-3) — the rate engine's routes (RE-3): who may call each view and command,
//! the `published_rates` answer, and the publication version moving on publish.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Provided-test amendment for #602 (RE-5), by the same author: `ratePolicySave` lets OWNER and
//! RATES through on Rpc (lane controller's ruling 2026-10-09: RATE_MANAGER may save a policy
//! version changing only buy/sell %, per Blueprint RATE_MANAGER 'ปรับ %'). The "save policy"
//! route's row is `Who::OwnerOrRates`; since a RATES save must keep the branch's current frame, XC-B01 is given
//! a frame equal to the route's body before the sweep. RATES's percent-only rule itself is pinned in
//! `api/tests/v3_rate_policy_save_test.rs`.
//!
//! Design: `docs/design/blueprint/rates.md` §4.7 (routes, Blueprint §10 paths), §6 (the command
//! seam, merged as CM-1; the "Policy rows" table: `ratePolicySave` OWNER (OWNER and RATES since
//! #602); `rateDraft` (save,
//! submit, simulate, publish, fetch reference) OWNER and RATES on Rpc, RATES on StaffRpc;
//! `approvalDecide` OWNER; `approvalRead` OWNER), §4.8 (`published_rates` inherits
//! `(Rpc, "adminSnapshot")`, `staff_published_rates` inherits `(StaffRpc, "adminSnapshot")`, the
//! same DTO, the §10 public shape with no reference cost or PII), §3.3 (a rate on the wire is the
//! 8-place string the database holds), §9 RE-3.
//!
//! The tests need a real PostgreSQL: `TEST_DATABASE_URL` must name an existing database whose name
//! ends in `_test`. They fail without it; they do not skip. Each test works in a schema of its own.
//!
//! Choices made here where the design leaves the exact form open (the implementer follows them):
//! - Each route's `INHERITS` (the policy pair):
//!   - `POST /api/rate-drafts`, `PUT /api/rate-drafts/{id}`, `POST /api/rate-drafts/{id}/simulate`,
//!     `…/submit`, `…/publish`, `POST /api/rate-references/fetch`, and the views `rate_drafts`,
//!     `rate_draft`, `rate_publications`, `rate_policy`: `(Rpc, "rateDraft")`;
//!   - `POST /api/rate-policies`: `(Rpc, "ratePolicySave")`;
//!   - `POST /api/approvals/{id}/decide`: `(Rpc, "approvalDecide")`;
//!   - the view `approval_requests`: `(Rpc, "approvalRead")`;
//!   - `published_rates`: `(Rpc, "adminSnapshot")`; `staff_published_rates`: `(StaffRpc,
//!     "adminSnapshot")`.
//! - Command bodies (camelCase; the seam adds the path's `{id}` and an optional `branch`):
//!   - a line: `{"currency": "USD", "buyMode": "AUTO"|"MANUAL", "sellMode": "AUTO"|"MANUAL",
//!     "manualBuy": "<rate>"|null, "manualSell": "<rate>"|null, "reason": "<text>"|null}`;
//!   - create `{"lines": [line…]}`; edit `{"expectedVersion": n, "lines": [line…]}`;
//!     simulate `{}`; submit `{}`; publish `{"expectedVersion": n|null}`;
//!   - decide `{"outcome": "APPROVED"|"REJECTED", "reason": "<text>"}`;
//!   - policy save `{"buyAdjustPct", "sellAdjustPct": "<pct>", "guards": {…§4.3…},
//!     "maxReferenceAgeHours": n|null, "onStale": "STOP"|"LIMIT"|null,
//!     "staleLimitThb": "<baht>"|null, "reason": "<text>"}`;
//!   - fetch `{}`.
//! - Command answers (the `Output` is the whole body, as CM-1 answers): create `{"id", "version",
//!   "status", …}`; submit `{"requestId", "status", …}`; publish the `PublicationRef`
//!   `{"id", "version"}`. A status is the database's spelling (`"DRAFT"`, `"PENDING_APPROVAL"`,
//!   `"APPROVED"`, …).
//! - The `rate_draft` view takes the filter `?id=<draft id>`.
//! - `published_rates` / `staff_published_rates` data, exactly these keys:
//!   `{"publication": {"id", "version", "publishedAt"} | null, "sourceDate": "<date>" | null,
//!   "stale": bool, "lines": [{"currency", "group", "unitSize", "buyRate", "sellRate",
//!   "lineId"}]}`; `buyRate`/`sellRate` are the 8-place strings (`"23.50000000"`) or null when a
//!   side is unavailable; `unitSize` is a decimal string. Before a branch's first publication:
//!   `publication` null and `lines` empty.
//! - `POST /api/rate-references/fetch` reads the BoT on the network, so this file asks it only as
//!   callers the policy refuses (refused before `run`); its allowed callers are pinned at the
//!   policy level in `crates/xc-auth/tests/policy_rate_engine_test.rs`.

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
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::pages::{SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";

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
                 The rate-engine route tests need a real PostgreSQL and do not skip. Point it \
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
        let schema = unique("v3rateengine");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01')")
            .execute(&pool)
            .await
            .expect("insert the branch this file names into branches");
        Self { pool, schema }
    }

    async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
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
    Actor::staff(
        format!("employee-{}", role.as_str()),
        role,
        BranchId::new(branch),
    )
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

/// Every caller kind: nobody, the owner, each staff role of XC-B01, and the owner and a staff
/// member in one browser.
fn every_caller() -> Vec<String> {
    let mut out = vec![String::new(), "owner".to_string()];
    for role in STAFF_ROLES {
        out.push(format!("staff:{role}:{B01}"));
        out.push(format!("owner+staff:{role}:{B01}"));
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

fn has_owner(spec: &str) -> bool {
    sessions_of(spec).owner.is_some()
}

fn has_staff(spec: &str) -> bool {
    sessions_of(spec).staff.is_some()
}

fn is_approver_staff(spec: &str) -> bool {
    use xc_auth::domain::access::v3_roles::blueprint_roles;
    use xc_auth::domain::access::{Permission, RoleGrants};
    sessions_of(spec).staff.is_some_and(|s| {
        blueprint_roles(s.role())
            .iter()
            .any(|&r| RoleGrants::seed(r).contains(&Permission::ApprovalsDecide))
    })
}

fn is_rates_staff(spec: &str) -> bool {
    use xc_auth::domain::access::v3_roles::blueprint_roles;
    use xc_auth::domain::access::{Permission, RoleGrants};
    sessions_of(spec).staff.is_some_and(|s| {
        blueprint_roles(s.role())
            .iter()
            .any(|&r| RoleGrants::seed(r).contains(&Permission::RatesDraft))
    })
}

/// The design's role rows (§6 "Policy rows", §4.7 "Who"), under §11.2 (`RoleGrants::seed`).
#[derive(Clone, Copy, Debug)]
enum Who {
    /// `rateDraft` and `ratePolicySave` on Rpc: OWNER and RATES. The owner's session wins on Rpc.
    OwnerOrRates,
    /// `approvalDecide`, `approvalRead` on Rpc: OWNER and staff roles holding `ApprovalsDecide`
    /// under §11.2 (FINANCE).
    OwnerOrApprover,
    /// `adminSnapshot` on Rpc: any signed-in caller.
    AnySession,
    /// `adminSnapshot` on StaffRpc: any staff session; the owner's session is not one.
    StaffSession,
}

impl Who {
    fn allows(self, spec: &str) -> bool {
        match self {
            Who::OwnerOrRates => has_owner(spec) || is_rates_staff(spec),
            Who::OwnerOrApprover => has_owner(spec) || is_approver_staff(spec),
            Who::AnySession => has_owner(spec) || has_staff(spec),
            Who::StaffSession => has_staff(spec),
        }
    }
}

/// The branch the request runs on with no branch named (`Policy::branch`).
fn branch_of(path: EntryPath, spec: &str) -> String {
    let actor = actor_on(path, spec).expect("a signed-in caller");
    Policy::branch(Some(&actor), None).as_str().to_owned()
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(db: &TestDb, who: &str, method: Method, uri: &str, body: Option<Value>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST));
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", unique("key")))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn command(db: &TestDb, who: &str, method: Method, uri: &str, body: Value) -> Answer {
    call(db, who, method, uri, Some(body)).await
}

async fn view(db: &TestDb, who: &str, name_and_query: &str) -> Answer {
    call(
        db,
        who,
        Method::GET,
        &format!("/api/v3/views/{name_and_query}"),
        None,
    )
    .await
}

fn ok(a: &Answer, what: &str) -> Value {
    assert_eq!(a.status, StatusCode::OK, "{what}: body {}", a.text);
    a.body.clone()
}

fn manual_line(currency: &str, buy: &str, sell: &str) -> Value {
    json!({
        "currency": currency,
        "buyMode": "MANUAL",
        "sellMode": "MANUAL",
        "manualBuy": buy,
        "manualSell": sell,
        "reason": "set by hand for the test",
    })
}

/// Create a draft of `lines` as `who` (on `branch` when named); answers its id.
async fn create_draft(db: &TestDb, who: &str, branch: Option<&str>, lines: Vec<Value>) -> String {
    let mut body = json!({ "lines": lines });
    if let Some(b) = branch {
        body["branch"] = json!(b);
    }
    let a = command(db, who, Method::POST, "/api/rate-drafts", body).await;
    let body = ok(&a, "create a draft");
    assert_eq!(body["status"], json!("DRAFT"), "a new draft: {}", a.text);
    assert_eq!(body["version"], json!(1), "a new draft: {}", a.text);
    body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the draft's id: {}", a.text))
        .to_owned()
}

/// Submit a draft as `who`; answers the whole body.
async fn submit_draft(db: &TestDb, who: &str, branch: Option<&str>, id: &str) -> Value {
    let mut body = json!({});
    if let Some(b) = branch {
        body["branch"] = json!(b);
    }
    let a = command(
        db,
        who,
        Method::POST,
        &format!("/api/rate-drafts/{id}/submit"),
        body,
    )
    .await;
    ok(&a, "submit a draft")
}

/// Publish a draft as `who` at `expected`; answers the `PublicationRef`.
async fn publish_draft(
    db: &TestDb,
    who: &str,
    branch: Option<&str>,
    id: &str,
    expected: Option<i64>,
) -> Value {
    let mut body = json!({ "expectedVersion": expected });
    if let Some(b) = branch {
        body["branch"] = json!(b);
    }
    let a = command(
        db,
        who,
        Method::POST,
        &format!("/api/rate-drafts/{id}/publish"),
        body,
    )
    .await;
    ok(&a, "publish a draft")
}

/// The codes a policy refusal answers with (`V3Error::from(PolicyError)`), which an allowed caller
/// with a well-formed request never receives.
const POLICY_CODES: [&str; 6] = [
    "signIn",
    "ownerOnly",
    "employeeAuthRequired",
    "invalidRequest",
    "roleNotAllowed",
    "roleNotAllowedHere",
];

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

// ------------------------------------------------------------ role rows

/// One route of §4.7: how to ask it, its policy pair, and who the design lets through.
struct Route {
    what: &'static str,
    method: Method,
    /// The URI for a caller whose branch is the argument (so an allowed caller names a real
    /// object of its own branch).
    uri: Box<dyn Fn(&str) -> String>,
    /// The body; `None` for a view.
    body: Option<Value>,
    pair: (EntryPath, &'static str),
    who: Who,
    /// Whether allowed callers are sent (false only for the BoT fetch; see the header).
    send_allowed: bool,
}

#[actix_web::test]
async fn every_route_lets_through_exactly_its_role_row_and_refuses_the_rest_as_the_policy_does() {
    let db = TestDb::start().await;

    // a real draft and a real request on each branch, so an allowed caller is never refused for
    // naming something that does not exist in its own branch
    let line = manual_line("USD", "32.1", "32.45");
    let main_draft = create_draft(&db, "owner", None, vec![line.clone()]).await;
    let b01_draft = create_draft(&db, "owner", Some(B01), vec![line.clone()]).await;
    let main_request = submit_draft(&db, "owner", None, &main_draft).await["requestId"]
        .as_str()
        .expect("a request id")
        .to_owned();
    let b01_request = submit_draft(&db, "owner", Some(B01), &b01_draft).await["requestId"]
        .as_str()
        .expect("a request id")
        .to_owned();
    let draft_of = move |branch: &str| -> String {
        if branch == B01 {
            b01_draft.clone()
        } else {
            main_draft.clone()
        }
    };
    let request_of = move |branch: &str| -> String {
        if branch == B01 {
            b01_request.clone()
        } else {
            main_request.clone()
        }
    };

    // #602: a RATES save must keep the branch's current frame, so XC-B01 (the RATES caller's
    // branch) gets, from OWNER, a frame equal to the "save policy" body below
    let policy_body = json!({
        "buyAdjustPct": "0.0000",
        "sellAdjustPct": "0.0000",
        "guards": {},
        "maxReferenceAgeHours": null,
        "onStale": null,
        "staleLimitThb": null,
        "reason": "test",
    });
    let mut framed = policy_body.clone();
    framed["branch"] = json!(B01);
    let frame = command(&db, "owner", Method::POST, "/api/rate-policies", framed).await;
    ok(&frame, "OWNER frames XC-B01");

    let rpc_draft = (EntryPath::Rpc, "rateDraft");
    let routes: Vec<Route> = vec![
        Route {
            what: "create",
            method: Method::POST,
            uri: Box::new(|_| "/api/rate-drafts".to_owned()),
            body: Some(json!({ "lines": [line.clone()] })),
            pair: rpc_draft,
            who: Who::OwnerOrRates,
            send_allowed: true,
        },
        Route {
            what: "edit",
            method: Method::PUT,
            uri: Box::new({
                let draft_of = draft_of.clone();
                move |b| format!("/api/rate-drafts/{}", draft_of(b))
            }),
            body: Some(json!({ "expectedVersion": 1, "lines": [line.clone()] })),
            pair: rpc_draft,
            who: Who::OwnerOrRates,
            send_allowed: true,
        },
        Route {
            what: "simulate",
            method: Method::POST,
            uri: Box::new({
                let draft_of = draft_of.clone();
                move |b| format!("/api/rate-drafts/{}/simulate", draft_of(b))
            }),
            body: Some(json!({})),
            pair: rpc_draft,
            who: Who::OwnerOrRates,
            send_allowed: true,
        },
        Route {
            what: "submit",
            method: Method::POST,
            uri: Box::new({
                let draft_of = draft_of.clone();
                move |b| format!("/api/rate-drafts/{}/submit", draft_of(b))
            }),
            body: Some(json!({})),
            pair: rpc_draft,
            who: Who::OwnerOrRates,
            send_allowed: true,
        },
        Route {
            what: "publish",
            method: Method::POST,
            uri: Box::new({
                let draft_of = draft_of.clone();
                move |b| format!("/api/rate-drafts/{}/publish", draft_of(b))
            }),
            body: Some(json!({ "expectedVersion": null })),
            pair: rpc_draft,
            who: Who::OwnerOrRates,
            send_allowed: true,
        },
        Route {
            what: "fetch reference",
            method: Method::POST,
            uri: Box::new(|_| "/api/rate-references/fetch".to_owned()),
            body: Some(json!({})),
            pair: rpc_draft,
            who: Who::OwnerOrRates,
            send_allowed: false,
        },
        Route {
            what: "save policy",
            method: Method::POST,
            uri: Box::new(|_| "/api/rate-policies".to_owned()),
            body: Some(policy_body.clone()),
            pair: (EntryPath::Rpc, "ratePolicySave"),
            who: Who::OwnerOrRates,
            send_allowed: true,
        },
        Route {
            what: "decide",
            method: Method::POST,
            uri: Box::new(move |b| format!("/api/approvals/{}/decide", request_of(b))),
            body: Some(json!({ "outcome": "APPROVED", "reason": "test" })),
            pair: (EntryPath::Rpc, "approvalDecide"),
            who: Who::OwnerOrApprover,
            send_allowed: true,
        },
        Route {
            what: "view rate_drafts",
            method: Method::GET,
            uri: Box::new(|_| "/api/v3/views/rate_drafts".to_owned()),
            body: None,
            pair: rpc_draft,
            who: Who::OwnerOrRates,
            send_allowed: true,
        },
        Route {
            what: "view rate_draft",
            method: Method::GET,
            uri: Box::new({
                let draft_of = draft_of.clone();
                move |b| format!("/api/v3/views/rate_draft?id={}", draft_of(b))
            }),
            body: None,
            pair: rpc_draft,
            who: Who::OwnerOrRates,
            send_allowed: true,
        },
        Route {
            what: "view rate_publications",
            method: Method::GET,
            uri: Box::new(|_| "/api/v3/views/rate_publications".to_owned()),
            body: None,
            pair: rpc_draft,
            who: Who::OwnerOrRates,
            send_allowed: true,
        },
        Route {
            what: "view rate_policy",
            method: Method::GET,
            uri: Box::new(|_| "/api/v3/views/rate_policy".to_owned()),
            body: None,
            pair: rpc_draft,
            who: Who::OwnerOrRates,
            send_allowed: true,
        },
        Route {
            what: "view approval_requests",
            method: Method::GET,
            uri: Box::new(|_| "/api/v3/views/approval_requests".to_owned()),
            body: None,
            pair: (EntryPath::Rpc, "approvalRead"),
            who: Who::OwnerOrApprover,
            send_allowed: true,
        },
        Route {
            what: "view published_rates",
            method: Method::GET,
            uri: Box::new(|_| "/api/v3/views/published_rates".to_owned()),
            body: None,
            pair: (EntryPath::Rpc, "adminSnapshot"),
            who: Who::AnySession,
            send_allowed: true,
        },
        Route {
            what: "view staff_published_rates",
            method: Method::GET,
            uri: Box::new(|_| "/api/v3/views/staff_published_rates".to_owned()),
            body: None,
            pair: (EntryPath::StaffRpc, "adminSnapshot"),
            who: Who::StaffSession,
            send_allowed: true,
        },
    ];

    let mut allowed = 0;
    let mut refused = 0;
    for route in &routes {
        let (path, action) = route.pair;
        for who in every_caller() {
            let what = format!("{} {} as {who:?}", route.what, route.method);
            let expected = route.who.allows(&who);
            let actor = actor_on(path, &who);
            let user = actor.as_ref().map(|ac| {
                xc_auth::domain::access::v3_roles::actor_from_v3(
                    ac,
                    xchange::presentation::pages::SEEDED_XC_ORG_ID,
                    chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
                )
            });
            let policy = Policy::authorize(
                user.as_ref(),
                path,
                &Action::new(action),
                None,
                chrono::Utc::now(),
            );
            assert_eq!(
                policy.is_ok(),
                expected,
                "{what}: the policy row of ({path:?}, {action:?}) is the design's"
            );
            if expected {
                if !route.send_allowed {
                    continue;
                }
                let branch = branch_of(path, &who);
                let uri = (route.uri)(&branch);
                let records = db.count("idempotency_records").await;
                let a = call(&db, &who, route.method.clone(), &uri, route.body.clone()).await;
                let code = a.body["error"]["code"].as_str().unwrap_or("");
                assert!(
                    !POLICY_CODES.contains(&code),
                    "{what}: allowed, but answered {} {}",
                    a.status,
                    a.text
                );
                if route.body.is_some() && a.status == StatusCode::OK {
                    assert_eq!(
                        db.count("idempotency_records").await,
                        records + 1,
                        "{what}: a command stores its answer under its key"
                    );
                }
                allowed += 1;
            } else {
                let uri = (route.uri)(MAIN_OR_B01);
                let records = db.count("idempotency_records").await;
                let a = call(&db, &who, route.method.clone(), &uri, route.body.clone()).await;
                let err = policy.expect_err("refused");
                let status = err.status();
                let message = err.to_string();
                let code = V3Error::from(err).code;
                assert_eq!(a.status.as_u16(), status, "{what}: status; body {}", a.text);
                assert_eq!(
                    a.body,
                    json!({ "error": { "code": code, "message": message } }),
                    "{what}: the policy's own refusal"
                );
                assert_eq!(
                    db.count("idempotency_records").await,
                    records,
                    "{what}: a refused caller claims no key"
                );
                refused += 1;
            }
        }
    }
    assert!(allowed >= 60, "allowed calls sent: {allowed}");
    assert!(refused >= 100, "refused calls sent: {refused}");
    db.finish().await;
}

/// A refused caller is refused before any id is looked at; any id will do.
const MAIN_OR_B01: &str = "MAIN";

// ------------------------------------------------------------ published_rates

fn assert_keys(value: &Value, keys: &[&str], what: &str) {
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("{what}: an object, got {value}"));
    let mut have: Vec<&str> = object.keys().map(String::as_str).collect();
    have.sort_unstable();
    let mut want = keys.to_vec();
    want.sort_unstable();
    assert_eq!(
        have, want,
        "{what}: exactly these keys (no reference cost, no PII)"
    );
}

fn line_of<'a>(data: &'a Value, currency: &str) -> &'a Value {
    data["lines"]
        .as_array()
        .unwrap_or_else(|| panic!("lines: {data}"))
        .iter()
        .find(|l| l["currency"] == json!(currency))
        .unwrap_or_else(|| panic!("a {currency} line in {data}"))
}

fn is_eight_places(text: &Value, what: &str) {
    let s = text
        .as_str()
        .unwrap_or_else(|| panic!("{what}: a string, got {text}"));
    let (int, frac) = s
        .split_once('.')
        .unwrap_or_else(|| panic!("{what}: a decimal point in {s:?}"));
    assert!(
        !int.is_empty() && int.bytes().all(|b| b.is_ascii_digit()),
        "{what}: {s:?}"
    );
    assert!(
        frac.len() == 8 && frac.bytes().all(|b| b.is_ascii_digit()),
        "{what}: exactly 8 places, got {s:?}"
    );
}

#[actix_web::test]
async fn published_rates_answers_eight_place_strings_with_unit_size_and_the_version_moves_on_publish()
 {
    let db = TestDb::start().await;

    // before any publication
    let before = ok(&view(&db, "owner", "published_rates").await, "before");
    assert_eq!(before["changed"], json!(true));
    assert_eq!(before["data"]["publication"], Value::Null);
    assert_eq!(before["data"]["lines"], json!([]));
    let v0 = before["version"].as_i64().expect("a numeric version");

    // OWNER: draft → submit (out-of-bound under no policy: OWNER's own decision) → publish
    let first = create_draft(
        &db,
        "owner",
        None,
        vec![
            manual_line("JPY", "23.4", "23.5"),
            manual_line("USD", "32.1", "32.45"),
        ],
    )
    .await;
    let submitted = submit_draft(&db, "owner", None, &first).await;
    assert_eq!(submitted["status"], json!("APPROVED"), "{submitted}");
    let p1 = publish_draft(&db, "owner", None, &first, None).await;
    assert_eq!(p1["version"], json!(1), "{p1}");

    let a = ok(&view(&db, "owner", "published_rates").await, "after v1");
    let v1 = a["version"].as_i64().expect("a numeric version");
    assert!(v1 > v0, "the view's version moves on publish: {v0} → {v1}");
    let data = &a["data"];
    assert_keys(
        data,
        &["publication", "sourceDate", "stale", "lines"],
        "data",
    );
    assert_keys(
        &data["publication"],
        &["id", "version", "publishedAt"],
        "publication",
    );
    assert_eq!(data["publication"]["version"], json!(1));
    assert_eq!(data["publication"]["id"], p1["id"]);
    assert!(data["publication"]["publishedAt"].is_string());
    assert_eq!(
        data["stale"],
        json!(false),
        "MANUAL lines with no reference are never stale"
    );
    assert_eq!(
        data["sourceDate"],
        Value::Null,
        "no reference behind these lines"
    );
    assert_eq!(data["lines"].as_array().map(Vec::len), Some(2));

    let jpy = line_of(data, "JPY");
    assert_keys(
        jpy,
        &[
            "currency", "group", "unitSize", "buyRate", "sellRate", "lineId",
        ],
        "a line",
    );
    is_eight_places(&jpy["buyRate"], "JPY buyRate");
    is_eight_places(&jpy["sellRate"], "JPY sellRate");
    assert_eq!(jpy["buyRate"], json!("23.40000000"));
    assert_eq!(jpy["sellRate"], json!("23.50000000"));
    assert_eq!(jpy["group"], json!("ALL"));
    assert_eq!(
        d(jpy["unitSize"]
            .as_str()
            .expect("unitSize is a decimal string")),
        d("100")
    );
    assert!(jpy["lineId"].as_str().is_some_and(|s| !s.is_empty()));

    let usd = line_of(data, "USD");
    assert_eq!(usd["buyRate"], json!("32.10000000"));
    assert_eq!(usd["sellRate"], json!("32.45000000"));
    assert_eq!(
        d(usd["unitSize"]
            .as_str()
            .expect("unitSize is a decimal string")),
        d("1")
    );

    // unchanged since v1: the poll says so
    let same = ok(
        &view(&db, "owner", &format!("published_rates?since={v1}")).await,
        "since v1",
    );
    assert_eq!(same["changed"], json!(false));

    // a second publication at expectedVersion 1 moves the publication version to 2
    let second = create_draft(
        &db,
        "owner",
        None,
        vec![
            manual_line("JPY", "23.4", "23.5"),
            manual_line("USD", "32.2", "32.55"),
        ],
    )
    .await;
    submit_draft(&db, "owner", None, &second).await;
    let p2 = publish_draft(&db, "owner", None, &second, Some(1)).await;
    assert_eq!(p2["version"], json!(2), "{p2}");
    let a = ok(
        &view(&db, "owner", &format!("published_rates?since={v1}")).await,
        "after v2",
    );
    assert_eq!(
        a["changed"],
        json!(true),
        "the poll sees the new publication"
    );
    assert!(a["version"].as_i64().expect("a numeric version") > v1);
    assert_eq!(a["data"]["publication"]["version"], json!(2));
    assert_eq!(a["data"]["publication"]["id"], p2["id"]);
    assert_eq!(line_of(&a["data"], "USD")["sellRate"], json!("32.55000000"));
    assert_ne!(
        line_of(&a["data"], "USD")["lineId"],
        usd["lineId"],
        "a new publication has new lines"
    );

    // a publish at a stale expectedVersion is refused and moves nothing
    let third = create_draft(
        &db,
        "owner",
        None,
        vec![manual_line("USD", "32.3", "32.65")],
    )
    .await;
    submit_draft(&db, "owner", None, &third).await;
    let stale = command(
        &db,
        "owner",
        Method::POST,
        &format!("/api/rate-drafts/{third}/publish"),
        json!({ "expectedVersion": 1 }),
    )
    .await;
    assert_eq!(
        stale.body["error"]["code"],
        json!("staleVersion"),
        "{}",
        stale.text
    );
    let a = ok(
        &view(&db, "owner", "published_rates").await,
        "after the stale publish",
    );
    assert_eq!(a["data"]["publication"]["version"], json!(2));
    db.finish().await;
}

#[actix_web::test]
async fn the_staff_view_reads_the_staff_sessions_branch_and_the_rpc_view_the_owners() {
    let db = TestDb::start().await;
    let draft = create_draft(
        &db,
        "owner",
        None,
        vec![manual_line("USD", "32.1", "32.45")],
    )
    .await;
    submit_draft(&db, "owner", None, &draft).await;
    publish_draft(&db, "owner", None, &draft, None).await;

    // one browser, the owner and a FRONT employee of XC-B01
    let both = format!("owner+staff:FRONT:{B01}");
    let rpc = ok(&view(&db, &both, "published_rates").await, "Rpc view");
    assert_eq!(
        rpc["data"]["publication"]["version"],
        json!(1),
        "Rpc: the owner's session, MAIN"
    );
    let staff = ok(
        &view(&db, &both, "staff_published_rates").await,
        "StaffRpc view",
    );
    assert_eq!(
        staff["data"]["publication"],
        Value::Null,
        "StaffRpc: the staff session's branch, XC-B01, which has no publication"
    );
    assert_eq!(staff["data"]["lines"], json!([]));

    // the owner alone has no staff session
    let owner_only = view(&db, "owner", "staff_published_rates").await;
    assert_eq!(
        owner_only.body["error"]["code"],
        json!("employeeAuthRequired"),
        "{}",
        owner_only.text
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_rates_draft_out_of_bound_waits_for_the_owner_then_rates_publishes_it() {
    let db = TestDb::start().await;
    let rates = format!("staff:RATES:{B01}");
    // RATES of XC-B01 drafts on its own branch; under no policy it is out-of-bound
    let draft = create_draft(&db, &rates, None, vec![manual_line("USD", "32.1", "32.45")]).await;
    let submitted = submit_draft(&db, &rates, None, &draft).await;
    assert_eq!(
        submitted["status"],
        json!("PENDING_APPROVAL"),
        "{submitted}"
    );
    let request = submitted["requestId"]
        .as_str()
        .expect("a request id")
        .to_owned();

    // not yet publishable
    let early = command(
        &db,
        &rates,
        Method::POST,
        &format!("/api/rate-drafts/{draft}/publish"),
        json!({ "expectedVersion": null }),
    )
    .await;
    assert_eq!(
        early.body["error"]["code"],
        json!("approvalRequired"),
        "{}",
        early.text
    );

    // the owner sees it in the A06 queue and approves it, on XC-B01
    let queue = ok(
        &view(&db, "owner", &format!("approval_requests?branch={B01}")).await,
        "A06",
    );
    assert!(
        queue["data"].to_string().contains(&request),
        "the pending request is in the owner's queue: {queue}"
    );
    let decided = command(
        &db,
        "owner",
        Method::POST,
        &format!("/api/approvals/{request}/decide"),
        json!({ "branch": B01, "outcome": "APPROVED", "reason": "checked" }),
    )
    .await;
    ok(&decided, "the owner decides");

    // RATES publishes; the staff view of XC-B01 shows it
    let p = publish_draft(&db, &rates, None, &draft, None).await;
    assert_eq!(p["version"], json!(1), "{p}");
    let front = ok(
        &view(&db, &format!("staff:FRONT:{B01}"), "staff_published_rates").await,
        "the staff view",
    );
    assert_eq!(front["data"]["publication"]["version"], json!(1));
    assert_eq!(
        line_of(&front["data"], "USD")["sellRate"],
        json!("32.45000000")
    );
    // and MAIN still has none
    let main = ok(&view(&db, "owner", "published_rates").await, "MAIN");
    assert_eq!(main["data"]["publication"], Value::Null);
    db.finish().await;
}
