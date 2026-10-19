//! LY-6 (#1411) up-front tests — EXECUTIVE loyalty rule versions: `loyaltyRuleDraft`, `loyaltyRuleSubmit`, the AP-1
//! subject `LOYALTY_RULE_VERSION` and its publish to Loyalty (design `docs/design/blueprint/membership-points.md`
//! §3, §4.3, §4.6, §9.1 row LY-6, §9.3 row LY-6). Written before the implementation by a test-writer agent of the
//! xchange BE lane controller claude/7ca80c, on its order. Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! **Black-box**, in the style of LY-2's `loyalty_delivery_test.rs`:
//! - the two commands only through their v3 routes `POST /api/v3/commands/loyalty_rule_draft` and
//!   `POST /api/v3/commands/loyalty_rule_submit` (the `/api/v3/commands/<file>` convention of
//!   `branch_alert_settings_save.rs`, `reconciliation_certify.rs`);
//! - the decision only through the existing `approvalDecide` (`POST /api/approvals/{id}/decide`);
//! - Loyalty configured only through `XC_LOYALTY_BASE_URL`, `XC_LOYALTY_TENANT_ID`, `XC_LOYALTY_WEBHOOK_SECRET`,
//!   `XC_LOYALTY_TIMEOUT_MS`, read by `Settings`;
//! - Loyalty is a local fake on `127.0.0.1:0` that serves the LP-1/LP-3 rule-version routes exactly as merged in
//!   108-Plaza/108-platform-services `989d66d` (`crates/loyalty/src/presentation/http/point_rules.rs:19-25`,
//!   `tier_rules.rs:19-25`, bodies `presentation/dto/rule_version_dto.rs`, states
//!   `domain/value_objects/rule_status.rs:24-34`: `DRAFT → PENDING_APPROVAL → APPROVED | REJECTED`, any other move is
//!   `422 INVALID_TRANSITION`), and records every request.
//!
//! Owner's answers (#1411 issuecomment-6180559023): a draft lives **in Loyalty** (no v3 table, no migration); Loyalty
//! unavailable when an approval is decided ⇒ the approval stays pending and can be retried, no outbox.
//!
//! BE controller claude/7ca80c's rulings (2026-10-15, design `membership-points.md` §6.4): Loyalty unreachable, timing
//! out or answering 5xx ⇒ **503 `loyaltyUnavailable`** (R1); the service credential is `XC_LOYALTY_SERVICE_TOKEN`, sent
//! as `Authorization: Bearer` on the rule-version calls, and unset ⇒ 503 `loyaltyUnavailable` before any write (R3).
//! Every caller here sets a dummy token (the fake does not check it) except the unset-token test.
//!
//! How the AP-1 subject reaches Loyalty and the publisher's types are fixed by the ticket and design §6, not here;
//! nothing here names them. The only configuration named is the environment, as above plus the token.
//!
//! Red on the test commit: neither command route exists (404), and the LY-6 files the static scan reads do not
//! exist. Needs `TEST_DATABASE_URL`; nothing skips.

mod common;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, HttpResponse, HttpServer, test as atest, web};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::{Map, Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Row};
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Permission, Role, RoleGrants, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const MAIN: &str = "MAIN";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const TENANT: &str = "3f7c2a10-6b4e-4d8a-9c15-2e7a8b9d0c41";
const SECRET: &str = "whsec-LY6-1411-c0ffee";
const SUBJECT: &str = "LOYALTY_RULE_VERSION";
const DRAFT: &str = "/api/v3/commands/loyalty_rule_draft";
const SUBMIT: &str = "/api/v3/commands/loyalty_rule_submit";
/// A dummy service credential (R3). The fake records it and checks nothing.
const SERVICE_TOKEN: &str = "ly6-1411-dummy-service-token";
const UNAVAILABLE: &str = "loyaltyUnavailable";

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

/// How the fake answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Up,
    /// Every request answers 503 and changes nothing.
    Down,
    /// The next `approve` is applied, then answered 503 (the answer is lost on the way back); afterwards `Up`.
    ApplyApproveThenFail,
}

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    body: Value,
    /// The `Authorization` header as sent, if any.
    authorization: Option<String>,
}

#[derive(Debug, Clone)]
struct Version {
    /// `point-rule-versions` or `tier-rule-versions`.
    kind: &'static str,
    id: Uuid,
    /// The stored DTO (camelCase, as `rule_version_dto.rs` serialises it).
    dto: Value,
    /// How many times an approval was applied to this version (the publish count).
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

const POINT_FIELDS: &[&str] = &[
    "id",
    "version",
    "basis",
    "pointsRatio",
    "rounding",
    "minimumBasisAmount",
    "expiryMonths",
    "validFrom",
    "validTo",
];
const TIER_FIELDS: &[&str] = &[
    "id",
    "version",
    "rollingMonths",
    "tiers",
    "grace",
    "validFrom",
    "validTo",
];

fn err(status: u16, code: &str, message: &str) -> HttpResponse {
    HttpResponse::build(StatusCode::from_u16(status).expect("status"))
        .json(json!({ "error": { "code": code, "message": message } }))
}

fn decimal_of(v: &Value) -> Option<Decimal> {
    match v {
        Value::String(s) => Decimal::from_str(s).ok(),
        Value::Number(n) => Decimal::from_str(&n.to_string()).ok(),
        _ => None,
    }
}

fn time_of(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
}

fn i32_of(v: &Value) -> Option<i32> {
    v.as_i64().and_then(|n| i32::try_from(n).ok())
}

fn optional<T>(
    body: &Map<String, Value>,
    k: &str,
    f: impl Fn(&Value) -> Option<T>,
) -> Result<(), String> {
    match body.get(k) {
        None | Some(Value::Null) => Ok(()),
        Some(v) => f(v).map(|_| ()).ok_or_else(|| format!("invalid {k}")),
    }
}

fn required<T>(
    body: &Map<String, Value>,
    k: &str,
    f: impl Fn(&Value) -> Option<T>,
) -> Result<T, String> {
    body.get(k)
        .and_then(f)
        .ok_or_else(|| format!("missing or invalid {k}"))
}

/// The same refusals Loyalty's serde extractor gives `CreatePointRuleVersionRequest` (`rule_version_dto.rs`).
fn point_dto(body: &Map<String, Value>, id: Uuid) -> Result<Value, String> {
    let version = required(body, "version", i32_of)?;
    let basis = required(body, "basis", |v| {
        v.as_str()
            .filter(|s| ["SETTLE_THB", "THB_AMOUNT", "REALIZED_MARGIN_THB"].contains(s))
            .map(str::to_owned)
    })?;
    let ratio = required(body, "pointsRatio", decimal_of)?;
    let rounding = required(body, "rounding", |v| {
        v.as_str()
            .filter(|s| ["FLOOR", "HALF_UP"].contains(s))
            .map(str::to_owned)
    })?;
    optional(body, "minimumBasisAmount", decimal_of)?;
    optional(body, "expiryMonths", i32_of)?;
    let valid_from = required(body, "validFrom", time_of)?;
    optional(body, "validTo", time_of)?;
    let minimum = body
        .get("minimumBasisAmount")
        .and_then(decimal_of)
        .unwrap_or(Decimal::ZERO);
    Ok(json!({
        "id": id,
        "tenantId": TENANT,
        "version": version,
        "basis": basis,
        "pointsRatio": ratio.to_string(),
        "rounding": rounding,
        "minimumBasisAmount": minimum.to_string(),
        "expiryMonths": body.get("expiryMonths").cloned().unwrap_or(Value::Null),
        "validFrom": valid_from.to_rfc3339(),
        "validTo": body.get("validTo").cloned().unwrap_or(Value::Null),
        "status": "DRAFT",
        "approvedRef": null,
        "approvedAt": null,
        "createdBy": null,
        "createdAt": Utc::now().to_rfc3339(),
    }))
}

/// The same refusals Loyalty's serde extractor gives `CreateTierRuleVersionRequest` (`tier_rules.rs:46-56`).
fn tier_dto(body: &Map<String, Value>, id: Uuid) -> Result<Value, String> {
    optional(body, "version", i32_of)?;
    let rolling = required(body, "rollingMonths", i32_of)?;
    let tiers = body
        .get("tiers")
        .cloned()
        .ok_or_else(|| "missing tiers".to_owned())?;
    let valid_from = required(body, "validFrom", time_of)?;
    optional(body, "validTo", time_of)?;
    Ok(json!({
        "id": id,
        "tenantId": TENANT,
        "version": body.get("version").and_then(i32_of).unwrap_or(0),
        "rollingMonths": rolling,
        "tiers": tiers,
        "grace": body.get("grace").cloned().unwrap_or(Value::Null),
        "validFrom": valid_from.to_rfc3339(),
        "validTo": body.get("validTo").cloned().unwrap_or(Value::Null),
        "status": "DRAFT",
        "approvedRef": null,
        "approvedAt": null,
        "createdBy": null,
        "createdAt": Utc::now().to_rfc3339(),
    }))
}

fn can_move(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("DRAFT", "PENDING_APPROVAL")
            | ("PENDING_APPROVAL", "APPROVED")
            | ("PENDING_APPROVAL", "REJECTED")
    )
}

async fn fake_handle(
    req: HttpRequest,
    bytes: web::Bytes,
    state: web::Data<Arc<Mutex<Inner>>>,
) -> HttpResponse {
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let method = req.method().as_str().to_owned();
    let path = req.path().to_owned();
    let authorization = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut inner = state.lock().expect("fake state");
    inner.seen.push(Seen {
        method: method.clone(),
        path: path.clone(),
        body: body.clone(),
        authorization,
    });
    if inner.mode == Mode::Down {
        return err(503, "UNAVAILABLE", "loyalty is down");
    }
    let Some(rest) = path.strip_prefix("/api/v1/loyalty/") else {
        return err(404, "NOT_FOUND", "no such route");
    };
    let parts: Vec<&str> = rest.split('/').collect();
    let kind: &'static str = match parts.first() {
        Some(&"point-rule-versions") => "point-rule-versions",
        Some(&"tier-rule-versions") => "tier-rule-versions",
        _ => return err(404, "NOT_FOUND", "no such route"),
    };
    match (method.as_str(), parts.len()) {
        ("GET", 1) => {
            let list: Vec<Value> = inner
                .versions
                .iter()
                .filter(|v| v.kind == kind)
                .map(|v| v.dto.clone())
                .collect();
            HttpResponse::Ok().json(list)
        }
        ("POST", 1) => {
            let Some(obj) = body.as_object() else {
                return err(400, "VALIDATION", "body is not an object");
            };
            let id = match obj.get("id") {
                None | Some(Value::Null) => Uuid::new_v4(),
                Some(v) => match v.as_str().and_then(|s| Uuid::parse_str(s).ok()) {
                    Some(id) => id,
                    None => return err(400, "VALIDATION", "invalid id"),
                },
            };
            let dto = if kind == "point-rule-versions" {
                point_dto(obj, id)
            } else {
                tier_dto(obj, id)
            };
            match dto {
                Ok(dto) => {
                    inner.versions.push(Version {
                        kind,
                        id,
                        dto: dto.clone(),
                        approvals_applied: 0,
                    });
                    HttpResponse::Created().json(dto)
                }
                Err(m) => err(400, "VALIDATION", &m),
            }
        }
        ("POST", 3) => {
            let Ok(id) = Uuid::parse_str(parts[1]) else {
                return err(404, "NOT_FOUND", "bad id");
            };
            let to = match parts[2] {
                "submit" => "PENDING_APPROVAL",
                "approve" => "APPROVED",
                "reject" => "REJECTED",
                _ => return err(404, "NOT_FOUND", "no such route"),
            };
            if to == "APPROVED" && body.get("approvedRef").and_then(Value::as_str).is_none() {
                return err(400, "VALIDATION", "missing approvedRef");
            }
            let mode = inner.mode;
            let Some(v) = inner
                .versions
                .iter_mut()
                .find(|v| v.kind == kind && v.id == id)
            else {
                return err(404, "NOT_FOUND", "PointRuleVersion not found");
            };
            let from = v.dto["status"].as_str().unwrap_or("").to_owned();
            if !can_move(&from, to) {
                return err(422, "INVALID_TRANSITION", &format!("{from} -> {to}"));
            }
            v.dto["status"] = json!(to);
            if to == "APPROVED" {
                v.dto["approvedRef"] = body["approvedRef"].clone();
                v.dto["approvedAt"] = json!(Utc::now().to_rfc3339());
                v.approvals_applied += 1;
            }
            let dto = v.dto.clone();
            if to == "APPROVED" && mode == Mode::ApplyApproveThenFail {
                inner.mode = Mode::Up;
                return err(503, "UNAVAILABLE", "applied, answer lost");
            }
            HttpResponse::Ok().json(dto)
        }
        _ => err(404, "NOT_FOUND", "no such route"),
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
                .app_data(web::PayloadConfig::new(1 << 20))
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

    /// The requests that would have changed something (POST), whatever they were answered.
    fn writes(&self) -> Vec<Seen> {
        self.seen()
            .into_iter()
            .filter(|s| s.method == "POST")
            .collect()
    }

    fn approve_calls(&self, id: &str) -> usize {
        self.seen()
            .iter()
            .filter(|s| s.method == "POST" && s.path.ends_with(&format!("/{id}/approve")))
            .count()
    }

    fn versions(&self) -> Vec<Version> {
        self.inner.lock().expect("fake state").versions.clone()
    }

    fn version(&self, id: &str) -> Version {
        self.versions()
            .into_iter()
            .find(|v| v.id.to_string() == id)
            .unwrap_or_else(|| panic!("Loyalty holds version {id}"))
    }

    fn status_of(&self, id: &str) -> String {
        self.version(id).dto["status"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    }

    /// Changes a stored version behind v3's back (another writer of Loyalty).
    fn edit(&self, id: &str, field: &str, value: Value) {
        let mut inner = self.inner.lock().expect("fake state");
        let v = inner
            .versions
            .iter_mut()
            .find(|v| v.id.to_string() == id)
            .expect("the version exists");
        v.dto[field] = value;
    }

    /// Creates a DRAFT directly in Loyalty (not through v3).
    fn seed_point_draft(&self) -> String {
        let id = Uuid::new_v4();
        let body = point_fields();
        let dto = point_dto(body.as_object().expect("object"), id).expect("valid point fields");
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

// ---------------------------------------------------------------- test data (data, never v3 code)

fn point_fields() -> Value {
    json!({
        "version": 3,
        "basis": "SETTLE_THB",
        "pointsRatio": "0.25",
        "rounding": "FLOOR",
        "minimumBasisAmount": "200.00",
        "expiryMonths": 18,
        "validFrom": "2027-01-01T00:00:00Z",
        "validTo": null
    })
}

fn tier_fields() -> Value {
    json!({
        "version": 2,
        "rollingMonths": 12,
        "tiers": [
            { "code": "SILVER", "minVolumeThb": "1234.00" },
            { "code": "TOP", "minVolumeThb": "56789.00" }
        ],
        "grace": { "months": 2 },
        "validFrom": "2027-01-01T00:00:00Z",
        "validTo": null
    })
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
        let schema = unique("ly6_rules");
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

    /// The one AP-1 request of type `LOYALTY_RULE_VERSION` whose object names `version_id`:
    /// `(id, status, requested_by)`. `None` when there is none.
    async fn request_for(&self, version_id: &str) -> Option<(String, String, String)> {
        let rows = sqlx::query(
            "SELECT id::text AS id, status, requested_by FROM approval_requests \
             WHERE type = $1 AND object_id LIKE '%' || $2 || '%' ORDER BY requested_at",
        )
        .bind(SUBJECT)
        .bind(version_id)
        .fetch_all(&self.pool)
        .await
        .expect("read approval_requests");
        assert!(
            rows.len() <= 1,
            "at most one {SUBJECT} request per version, found {}",
            rows.len()
        );
        rows.first()
            .map(|r| (r.get("id"), r.get("status"), r.get("requested_by")))
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
        "ADMIN" => ("user-ADMIN", Role::Admin),
        "FINANCE" => ("user-FINANCE", Role::Finance),
        "MARKETING" => ("user-MARKETING", Role::Marketing),
        "RATE_MANAGER" => ("user-RATE_MANAGER", Role::RateManager),
        "BRANCH_MANAGER" => ("user-BRANCH_MANAGER", Role::BranchManager),
        "TELLER" => ("user-TELLER", Role::Teller),
        _ => return None,
    };
    let branch = if matches!(role, Role::Teller | Role::BranchManager) {
        Some(BranchId::new(MAIN))
    } else {
        None
    };
    Some(Actor::new(
        user_id,
        ORG,
        vec![ScopedRole::new(role, branch, from, None)],
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
}

async fn post(room: &Room, who: &str, uri: &str, body: Value, key: &str) -> Answer {
    post_with_token(room, who, uri, body, key, Some(SERVICE_TOKEN)).await
}

/// `post`, with the service credential `XC_LOYALTY_SERVICE_TOKEN` set to `token`, or unset when `None`.
async fn post_with_token(
    room: &Room,
    who: &str,
    uri: &str,
    body: Value,
    key: &str,
    token: Option<&str>,
) -> Answer {
    let mut env = vec![
        ("XC_LOYALTY_BASE_URL", room.loyalty.url.as_str()),
        ("XC_LOYALTY_TENANT_ID", TENANT),
        ("XC_LOYALTY_WEBHOOK_SECRET", SECRET),
        ("XC_LOYALTY_TIMEOUT_MS", "2000"),
    ];
    if let Some(token) = token {
        env.push(("XC_LOYALTY_SERVICE_TOKEN", token));
    }
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

async fn draft(room: &Room, who: &str, kind: &str, fields: Value) -> Answer {
    post(
        room,
        who,
        DRAFT,
        json!({ "kind": kind, "fields": fields }),
        &key(),
    )
    .await
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

async fn decide(room: &Room, who: &str, request_id: &str, outcome: &str, key: &str) -> Answer {
    post(
        room,
        who,
        &format!("/api/approvals/{request_id}/decide"),
        json!({ "outcome": outcome, "reason": "LY-6 test" }),
        key,
    )
    .await
}

/// EXECUTIVE drafts a POINT version and submits it; returns `(version_id, request_id)`.
async fn drafted_and_submitted(room: &Room) -> (String, String) {
    let d = draft(room, "EXECUTIVE", "POINT", point_fields()).await;
    d.ok("loyaltyRuleDraft by EXECUTIVE");
    let version_id = d.body["versionId"]
        .as_str()
        .expect("loyaltyRuleDraft answers versionId")
        .to_owned();
    let s = submit(room, "EXECUTIVE", &version_id).await;
    s.ok("loyaltyRuleSubmit by EXECUTIVE");
    let (request_id, status, _) = room
        .request_for(&version_id)
        .await
        .expect("submit opens one LOYALTY_RULE_VERSION request");
    assert_eq!(status, "PENDING");
    (version_id, request_id)
}

// ================================================================ §9.3 LY-6: EXECUTIVE drafts and submits

/// Draft: the fields reach Loyalty's own write route as a DRAFT and nothing else is sent; the answer is
/// `{versionId, status: DRAFT}` with Loyalty's id.
/// Red while: the route is missing (404); the draft is kept in v3 instead of Loyalty; a field is renamed,
/// dropped or converted to a float; v3 sends a `status`/`current` of its own.
#[actix_web::test]
async fn executive_drafts_a_point_rule_version_into_loyalty_as_draft() {
    let room = Room::start().await;
    let d = draft(&room, "EXECUTIVE", "POINT", point_fields()).await;
    d.ok("loyaltyRuleDraft POINT by EXECUTIVE");
    assert_eq!(d.body["status"], "DRAFT", "{}", d.text);
    let version_id = d.body["versionId"].as_str().expect("versionId").to_owned();

    let creates: Vec<Seen> = room
        .loyalty
        .writes()
        .into_iter()
        .filter(|s| s.path == "/api/v1/loyalty/point-rule-versions")
        .collect();
    assert_eq!(
        creates.len(),
        1,
        "one create on Loyalty's point route: {creates:?}"
    );
    let sent = creates[0].body.as_object().expect("a JSON object").clone();
    for k in sent.keys() {
        assert!(
            POINT_FIELDS.contains(&k.as_str()),
            "v3 sent `{k}`, which is not a field of Loyalty's CreatePointRuleVersionRequest (no status, no `current`)"
        );
    }
    let given = point_fields();
    for k in ["version", "basis", "rounding", "expiryMonths", "validFrom"] {
        assert_eq!(
            sent.get(k),
            given.get(k),
            "field {k} reaches Loyalty unchanged"
        );
    }
    for k in ["pointsRatio", "minimumBasisAmount"] {
        assert!(
            sent.get(k).is_some_and(Value::is_string),
            "{k} is a decimal string on the wire, never a float: {:?}",
            sent.get(k)
        );
        assert_eq!(
            sent.get(k).and_then(decimal_of),
            given.get(k).and_then(decimal_of),
            "{k}"
        );
    }

    assert_eq!(room.loyalty.versions().len(), 1);
    assert_eq!(room.loyalty.status_of(&version_id), "DRAFT");
    assert!(
        room.request_for(&version_id).await.is_none(),
        "a draft opens no AP-1 request"
    );
    room.finish().await;
}

/// The tier kind goes to Loyalty's tier route; the whole flow ends APPROVED in Loyalty.
/// Red while: TIER is refused or sent to the point route; the tier body is reshaped.
#[actix_web::test]
async fn executive_drafts_submits_and_another_executive_approves_a_tier_rule_version() {
    let room = Room::start().await;
    let d = draft(&room, "EXECUTIVE", "TIER", tier_fields()).await;
    d.ok("loyaltyRuleDraft TIER by EXECUTIVE");
    assert_eq!(d.body["status"], "DRAFT");
    let version_id = d.body["versionId"].as_str().expect("versionId").to_owned();
    let v = room.loyalty.version(&version_id);
    assert_eq!(v.kind, "tier-rule-versions");
    let create = room
        .loyalty
        .writes()
        .into_iter()
        .find(|s| s.path == "/api/v1/loyalty/tier-rule-versions")
        .expect("one create on Loyalty's tier route");
    let given = tier_fields();
    for k in create.body.as_object().expect("object").keys() {
        assert!(
            TIER_FIELDS.contains(&k.as_str()),
            "unexpected field `{k}` sent to Loyalty"
        );
    }
    for k in ["rollingMonths", "tiers", "grace", "validFrom"] {
        assert_eq!(
            create.body.get(k),
            given.get(k),
            "field {k} reaches Loyalty unchanged"
        );
    }

    let s = submit(&room, "EXECUTIVE", &version_id).await;
    s.ok("loyaltyRuleSubmit TIER");
    assert_eq!(s.body["status"], "PENDING_APPROVAL", "{}", s.text);
    assert_eq!(room.loyalty.status_of(&version_id), "PENDING_APPROVAL");
    let (request_id, _, _) = room.request_for(&version_id).await.expect("one request");

    let a = decide(&room, "EXECUTIVE_2", &request_id, "APPROVED", &key()).await;
    a.ok("approvalDecide by a second EXECUTIVE");
    assert_eq!(room.loyalty.status_of(&version_id), "APPROVED");
    assert_eq!(room.loyalty.version(&version_id).approvals_applied, 1);
    room.finish().await;
}

/// Submit moves the Loyalty version to PENDING_APPROVAL and opens exactly one AP-1 request of type
/// `LOYALTY_RULE_VERSION`, proposed by the submitter.
/// Red while: the submit route is missing; Loyalty's `/submit` is not called (a later approve would be
/// `422 INVALID_TRANSITION` from DRAFT); the request is opened under another type or not at all.
#[actix_web::test]
async fn executive_submit_opens_one_loyalty_rule_version_request_and_moves_loyalty_to_pending() {
    let room = Room::start().await;
    let d = draft(&room, "EXECUTIVE", "POINT", point_fields()).await;
    d.ok("loyaltyRuleDraft");
    let version_id = d.body["versionId"].as_str().expect("versionId").to_owned();

    let s = submit(&room, "EXECUTIVE", &version_id).await;
    s.ok("loyaltyRuleSubmit");
    assert_eq!(s.body["status"], "PENDING_APPROVAL", "{}", s.text);
    assert_eq!(room.loyalty.status_of(&version_id), "PENDING_APPROVAL");
    let (_, status, requested_by) = room.request_for(&version_id).await.expect("one request");
    assert_eq!(status, "PENDING");
    assert_eq!(
        requested_by, "user-EXECUTIVE",
        "the submitter is the proposer"
    );
    assert_eq!(
        room.loyalty.version(&version_id).approvals_applied,
        0,
        "submit publishes nothing"
    );
    room.finish().await;
}

// ================================================================ permission: LoyaltyRulesManage, EXECUTIVE only

/// Every role but EXECUTIVE is refused `400 roleNotAllowed` on both commands, and Loyalty sees no write.
/// Red while: the routes are missing (404); a role other than EXECUTIVE holds `LoyaltyRulesManage`; the gate
/// answers 403 or names a role instead of the permission.
#[actix_web::test]
async fn roles_without_loyalty_rules_manage_are_refused_400_role_not_allowed() {
    let room = Room::start().await;
    let seeded = room.loyalty.seed_point_draft();
    let unauth_roles = [
        "ADMIN",
        "FINANCE",
        "MARKETING",
        "RATE_MANAGER",
        "BRANCH_MANAGER",
        "TELLER",
    ]
    .into_iter()
    .filter(|who| {
        let role = Role::parse(who).expect("role");
        !RoleGrants::seed(role).contains(&Permission::LoyaltyRulesManage)
    });
    for who in unauth_roles {
        let d = draft(&room, who, "POINT", point_fields()).await;
        assert_eq!(
            d.status,
            StatusCode::BAD_REQUEST,
            "{who} drafts: {}",
            d.text
        );
        assert_eq!(d.code(), "roleNotAllowed", "{who} drafts: {}", d.text);

        let s = submit(&room, who, &seeded).await;
        assert_eq!(
            s.status,
            StatusCode::BAD_REQUEST,
            "{who} submits: {}",
            s.text
        );
        assert_eq!(s.code(), "roleNotAllowed", "{who} submits: {}", s.text);
    }
    assert!(
        room.loyalty.writes().is_empty(),
        "a refused caller writes nothing to Loyalty"
    );
    assert_eq!(room.loyalty.status_of(&seeded), "DRAFT");
    assert!(room.request_for(&seeded).await.is_none());
    room.finish().await;
}

// ================================================================ validation

/// A body Loyalty's write route would refuse, or an unknown kind, answers `400 validationFailed`; Loyalty stores
/// nothing.
/// Red while: the route is missing; Loyalty's 400 is passed on as another code or a 500; an unknown kind
/// falls through to one of the routes.
#[actix_web::test]
async fn invalid_drafts_answer_validation_failed_and_store_nothing_in_loyalty() {
    let room = Room::start().await;
    let mut no_ratio = point_fields();
    no_ratio
        .as_object_mut()
        .expect("object")
        .remove("pointsRatio");
    let mut bad_ratio = point_fields();
    bad_ratio["pointsRatio"] = json!("not-a-number");
    let mut no_window = tier_fields();
    no_window
        .as_object_mut()
        .expect("object")
        .remove("rollingMonths");
    let cases = [
        ("BONUS", point_fields(), "unknown kind"),
        ("POINT", no_ratio, "point rule without pointsRatio"),
        (
            "POINT",
            bad_ratio,
            "point rule with a ratio that is not a decimal",
        ),
        ("TIER", no_window, "tier rule without rollingMonths"),
        (
            "POINT",
            json!("fields-not-an-object"),
            "fields not an object",
        ),
    ];
    for (kind, fields, what) in cases {
        let a = draft(&room, "EXECUTIVE", kind, fields).await;
        assert_eq!(a.status, StatusCode::BAD_REQUEST, "{what}: {}", a.text);
        assert_eq!(a.code(), "validationFailed", "{what}: {}", a.text);
    }
    assert!(
        room.loyalty.versions().is_empty(),
        "nothing is stored in Loyalty"
    );
    room.finish().await;
}

// ================================================================ the proposer never decides

/// The submitter cannot approve (nor reject) their own version: `403 outOfScope`, Loyalty untouched.
/// Red while: the AP-1 subject is not registered (the request is never opened); the submit records another
/// proposer; the subject publishes before the core's proposer check.
#[actix_web::test]
async fn the_submitter_cannot_decide_their_own_version() {
    let room = Room::start().await;
    let (version_id, request_id) = drafted_and_submitted(&room).await;
    for outcome in ["APPROVED", "REJECTED"] {
        let a = decide(&room, "EXECUTIVE", &request_id, outcome, &key()).await;
        assert_eq!(a.status, StatusCode::FORBIDDEN, "{outcome}: {}", a.text);
        assert_eq!(a.code(), "outOfScope", "{outcome}: {}", a.text);
    }
    assert_eq!(room.loyalty.status_of(&version_id), "PENDING_APPROVAL");
    assert_eq!(room.loyalty.approve_calls(&version_id), 0);
    let (_, status, _) = room.request_for(&version_id).await.expect("request");
    assert_eq!(status, "PENDING");
    assert_eq!(room.decisions_for(&request_id).await, 0);
    room.finish().await;
}

/// Only EXECUTIVE decides this subject (design §3): under RBAC §11.2, only Finance and Executive hold ApprovalsDecide
/// (Finance: Full); Finance is refused 403 outOfScope, and roles lacking ApprovalsDecide (such as BranchManager) are refused 400.
#[actix_web::test]
async fn only_an_executive_decides_a_loyalty_rule_version() {
    let room = Room::start().await;
    let (version_id, request_id) = drafted_and_submitted(&room).await;
    let a = decide(&room, "FINANCE", &request_id, "APPROVED", &key()).await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "FINANCE: {}", a.text);
    assert_eq!(a.code(), "outOfScope", "FINANCE: {}", a.text);

    let bm = decide(&room, "BRANCH_MANAGER", &request_id, "APPROVED", &key()).await;
    assert_eq!(
        bm.status,
        StatusCode::BAD_REQUEST,
        "BRANCH_MANAGER: {}",
        bm.text
    );
    assert_eq!(
        bm.code(),
        "roleNotAllowedHere",
        "BRANCH_MANAGER: {}",
        bm.text
    );

    assert_eq!(room.loyalty.approve_calls(&version_id), 0);
    assert_eq!(room.loyalty.status_of(&version_id), "PENDING_APPROVAL");
    room.finish().await;
}

// ================================================================ approval publishes exactly once

/// A second EXECUTIVE approves: the request is APPROVED and Loyalty's version is APPROVED once, with the AP-1
/// request id as `approvedRef` (the rule version stays traceable to its approval).
/// Red while: approval does not publish; the publish happens at submit; `approvedRef` is not the request.
#[actix_web::test]
async fn approval_by_another_executive_publishes_the_version_to_loyalty_once() {
    let room = Room::start().await;
    let (version_id, request_id) = drafted_and_submitted(&room).await;
    let a = decide(&room, "EXECUTIVE_2", &request_id, "APPROVED", &key()).await;
    a.ok("approvalDecide APPROVED by EXECUTIVE_2");

    let v = room.loyalty.version(&version_id);
    assert_eq!(v.dto["status"], "APPROVED");
    assert_eq!(v.approvals_applied, 1);
    assert_eq!(room.loyalty.approve_calls(&version_id), 1);
    assert_eq!(
        v.dto["approvedRef"],
        json!(request_id),
        "approvedRef names the AP-1 request"
    );
    let (_, status, _) = room.request_for(&version_id).await.expect("request");
    assert_eq!(status, "APPROVED");
    room.finish().await;
}

/// Approval delivered twice ⇒ one publish: a replay with the same Idempotency-Key and a second decision with a
/// new key both leave exactly one approve call at Loyalty; the second decision is `409 staleVersion`.
/// Red while: a replay re-runs `on_decided`; the subject publishes outside the decided transaction.
#[actix_web::test]
async fn approval_delivered_twice_publishes_once() {
    let room = Room::start().await;
    let (version_id, request_id) = drafted_and_submitted(&room).await;
    let k = key();
    decide(&room, "EXECUTIVE_2", &request_id, "APPROVED", &k)
        .await
        .ok("first approval");
    let replay = decide(&room, "EXECUTIVE_2", &request_id, "APPROVED", &k).await;
    replay.ok("replay with the same Idempotency-Key");
    let again = decide(&room, "EXECUTIVE_2", &request_id, "APPROVED", &key()).await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.text);
    assert_eq!(again.code(), "staleVersion", "{}", again.text);

    assert_eq!(
        room.loyalty.approve_calls(&version_id),
        1,
        "{:?}",
        room.loyalty.seen()
    );
    assert_eq!(room.loyalty.version(&version_id).approvals_applied, 1);
    room.finish().await;
}

/// A repeat publish is a no-op: Loyalty applied the approval but the answer was lost (503), so v3 rolls back and
/// the request stays PENDING; the retry meets Loyalty's `422 INVALID_TRANSITION` for an already APPROVED version
/// carrying this request's `approvedRef`, and must treat it as done — 200, request APPROVED, applied once.
/// Red while: the publisher treats the repeat's 422 as a failure (the retry never succeeds); the retry creates
/// a second version or approves twice; the subject's `version_hash` covers Loyalty's `status`/`approvedRef`/
/// `approvedAt` (the retry would then read as stale — the hash covers the rule's content, which is what
/// `a_version_changed_after_submit_is_stale_and_cannot_be_approved` edits).
#[actix_web::test]
async fn a_repeat_publish_after_a_lost_answer_is_a_no_op() {
    let room = Room::start().await;
    let (version_id, request_id) = drafted_and_submitted(&room).await;
    room.loyalty.set_mode(Mode::ApplyApproveThenFail);

    let first = decide(&room, "EXECUTIVE_2", &request_id, "APPROVED", &key()).await;
    assert!(
        !first.status.is_success(),
        "the lost answer is not reported as success: {} {}",
        first.status,
        first.text
    );
    let (_, status, _) = room.request_for(&version_id).await.expect("request");
    assert_eq!(
        status, "PENDING",
        "the decision rolled back with the failed publish"
    );
    assert_eq!(room.decisions_for(&request_id).await, 0);

    let retry = decide(&room, "EXECUTIVE_2", &request_id, "APPROVED", &key()).await;
    retry.ok("retry after the lost answer");
    let (_, status, _) = room.request_for(&version_id).await.expect("request");
    assert_eq!(status, "APPROVED");
    let v = room.loyalty.version(&version_id);
    assert_eq!(v.dto["status"], "APPROVED");
    assert_eq!(v.approvals_applied, 1, "published once");
    assert_eq!(v.dto["approvedRef"], json!(request_id));
    assert_eq!(
        room.loyalty.versions().len(),
        1,
        "no second version is created"
    );
    room.finish().await;
}

// ================================================================ Loyalty unavailable (owner, 2026-10-15)

/// Loyalty down when the approval is decided: the answer is a refusal (not 2xx), nothing is decided, the
/// request stays PENDING and the Loyalty version PENDING_APPROVAL; once Loyalty is back the same decision
/// succeeds and publishes once. No outbox row is written.
///
/// The refusal is **503 `loyaltyUnavailable`** (BE controller's ruling R1, 2026-10-15: none of the blueprint §10
/// codes fits, so the code is new), mapped from a typed error, not a string match.
/// Red while: the decision commits without the publish; an outbox row is used instead; the retry is refused; the
/// refusal is a 500 `serverError` (the subject's failure reaches the core as `Internal`) or another code.
#[actix_web::test]
async fn loyalty_down_keeps_the_approval_pending_and_a_retry_publishes_once() {
    let room = Room::start().await;
    let (version_id, request_id) = drafted_and_submitted(&room).await;
    let outbox_before: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox_events")
        .fetch_one(&room.pool)
        .await
        .expect("count outbox_events");
    room.loyalty.set_mode(Mode::Down);

    let down = decide(&room, "EXECUTIVE_2", &request_id, "APPROVED", &key()).await;
    assert!(
        !down.status.is_success(),
        "a decision Loyalty did not take is not a success: {} {}",
        down.status,
        down.text
    );
    assert_eq!(
        down.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "Loyalty down answers 503: {}",
        down.text
    );
    assert_eq!(down.code(), UNAVAILABLE, "{}", down.text);
    let (_, status, _) = room.request_for(&version_id).await.expect("request");
    assert_eq!(status, "PENDING");
    assert_eq!(
        room.decisions_for(&request_id).await,
        0,
        "no decision row is kept"
    );
    assert_eq!(room.loyalty.status_of(&version_id), "PENDING_APPROVAL");
    let outbox_after: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox_events")
        .fetch_one(&room.pool)
        .await
        .expect("count outbox_events");
    assert_eq!(
        outbox_after, outbox_before,
        "no outbox: nothing new is built"
    );

    room.loyalty.set_mode(Mode::Up);
    let retry = decide(&room, "EXECUTIVE_2", &request_id, "APPROVED", &key()).await;
    retry.ok("retry once Loyalty is back");
    assert_eq!(room.loyalty.status_of(&version_id), "APPROVED");
    assert_eq!(room.loyalty.version(&version_id).approvals_applied, 1);
    room.finish().await;
}

// ================================================================ the service credential (ruling R3)

/// Every rule-version call v3 makes to Loyalty carries the service credential as `Authorization: Bearer <token>`
/// (Loyalty's write routes need `loyalty:write`, its list route `loyalty:read`).
/// Red while: the token is not read from `XC_LOYALTY_SERVICE_TOKEN`; a call goes out without it or in another form.
#[actix_web::test]
async fn rule_version_calls_carry_the_service_token_as_bearer() {
    let room = Room::start().await;
    let (_, request_id) = drafted_and_submitted(&room).await;
    decide(&room, "EXECUTIVE_2", &request_id, "APPROVED", &key())
        .await
        .ok("approve");
    let seen = room.loyalty.seen();
    assert!(
        seen.iter().any(|s| s.path.ends_with("/approve")),
        "the approval reached Loyalty: {seen:?}"
    );
    let bearer = format!("Bearer {SERVICE_TOKEN}");
    for s in &seen {
        assert_eq!(
            s.authorization.as_deref(),
            Some(bearer.as_str()),
            "{} {} carries the service token as Bearer",
            s.method,
            s.path
        );
    }
    room.finish().await;
}

/// `XC_LOYALTY_SERVICE_TOKEN` unset: `loyaltyRuleDraft` and `loyaltyRuleSubmit` answer **503 `loyaltyUnavailable`**
/// before any write, and so does deciding a pending request — it stays PENDING with no decision row.
/// Red while: v3 calls Loyalty without a credential (Loyalty would answer 401 and nothing says why); the missing
/// token is reported as 500, 400 or another code; the decision commits without the publish.
#[actix_web::test]
async fn an_unset_service_token_answers_loyalty_unavailable_and_writes_nothing() {
    let room = Room::start().await;
    let seeded = room.loyalty.seed_point_draft();

    let d = post_with_token(
        &room,
        "EXECUTIVE",
        DRAFT,
        json!({ "kind": "POINT", "fields": point_fields() }),
        &key(),
        None,
    )
    .await;
    assert_eq!(
        d.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "draft: {}",
        d.text
    );
    assert_eq!(d.code(), UNAVAILABLE, "draft: {}", d.text);

    let s = post_with_token(
        &room,
        "EXECUTIVE",
        SUBMIT,
        json!({ "versionId": seeded }),
        &key(),
        None,
    )
    .await;
    assert_eq!(
        s.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "submit: {}",
        s.text
    );
    assert_eq!(s.code(), UNAVAILABLE, "submit: {}", s.text);
    assert!(
        room.loyalty.writes().is_empty(),
        "no write reaches Loyalty without the token: {:?}",
        room.loyalty.writes()
    );
    assert_eq!(room.loyalty.versions().len(), 1, "only the seeded version");
    assert_eq!(room.loyalty.status_of(&seeded), "DRAFT");
    assert!(room.request_for(&seeded).await.is_none(), "no AP-1 request");

    // A request opened while the token was set, decided after it is gone.
    let (version_id, request_id) = drafted_and_submitted(&room).await;
    let a = post_with_token(
        &room,
        "EXECUTIVE_2",
        &format!("/api/approvals/{request_id}/decide"),
        json!({ "outcome": "APPROVED", "reason": "LY-6 test" }),
        &key(),
        None,
    )
    .await;
    assert_eq!(
        a.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "decide: {}",
        a.text
    );
    assert_eq!(a.code(), UNAVAILABLE, "decide: {}", a.text);
    assert_eq!(room.loyalty.approve_calls(&version_id), 0);
    assert_eq!(room.loyalty.status_of(&version_id), "PENDING_APPROVAL");
    let (_, status, _) = room.request_for(&version_id).await.expect("request");
    assert_eq!(status, "PENDING");
    assert_eq!(room.decisions_for(&request_id).await, 0);
    room.finish().await;
}

// ================================================================ stale after submit

/// A version changed after submit is stale: Loyalty's copy is edited behind v3's back (another writer), so the
/// decision answers `409 staleVersion` and nothing is published.
/// Red while: the subject's `version_hash` does not cover the version's content as Loyalty holds it (e.g. it
/// hashes the id only, or a v3-side copy taken at submit).
#[actix_web::test]
async fn a_version_changed_after_submit_is_stale_and_cannot_be_approved() {
    let room = Room::start().await;
    let (version_id, request_id) = drafted_and_submitted(&room).await;
    room.loyalty.edit(&version_id, "pointsRatio", json!("0.75"));

    let a = decide(&room, "EXECUTIVE_2", &request_id, "APPROVED", &key()).await;
    assert_eq!(a.status, StatusCode::CONFLICT, "{}", a.text);
    assert_eq!(a.code(), "staleVersion", "{}", a.text);
    assert_eq!(room.loyalty.approve_calls(&version_id), 0);
    assert_eq!(room.loyalty.version(&version_id).approvals_applied, 0);
    let (_, status, _) = room.request_for(&version_id).await.expect("request");
    assert_eq!(status, "PENDING");
    room.finish().await;
}

// ================================================================ REJECTED → a new DRAFT

/// A rejection moves Loyalty's version to REJECTED and publishes nothing; the rejected version cannot be
/// submitted again; the way on is a new DRAFT (a new version id), which submits normally.
/// Red while: a rejection leaves Loyalty PENDING_APPROVAL; a rejected version is reopened in place.
#[actix_web::test]
async fn a_rejected_version_stays_rejected_and_a_new_draft_follows() {
    let room = Room::start().await;
    let (version_id, request_id) = drafted_and_submitted(&room).await;
    let r = decide(&room, "EXECUTIVE_2", &request_id, "REJECTED", &key()).await;
    r.ok("approvalDecide REJECTED by EXECUTIVE_2");
    assert_eq!(room.loyalty.status_of(&version_id), "REJECTED");
    assert_eq!(room.loyalty.approve_calls(&version_id), 0);
    let (_, status, _) = room.request_for(&version_id).await.expect("request");
    assert_eq!(status, "REJECTED");

    let again = submit(&room, "EXECUTIVE", &version_id).await;
    assert!(
        !again.status.is_success(),
        "a REJECTED version is not submitted again: {} {}",
        again.status,
        again.text
    );
    assert_eq!(room.loyalty.status_of(&version_id), "REJECTED");

    let d = draft(&room, "EXECUTIVE", "POINT", point_fields()).await;
    d.ok("a new draft after the rejection");
    let new_id = d.body["versionId"].as_str().expect("versionId").to_owned();
    assert_ne!(new_id, version_id, "a new DRAFT is a new version");
    assert_eq!(room.loyalty.status_of(&new_id), "DRAFT");
    submit(&room, "EXECUTIVE", &new_id)
        .await
        .ok("the new draft submits");
    assert_eq!(room.loyalty.status_of(&new_id), "PENDING_APPROVAL");
    room.finish().await;
}

// ================================================================ "current" is derived, never written

/// v3 keeps no rule version and no "current" of its own (owner: drafts live in Loyalty, no v3 table): after a
/// full draft → submit → approve, no v3 table is about rule versions, and no request v3 sent to Loyalty carries a
/// `current`/`isCurrent`/`status` field — "current" is derived by Loyalty from the clock.
/// Red while: the routes are missing; v3 mirrors versions into a table or writes a current flag.
#[actix_web::test]
async fn current_is_never_written_by_v3() {
    let room = Room::start().await;
    let (_, request_id) = drafted_and_submitted(&room).await;
    decide(&room, "EXECUTIVE_2", &request_id, "APPROVED", &key())
        .await
        .ok("approve");

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = current_schema() \
           AND (table_name ILIKE '%rule_version%' OR table_name ILIKE '%loyalty_rule%')",
    )
    .fetch_all(&room.pool)
    .await
    .expect("read information_schema");
    assert!(
        tables.is_empty(),
        "v3 has no rule-version table: {tables:?}"
    );

    for s in room.loyalty.writes() {
        if let Some(obj) = s.body.as_object() {
            for k in ["current", "isCurrent", "status"] {
                assert!(
                    !obj.contains_key(k),
                    "{} {} carries `{k}`: {}",
                    s.method,
                    s.path,
                    s.body
                );
            }
        }
    }
    room.finish().await;
}

// ================================================================ no number of §17 in v3 code

fn api_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            files_under(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// No threshold, ratio or demo tier of blueprint §17 (floor(THB/100), Gold ≥1,000, Platinum ≥5,000) is written
/// in any LY-6 v3 file: the four files design §9.1 gives LY-6, plus the AP-1 subject registration that names
/// `LOYALTY_RULE_VERSION`. The four must exist. A timeout comes from `LoyaltyConfig`, never a literal here.
/// Red while: LY-6's files do not exist (the test commit); a default ratio/threshold/tier name is coded.
#[test]
fn no_threshold_or_ratio_is_hard_coded_in_ly6_files() {
    let root = api_root();
    let mut files: Vec<PathBuf> = [
        "src/presentation/http/v3/commands/loyalty_rule_draft.rs",
        "src/presentation/http/v3/commands/loyalty_rule_submit.rs",
        "crates/xc-loyalty/src/application/rule_publisher.rs",
        "src/application/workflows/loyalty/rule_approval.rs",
    ]
    .iter()
    .map(|f| root.join(f))
    .collect();
    for f in &files {
        assert!(f.is_file(), "LY-6 file {} does not exist", f.display());
    }
    let mut subjects = Vec::new();
    files_under(&root.join("src/bootstrap/approval_subjects"), &mut subjects);
    let named: Vec<PathBuf> = subjects
        .into_iter()
        .filter(|p| std::fs::read_to_string(p).is_ok_and(|s| s.contains(SUBJECT)))
        .collect();
    assert_eq!(
        named.len(),
        1,
        "exactly one approval subject file names {SUBJECT}: {named:?}"
    );
    files.extend(named);

    let forbidden_numbers = ["1000", "1_000", "1,000", "5000", "5_000", "5,000", "0.01"];
    let forbidden_words = ["gold", "platinum"];
    for f in &files {
        let text = std::fs::read_to_string(f).expect("read LY-6 file");
        for (n, line) in text.lines().enumerate() {
            let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            for tok in forbidden_numbers {
                let hit = compact.match_indices(tok).any(|(i, _)| {
                    let before = compact[..i].chars().next_back();
                    let after = compact[i + tok.len()..].chars().next();
                    !before.is_some_and(|c| c.is_ascii_digit() || c == '.' || c == '_')
                        && !after.is_some_and(|c| c.is_ascii_digit() || c == '_')
                });
                assert!(
                    !hit,
                    "{}:{} writes `{tok}` (blueprint §17 demo value): {line}",
                    f.display(),
                    n + 1
                );
            }
            let by_100 = compact.match_indices("/100").any(|(i, _)| {
                !compact[i + 4..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit() || c == '_')
            });
            assert!(
                !by_100,
                "{}:{} divides by 100 (floor(THB/100) is §17's demo ratio): {line}",
                f.display(),
                n + 1
            );
            let lower = line.to_ascii_lowercase();
            for w in forbidden_words {
                assert!(
                    !lower.contains(w),
                    "{}:{} names the demo tier `{w}`: {line}",
                    f.display(),
                    n + 1
                );
            }
        }
    }
}
