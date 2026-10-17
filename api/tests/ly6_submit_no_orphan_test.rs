//! LY-6 (#1411) follow-up from the review of PR 2050 (issuecomment-6203417441, follow-up 3): `loyaltyRuleSubmit`
//! must never leave an orphan in Loyalty, meaning a version `PENDING_APPROVAL` there with no AP-1 request here.
//!
//! The bug at PR 2050's head `af9ab824`: `loyalty_rule_submit.rs:139-142` calls Loyalty `/{id}/submit` before
//! `:150-181` opens the AP-1 request. When opening fails (the `version_hash` read of Loyalty fails, or
//! `alreadyPending`), the transaction rolls back but Loyalty keeps the version `PENDING_APPROVAL`; a re-submit then
//! meets Loyalty's 422 and answers 409 `staleVersion`, for good.
//!
//! BE controller's ruling (2026-10-16, options (a) + (b)):
//! - (a) the request opens first, inside the command's transaction; Loyalty `/submit` runs after it, and a failure
//!   rolls the transaction back (no request, Loyalty unchanged);
//! - (b) a re-submit is idempotent and resumable: a version Loyalty holds as `PENDING_APPROVAL` with no open AP-1
//!   request is resumed by opening exactly ONE request (this heals the rare "Loyalty accepted, our side failed"
//!   case and any orphan already left).
//!
//! Black-box like the provided `ly6_loyalty_rule_versions_test.rs`: the real app through `bootstrap::configure`,
//! submit through `POST /api/v3/commands/loyalty_rule_submit`, decide through `POST /api/approvals/{id}/decide`,
//! Loyalty a local fake on `127.0.0.1:0`. Needs `TEST_DATABASE_URL`; nothing skips.

const SCHEMA_PREFIX: &str = "ly6_no_orphan";

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, HttpResponse, HttpServer, test as atest, web};
use chrono::Utc;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Row};
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const TENANT: &str = "3f7c2a10-6b4e-4d8a-9c15-2e7a8b9d0c41";
const SECRET: &str = "whsec-LY6-1411-c0ffee";
const SUBJECT: &str = "LOYALTY_RULE_VERSION";
const SUBMIT: &str = "/api/v3/commands/loyalty_rule_submit";
/// A dummy service credential (ruling R3). The fake records nothing about it and checks nothing.
const SERVICE_TOKEN: &str = "ly6-1411-dummy-service-token";
const UNAVAILABLE: &str = "loyaltyUnavailable";
/// v3's Loyalty timeout in these tests (`XC_LOYALTY_TIMEOUT_MS`).
const TIMEOUT_MS: u64 = 400;
/// How long a hanging fake answer waits: well past `TIMEOUT_MS`.
const HANG: Duration = Duration::from_millis(1_500);

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn key() -> String {
    unique("ly6-key")
}

// ---------------------------------------------------------------- the fake Loyalty (LP-1 / LP-3 routes)
//
// The same routes and state machine as the fake in `ly6_loyalty_rule_versions_test.rs` (Loyalty at
// 108-platform-services `989d66d`: `GET /api/v1/loyalty/{point,tier}-rule-versions` lists,
// `POST …/{id}/{submit,approve,reject}` moves `DRAFT → PENDING_APPROVAL → APPROVED | REJECTED`, any other move is
// `422 INVALID_TRANSITION`), plus the failure modes this file needs. Versions are seeded directly.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum Mode {
    Up,
    /// Every request answers 503 and changes nothing.
    Down,
    /// GETs answer normally this many more times, then 503; POSTs keep working.
    GetsLeft(u32),
    /// The next `action` POST answers 503 without being applied; afterwards `Up`.
    FailNext(&'static str),
    /// The next `action` POST is applied, then answered 503 (the answer is lost); afterwards `Up`.
    ApplyNextThenFail(&'static str),
    /// The next `action` POST is not applied and answers only after `HANG` (v3 times out); afterwards `Up`.
    HangNext(&'static str),
}

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
}

#[derive(Debug, Clone)]
struct Version {
    kind: &'static str,
    id: Uuid,
    dto: Value,
    approvals_applied: u32,
}

#[derive(Debug)]
struct Inner {
    mode: Mode,
    versions: Vec<Version>,
    seen: Vec<Seen>,
}

#[derive(Clone)]
struct FakeLoyalty {
    url: String,
    inner: Arc<Mutex<Inner>>,
}

fn err(status: u16, code: &str, message: &str) -> HttpResponse {
    HttpResponse::build(StatusCode::from_u16(status).expect("status"))
        .json(json!({ "error": { "code": code, "message": message } }))
}

fn can_move(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("DRAFT", "PENDING_APPROVAL")
            | ("PENDING_APPROVAL", "APPROVED")
            | ("PENDING_APPROVAL", "REJECTED")
    )
}

enum Plan {
    Answer(HttpResponse),
    HangThen503,
}

fn fake_plan(inner: &mut Inner, method: &str, path: &str, body: &Value) -> Plan {
    inner.seen.push(Seen {
        method: method.to_owned(),
        path: path.to_owned(),
    });
    if inner.mode == Mode::Down {
        return Plan::Answer(err(503, "UNAVAILABLE", "loyalty is down"));
    }
    let Some(rest) = path.strip_prefix("/api/v1/loyalty/") else {
        return Plan::Answer(err(404, "NOT_FOUND", "no such route"));
    };
    let parts: Vec<&str> = rest.split('/').collect();
    let kind: &'static str = match parts.first() {
        Some(&"point-rule-versions") => "point-rule-versions",
        Some(&"tier-rule-versions") => "tier-rule-versions",
        _ => return Plan::Answer(err(404, "NOT_FOUND", "no such route")),
    };
    match (method, parts.len()) {
        ("GET", 1) => {
            if let Mode::GetsLeft(n) = inner.mode {
                if n == 0 {
                    return Plan::Answer(err(503, "UNAVAILABLE", "loyalty read is down"));
                }
                inner.mode = Mode::GetsLeft(n - 1);
            }
            let list: Vec<Value> = inner
                .versions
                .iter()
                .filter(|v| v.kind == kind)
                .map(|v| v.dto.clone())
                .collect();
            Plan::Answer(HttpResponse::Ok().json(list))
        }
        ("POST", 3) => {
            let Ok(id) = Uuid::parse_str(parts[1]) else {
                return Plan::Answer(err(404, "NOT_FOUND", "bad id"));
            };
            let action = parts[2];
            let to = match action {
                "submit" => "PENDING_APPROVAL",
                "approve" => "APPROVED",
                "reject" => "REJECTED",
                _ => return Plan::Answer(err(404, "NOT_FOUND", "no such route")),
            };
            match inner.mode {
                Mode::FailNext(a) if a == action => {
                    inner.mode = Mode::Up;
                    return Plan::Answer(err(503, "UNAVAILABLE", "not applied"));
                }
                Mode::HangNext(a) if a == action => {
                    inner.mode = Mode::Up;
                    return Plan::HangThen503;
                }
                _ => {}
            }
            if to == "APPROVED" && body.get("approvedRef").and_then(Value::as_str).is_none() {
                return Plan::Answer(err(400, "VALIDATION", "missing approvedRef"));
            }
            let mode = inner.mode;
            let Some(v) = inner
                .versions
                .iter_mut()
                .find(|v| v.kind == kind && v.id == id)
            else {
                return Plan::Answer(err(404, "NOT_FOUND", "rule version not found"));
            };
            let from = v.dto["status"].as_str().unwrap_or("").to_owned();
            if !can_move(&from, to) {
                return Plan::Answer(err(422, "INVALID_TRANSITION", &format!("{from} -> {to}")));
            }
            v.dto["status"] = json!(to);
            if to == "APPROVED" {
                v.dto["approvedRef"] = body["approvedRef"].clone();
                v.dto["approvedAt"] = json!(Utc::now().to_rfc3339());
                v.approvals_applied += 1;
            }
            let dto = v.dto.clone();
            if matches!(mode, Mode::ApplyNextThenFail(a) if a == action) {
                inner.mode = Mode::Up;
                return Plan::Answer(err(503, "UNAVAILABLE", "applied, answer lost"));
            }
            Plan::Answer(HttpResponse::Ok().json(dto))
        }
        _ => Plan::Answer(err(404, "NOT_FOUND", "no such route")),
    }
}

async fn fake_handle(
    req: HttpRequest,
    bytes: web::Bytes,
    state: web::Data<Arc<Mutex<Inner>>>,
) -> HttpResponse {
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let plan = {
        let mut inner = state.lock().expect("fake state");
        fake_plan(&mut inner, req.method().as_str(), req.path(), &body)
    };
    match plan {
        Plan::Answer(r) => r,
        Plan::HangThen503 => {
            actix_web::rt::time::sleep(HANG).await;
            err(503, "UNAVAILABLE", "too late")
        }
    }
}

impl FakeLoyalty {
    fn start() -> Self {
        let inner = Arc::new(Mutex::new(Inner {
            mode: Mode::Up,
            versions: Vec::new(),
            seen: Vec::new(),
        }));
        let data = web::Data::new(inner.clone());
        let server = HttpServer::new(move || {
            App::new()
                .app_data(data.clone())
                .default_service(web::to(fake_handle))
        })
        .workers(1)
        .bind(("127.0.0.1", 0))
        .expect("bind a free port");
        let addr = server.addrs()[0];
        actix_web::rt::spawn(server.run());
        Self {
            url: format!("http://{addr}"),
            inner,
        }
    }

    fn set_mode(&self, mode: Mode) {
        self.inner.lock().expect("fake state").mode = mode;
    }

    fn seen(&self) -> Vec<Seen> {
        self.inner.lock().expect("fake state").seen.clone()
    }

    /// How many `POST …/{id}/{action}` v3 sent, whatever they were answered.
    fn calls(&self, id: &str, action: &str) -> usize {
        self.seen()
            .iter()
            .filter(|s| s.method == "POST" && s.path.ends_with(&format!("/{id}/{action}")))
            .count()
    }

    fn version(&self, id: &str) -> Version {
        self.inner
            .lock()
            .expect("fake state")
            .versions
            .iter()
            .find(|v| v.id.to_string() == id)
            .cloned()
            .unwrap_or_else(|| panic!("Loyalty holds version {id}"))
    }

    fn status_of(&self, id: &str) -> String {
        self.version(id).dto["status"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    }

    /// Puts a POINT version with `status` directly into Loyalty (not through v3).
    fn seed_point(&self, status: &str) -> String {
        let id = Uuid::new_v4();
        let dto = json!({
            "id": id,
            "tenantId": TENANT,
            "version": 3,
            "basis": "SETTLE_THB",
            "pointsRatio": "0.25",
            "rounding": "FLOOR",
            "minimumBasisAmount": "200.00",
            "expiryMonths": 18,
            "validFrom": "2027-01-01T00:00:00+00:00",
            "validTo": null,
            "status": status,
            "approvedRef": null,
            "approvedAt": null,
            "createdBy": null,
            "createdAt": Utc::now().to_rfc3339(),
        });
        self.inner
            .lock()
            .expect("fake state")
            .versions
            .push(Version {
                kind: "point-rule-versions",
                id,
                dto,
                approvals_applied: 0,
            });
        id.to_string()
    }
}

// ---------------------------------------------------------------- the database room

struct Room {
    pool: PgPool,
    schema: String,
    guard: Arc<AppSessionGuard>,
    loyalty: FakeLoyalty,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect(
            "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip",
        );
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique(SCHEMA_PREFIX);
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");

        common::install_test_pii_keys();

        Self {
            pool,
            schema,
            guard: Arc::new(AppSessionGuard::new(20)),
            loyalty: FakeLoyalty::start(),
        }
    }

    async fn finish(self) {
        let Room { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
    }

    /// Every `LOYALTY_RULE_VERSION` request whose object names `version_id`, oldest first: `(id, status)`.
    async fn requests_for(&self, version_id: &str) -> Vec<(String, String)> {
        sqlx::query(
            "SELECT id::text AS id, status FROM approval_requests \
             WHERE type = $1 AND object_id LIKE '%' || $2 || '%' ORDER BY requested_at",
        )
        .bind(SUBJECT)
        .bind(version_id)
        .fetch_all(&self.pool)
        .await
        .expect("read approval_requests")
        .iter()
        .map(|r| (r.get("id"), r.get("status")))
        .collect()
    }

    /// The one request for `version_id`; fails when there is none or more than one.
    async fn one_request_for(&self, version_id: &str) -> (String, String) {
        let rows = self.requests_for(version_id).await;
        assert_eq!(
            rows.len(),
            1,
            "exactly one {SUBJECT} request for {version_id}: {rows:?}"
        );
        rows[0].clone()
    }

    async fn decisions_for(&self, request_id: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM approval_decisions WHERE request_id::text = $1")
            .bind(request_id)
            .fetch_one(&self.pool)
            .await
            .expect("count approval_decisions")
    }
}

// ---------------------------------------------------------------- callers

fn user_of(spec: &str) -> Option<Actor> {
    let from = chrono::DateTime::<Utc>::UNIX_EPOCH;
    let (user_id, role) = match spec {
        "EXECUTIVE" => ("user-EXECUTIVE", Role::Executive),
        "EXECUTIVE_2" => ("user-EXECUTIVE-2", Role::Executive),
        _ => return None,
    };
    Some(Actor::new(
        user_id,
        ORG,
        vec![ScopedRole::new(role, None, from, None)],
        None,
    ))
}

struct HeaderCallers;

impl SessionReader for HeaderCallers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let user = user_of(spec);
        let staff_user = user.clone();
        Box::pin(async move {
            Callers {
                owner: None,
                staff: None,
                user,
                staff_user,
            }
        })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> String {
        self.body["error"]["code"].as_str().unwrap_or("").to_owned()
    }

    fn ok(&self, what: &str) {
        assert_eq!(
            self.status,
            StatusCode::OK,
            "{what}: expected 200, got {} {}",
            self.status,
            self.text
        );
    }

    fn unavailable(&self, what: &str) {
        assert_eq!(
            self.status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{what}: expected 503, got {} {}",
            self.status,
            self.text
        );
        assert_eq!(self.code(), UNAVAILABLE, "{what}: {}", self.text);
    }
}

async fn post(room: &Room, who: &str, uri: &str, body: Value, key: &str) -> Answer {
    let timeout = TIMEOUT_MS.to_string();
    let env = vec![
        ("XC_LOYALTY_BASE_URL", room.loyalty.url.as_str()),
        ("XC_LOYALTY_TENANT_ID", TENANT),
        ("XC_LOYALTY_WEBHOOK_SECRET", SECRET),
        ("XC_LOYALTY_TIMEOUT_MS", timeout.as_str()),
        ("XC_LOYALTY_SERVICE_TOKEN", SERVICE_TOKEN),
    ];
    let settings = common::settings(&env);
    let state = AppState::new(Arc::new(settings), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let guard = room.guard.clone();
    let app = atest::init_service(
        App::new()
            .app_data(web::Data::from(guard))
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((AS, who))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header(("idempotency-key", key.to_owned()))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn submit(room: &Room, who: &str, version_id: &str) -> Answer {
    post(
        room,
        who,
        SUBMIT,
        json!({ "versionId": version_id }),
        &key(),
    )
    .await
}

async fn decide(room: &Room, who: &str, request_id: &str, outcome: &str) -> Answer {
    post(
        room,
        who,
        &format!("/api/approvals/{request_id}/decide"),
        json!({ "outcome": outcome, "reason": "LY-6 follow-up test" }),
        &key(),
    )
    .await
}

/// An EXECUTIVE_2 approval of `request_id` publishes `version_id` to Loyalty exactly once, with that request as
/// `approvedRef`. Proves a resumed request is a working one (its `version_hash` matches Loyalty's copy).
async fn approves_and_publishes_once(room: &Room, version_id: &str, request_id: &str) {
    decide(room, "EXECUTIVE_2", request_id, "APPROVED")
        .await
        .ok("approvalDecide APPROVED by EXECUTIVE_2");
    let (_, status) = room.one_request_for(version_id).await;
    assert_eq!(status, "APPROVED");
    assert_eq!(room.decisions_for(request_id).await, 1, "one decision row");
    let v = room.loyalty.version(version_id);
    assert_eq!(v.dto["status"], "APPROVED");
    assert_eq!(v.approvals_applied, 1, "published once");
    assert_eq!(v.dto["approvedRef"], json!(request_id));
}

// ================================================================ (i) the request opens before Loyalty is asked

/// Opening the AP-1 request fails at submit (Loyalty answers the version read but then not the `version_hash` read):
/// 503 `loyaltyUnavailable`, and Loyalty never received `/submit`, so the version is still `DRAFT` and there is no
/// request. Once Loyalty recovers, a re-submit opens exactly one request and sends exactly one `/submit`.
/// Red while: `/submit` is sent before the request is opened (the order at `af9ab824`), which leaves the version
/// `PENDING_APPROVAL` with no request and makes the re-submit 409.
#[actix_web::test]
async fn a_request_that_cannot_be_opened_sends_no_submit_to_loyalty() {
    let room = Room::start().await;
    let version_id = room.loyalty.seed_point("DRAFT");
    // One read (finding the version's kind) works; the next (the request's version_hash) fails.
    room.loyalty.set_mode(Mode::GetsLeft(1));

    let s = submit(&room, "EXECUTIVE", &version_id).await;
    s.unavailable("the version_hash read failed");
    assert_eq!(
        room.loyalty.calls(&version_id, "submit"),
        0,
        "Loyalty is not asked to submit before the request is open: {:?}",
        room.loyalty.seen()
    );
    assert_eq!(room.loyalty.status_of(&version_id), "DRAFT");
    assert!(
        room.requests_for(&version_id).await.is_empty(),
        "no request"
    );

    room.loyalty.set_mode(Mode::Up);
    let again = submit(&room, "EXECUTIVE", &version_id).await;
    again.ok("re-submit once Loyalty is back");
    assert_eq!(again.body["status"], "PENDING_APPROVAL", "{}", again.text);
    let (request_id, status) = room.one_request_for(&version_id).await;
    assert_eq!(status, "PENDING");
    assert_eq!(again.body["requestId"], json!(request_id), "{}", again.text);
    assert_eq!(room.loyalty.status_of(&version_id), "PENDING_APPROVAL");
    assert_eq!(room.loyalty.calls(&version_id, "submit"), 1);
    room.finish().await;
}

// ================================================================ (ii) Loyalty refuses: nothing is kept here

/// Loyalty answers `/submit` 503 without applying it: 503 `loyaltyUnavailable`, no request row is kept (the request
/// opened first is rolled back with the transaction), and the version is still `DRAFT`. A re-submit then works and
/// opens one request.
/// Red while: the request is committed although Loyalty did not take the submit.
#[actix_web::test]
async fn a_failed_loyalty_submit_keeps_no_request_and_leaves_the_version_draft() {
    let room = Room::start().await;
    let version_id = room.loyalty.seed_point("DRAFT");
    room.loyalty.set_mode(Mode::FailNext("submit"));

    let s = submit(&room, "EXECUTIVE", &version_id).await;
    s.unavailable("Loyalty answered /submit 503");
    assert_eq!(room.loyalty.calls(&version_id, "submit"), 1);
    assert!(
        room.requests_for(&version_id).await.is_empty(),
        "no request row is kept"
    );
    assert_eq!(room.loyalty.status_of(&version_id), "DRAFT");

    submit(&room, "EXECUTIVE", &version_id)
        .await
        .ok("re-submit once Loyalty is back");
    let (_, status) = room.one_request_for(&version_id).await;
    assert_eq!(status, "PENDING");
    assert_eq!(room.loyalty.status_of(&version_id), "PENDING_APPROVAL");
    room.finish().await;
}

// ================================================================ (iii) "Loyalty accepted, our side failed" heals

/// Loyalty applies `/submit` but its answer is lost (503), so v3 rolls back: the version is `PENDING_APPROVAL` in
/// Loyalty with no request here, the same state a failed commit after an accepted `/submit` leaves. A re-submit
/// resumes it: 200 `{status: PENDING_APPROVAL, requestId}` and exactly one request; a second re-submit is 409 and
/// still leaves one request; the resumed request approves and publishes once.
/// Red while: a re-submit of a `PENDING_APPROVAL` version with no request is refused (at `af9ab824`: Loyalty's 422
/// ⇒ 409 `staleVersion`), or a re-submit opens a second request.
#[actix_web::test]
async fn a_submit_loyalty_accepted_but_we_did_not_keep_is_resumed_with_one_request() {
    let room = Room::start().await;
    let version_id = room.loyalty.seed_point("DRAFT");
    room.loyalty.set_mode(Mode::ApplyNextThenFail("submit"));

    let lost = submit(&room, "EXECUTIVE", &version_id).await;
    assert!(
        !lost.status.is_success(),
        "a submit whose answer was lost is not reported as success: {} {}",
        lost.status,
        lost.text
    );
    assert_eq!(room.loyalty.status_of(&version_id), "PENDING_APPROVAL");
    assert!(
        room.requests_for(&version_id).await.is_empty(),
        "the orphan: Loyalty took it, v3 kept nothing"
    );

    let resumed = submit(&room, "EXECUTIVE", &version_id).await;
    resumed.ok("re-submit resumes the orphan");
    assert_eq!(
        resumed.body["status"], "PENDING_APPROVAL",
        "{}",
        resumed.text
    );
    let (request_id, status) = room.one_request_for(&version_id).await;
    assert_eq!(status, "PENDING");
    assert_eq!(
        resumed.body["requestId"],
        json!(request_id),
        "{}",
        resumed.text
    );

    let twice = submit(&room, "EXECUTIVE", &version_id).await;
    assert_eq!(twice.status, StatusCode::CONFLICT, "{}", twice.text);
    assert_eq!(
        room.one_request_for(&version_id).await.0,
        request_id,
        "a second re-submit opens no second request"
    );

    approves_and_publishes_once(&room, &version_id, &request_id).await;
    room.finish().await;
}

/// An orphan already in Loyalty (seeded `PENDING_APPROVAL`, no request here; e.g. left by the `af9ab824` order) is
/// healed by a re-submit, and two re-submits at once still open exactly ONE request: one answers 200, the other 409.
/// Red while: the resume path is missing (both refused), or it does not hold the AP-1 one-pending-request rule.
#[actix_web::test]
async fn an_existing_orphan_is_healed_by_exactly_one_request_even_under_two_resubmits() {
    let room = Room::start().await;
    let version_id = room.loyalty.seed_point("PENDING_APPROVAL");

    let (a, b) = tokio::join!(
        submit(&room, "EXECUTIVE", &version_id),
        submit(&room, "EXECUTIVE", &version_id)
    );
    let mut statuses = [a.status, b.status];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::CONFLICT],
        "one re-submit resumes, the other is refused: {} / {}",
        a.text,
        b.text
    );
    let (request_id, status) = room.one_request_for(&version_id).await;
    assert_eq!(status, "PENDING");
    let winner = if a.status == StatusCode::OK { &a } else { &b };
    assert_eq!(
        winner.body["requestId"],
        json!(request_id),
        "{}",
        winner.text
    );
    assert_eq!(room.loyalty.status_of(&version_id), "PENDING_APPROVAL");

    approves_and_publishes_once(&room, &version_id, &request_id).await;
    room.finish().await;
}

// ================================================================ (iv) keep-pins: today's behaviour that stays

/// A normal submit sends exactly one `/submit` to Loyalty and opens exactly one request; the answer carries it.
#[actix_web::test]
async fn a_normal_submit_sends_one_submit_and_opens_one_request() {
    let room = Room::start().await;
    let version_id = room.loyalty.seed_point("DRAFT");

    let s = submit(&room, "EXECUTIVE", &version_id).await;
    s.ok("loyaltyRuleSubmit");
    assert_eq!(s.body["versionId"], json!(version_id), "{}", s.text);
    assert_eq!(s.body["status"], "PENDING_APPROVAL", "{}", s.text);
    let (request_id, status) = room.one_request_for(&version_id).await;
    assert_eq!(status, "PENDING");
    assert_eq!(s.body["requestId"], json!(request_id), "{}", s.text);
    assert_eq!(room.loyalty.calls(&version_id, "submit"), 1);
    assert_eq!(room.loyalty.status_of(&version_id), "PENDING_APPROVAL");
    room.finish().await;
}

/// Submitting a version whose request is still open stays 409 `staleVersion` and opens no second request.
#[actix_web::test]
async fn a_submit_while_a_request_is_open_stays_409() {
    let room = Room::start().await;
    let version_id = room.loyalty.seed_point("DRAFT");
    submit(&room, "EXECUTIVE", &version_id)
        .await
        .ok("first submit");
    let (request_id, _) = room.one_request_for(&version_id).await;

    let again = submit(&room, "EXECUTIVE", &version_id).await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.text);
    assert_eq!(again.code(), "staleVersion", "{}", again.text);
    let (still, status) = room.one_request_for(&version_id).await;
    assert_eq!(still, request_id);
    assert_eq!(status, "PENDING");
    assert_eq!(room.loyalty.status_of(&version_id), "PENDING_APPROVAL");
    room.finish().await;
}

/// Versions Loyalty no longer holds open (`APPROVED`, `REJECTED`) are not submitted again: 409 `staleVersion`, no
/// request, Loyalty unchanged.
#[actix_web::test]
async fn a_decided_version_is_not_submitted_again() {
    let room = Room::start().await;
    for state in ["APPROVED", "REJECTED"] {
        let version_id = room.loyalty.seed_point(state);
        let s = submit(&room, "EXECUTIVE", &version_id).await;
        assert_eq!(s.status, StatusCode::CONFLICT, "{state}: {}", s.text);
        assert_eq!(s.code(), "staleVersion", "{state}: {}", s.text);
        assert!(room.requests_for(&version_id).await.is_empty(), "{state}");
        assert_eq!(room.loyalty.status_of(&version_id), state);
    }
    room.finish().await;
}
