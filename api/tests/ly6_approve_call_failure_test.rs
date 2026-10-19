//! LY-6 (#1411) follow-up from the review of PR 2050 (issuecomment-6203417441, follow-up 4): a Loyalty failure on
//! the **approve call itself** answers 503 `loyaltyUnavailable` and keeps the approval `PENDING`.
//!
//! The provided tests (`ly6_loyalty_rule_versions_test.rs`) make Loyalty fully down, so the `version_hash` read fails
//! before `on_decided` is reached, and the lost-answer test accepts any non-2xx. Mutant M2 (the approve call's
//! `SubjectError::DependencyUnavailable` reaching the core as `Internal`, answered 500) therefore survived. Here
//! every read works and only `POST …/{id}/approve` fails (5xx, or no answer within `XC_LOYALTY_TIMEOUT_MS`).
//!
//! Contract (design `membership-points.md` §6.4 R1 and R4): Loyalty unreachable, timing out or answering 5xx while an
//! approval is decided ⇒ 503 `loyaltyUnavailable`, the AP-1 request stays `PENDING`, no decision row is written,
//! nothing is published, and a retry once Loyalty recovers publishes exactly once.
//!
//! Black-box like the provided file: the real app through `bootstrap::configure`, decide through
//! `POST /api/approvals/{id}/decide`, Loyalty a local fake on `127.0.0.1:0`. Needs `TEST_DATABASE_URL`; nothing skips.

const SCHEMA_PREFIX: &str = "ly6_approve_fail";

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
use xchange::presentation::pages::{Callers, SessionReader};

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
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let user = user_of(spec);
        let staff_user = user.clone();
        Box::pin(async move { Callers { user, staff_user } })
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

/// Seeds a DRAFT in Loyalty and submits it through v3; returns `(version_id, request_id)`.
async fn submitted(room: &Room) -> (String, String) {
    let version_id = room.loyalty.seed_point("DRAFT");
    submit(room, "EXECUTIVE", &version_id)
        .await
        .ok("loyaltyRuleSubmit by EXECUTIVE");
    let (request_id, status) = room.one_request_for(&version_id).await;
    assert_eq!(status, "PENDING");
    (version_id, request_id)
}

/// Runs one decide whose approve call fails as `mode` says, checks the 503 and that nothing moved, then retries
/// with Loyalty up and checks it publishes once.
async fn approve_call_fails_then_recovers(mode: Mode, what: &str) {
    let room = Room::start().await;
    let (version_id, request_id) = submitted(&room).await;
    room.loyalty.set_mode(mode);

    let failed = decide(&room, "EXECUTIVE_2", &request_id, "APPROVED").await;
    assert_eq!(
        room.loyalty.calls(&version_id, "approve"),
        1,
        "{what}: the failure is on the approve call itself, so it was reached: {:?}",
        room.loyalty.seen()
    );
    failed.unavailable(what);
    let (_, status) = room.one_request_for(&version_id).await;
    assert_eq!(status, "PENDING", "{what}: the request stays PENDING");
    assert_eq!(
        room.decisions_for(&request_id).await,
        0,
        "{what}: no decision row is kept"
    );
    assert_eq!(room.loyalty.status_of(&version_id), "PENDING_APPROVAL");
    assert_eq!(
        room.loyalty.version(&version_id).approvals_applied,
        0,
        "{what}: nothing is published"
    );

    // Loyalty answered (late) or recovered: the same decision with a new key publishes once.
    room.loyalty.set_mode(Mode::Up);
    decide(&room, "EXECUTIVE_2", &request_id, "APPROVED")
        .await
        .ok(&format!("{what}: retry once Loyalty is back"));
    let (_, status) = room.one_request_for(&version_id).await;
    assert_eq!(status, "APPROVED");
    assert_eq!(room.decisions_for(&request_id).await, 1);
    let v = room.loyalty.version(&version_id);
    assert_eq!(v.dto["status"], "APPROVED");
    assert_eq!(v.approvals_applied, 1, "{what}: published once");
    assert_eq!(v.dto["approvedRef"], json!(request_id));
    room.finish().await;
}

/// Loyalty reads the version fine but answers the approve call 503 (not applied).
/// Red while: the approve call's `Unavailable` reaches `ApprovalService::decide` as `Internal` (answered 500
/// `serverError`), or the decision commits without the publish.
#[actix_web::test]
async fn an_approve_call_answered_5xx_is_503_and_the_approval_stays_pending() {
    approve_call_fails_then_recovers(Mode::FailNext("approve"), "approve answered 503").await;
}

/// Loyalty reads the version fine but does not answer the approve call within `XC_LOYALTY_TIMEOUT_MS`.
/// Red while: a timeout on the approve call is answered as anything but 503 `loyaltyUnavailable` (e.g. 500).
#[actix_web::test]
async fn an_approve_call_that_times_out_is_503_and_the_approval_stays_pending() {
    approve_call_fails_then_recovers(Mode::HangNext("approve"), "approve timed out").await;
}

/// The same for a rejection: Loyalty answers the reject call 503 ⇒ 503, request `PENDING`, Loyalty unchanged; the
/// retry rejects once.
/// Red while: the reject call's `Unavailable` is answered 500.
#[actix_web::test]
async fn a_reject_call_answered_5xx_is_503_and_the_approval_stays_pending() {
    let room = Room::start().await;
    let (version_id, request_id) = submitted(&room).await;
    room.loyalty.set_mode(Mode::FailNext("reject"));

    let failed = decide(&room, "EXECUTIVE_2", &request_id, "REJECTED").await;
    assert_eq!(room.loyalty.calls(&version_id, "reject"), 1);
    failed.unavailable("reject answered 503");
    let (_, status) = room.one_request_for(&version_id).await;
    assert_eq!(status, "PENDING");
    assert_eq!(room.decisions_for(&request_id).await, 0);
    assert_eq!(room.loyalty.status_of(&version_id), "PENDING_APPROVAL");

    decide(&room, "EXECUTIVE_2", &request_id, "REJECTED")
        .await
        .ok("retry the rejection once Loyalty is back");
    let (_, status) = room.one_request_for(&version_id).await;
    assert_eq!(status, "REJECTED");
    assert_eq!(room.loyalty.status_of(&version_id), "REJECTED");
    assert_eq!(room.loyalty.version(&version_id).approvals_applied, 0);
    room.finish().await;
}
