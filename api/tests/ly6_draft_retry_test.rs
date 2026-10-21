//! #2881 §4 (ticket: see the PR / issue that names branch `test/2881-d-draft-retry`) up-front tests —
//! `loyaltyRuleDraft` retried after Loyalty created the draft but its answer was lost.
//! Written before the implementation by the ticket author claude/ef3e6c91. Do not edit, delete or weaken; add
//! tests in a file of your own.
//!
//! **What Loyalty really does** (108-Plaza/108-platform-services `d19249f8`):
//! - `POST /api/v1/loyalty/{point,tier}-rule-versions` takes an optional client `id`
//!   (`crates/loyalty/src/presentation/http/point_rules.rs:37`, `tier_rules.rs:86`);
//! - both tables are `UNIQUE (tenant_id, version)` with `id uuid PRIMARY KEY`
//!   (`crates/loyalty/migrations/20261012000001_versioned_rules_tiers.sql:11,20,24,31`);
//! - a duplicate id or version is `23505` ⇒ `AppError::Conflict` ⇒ **409**
//!   (`crates/loyalty/src/infrastructure/pg_rule_versions.rs:139-145`, `crates/pos-error/src/http.rs:23`).
//!
//! So a retry can never create a second draft. What breaks is the answer: XC releases the idempotency claim on
//! any error (`api/src/presentation/http/v3/command.rs:551-563` at `645249f8`), the retry calls Loyalty again,
//! Loyalty answers 409, and `HttpLoyaltyRuleWriter` maps every status other than 400/404/422 to `Unavailable`
//! (`http_rule_writer.rs:117-124`) ⇒ 503 `loyaltyUnavailable` on **every** retry, while the draft exists.
//!
//! The fake below serves the create and list routes the same way: it honours a client `id`, and it answers 409
//! for a taken id or a taken `version` of the same kind. Mode `ApplyCreateThenFail` stores the draft, then answers 503
//! once (the answer lost on the way back), then returns to `Up`.
//!
//! Black-box: the command only through `POST /api/v3/commands/loyalty_rule_draft`; Loyalty configured only through
//! the `XC_LOYALTY_*` environment, as in `ly6_loyalty_rule_versions_test.rs`. Needs `TEST_DATABASE_URL`; nothing
//! skips.
//!
//! Red on the test commit (`origin/main` `645249f8`): rows 1, 2 and 3 (503 `loyaltyUnavailable` instead of the
//! draft / `staleVersion`). Row 4 is a control and is green on both.

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
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::{Map, Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
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
const SECRET: &str = "whsec-2881-draft-retry";
const DRAFT: &str = "/api/v3/commands/loyalty_rule_draft";
const SERVICE_TOKEN: &str = "ly6-2881-dummy-service-token";
const UNAVAILABLE: &str = "loyaltyUnavailable";
const STALE: &str = "staleVersion";
const POINT_ROUTE: &str = "point-rule-versions";
const TIER_ROUTE: &str = "tier-rule-versions";

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
    unique("ly6-2881-key")
}

// ---------------------------------------------------------------- the fake Loyalty (create + list)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Up,
    /// The next create is stored, then answered 503 (the answer is lost on the way back); afterwards `Up`.
    ApplyCreateThenFail,
}

#[derive(Debug, Clone)]
struct Version {
    kind: &'static str,
    id: Uuid,
    dto: Value,
}

#[derive(Debug)]
struct Inner {
    mode: Mode,
    versions: Vec<Version>,
    creates: usize,
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
async fn fake_handle(
    req: HttpRequest,
    bytes: web::Bytes,
    state: web::Data<Arc<Mutex<Inner>>>,
) -> HttpResponse {
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let method = req.method().as_str().to_owned();
    let path = req.path().to_owned();
    let mut inner = state.lock().expect("fake state");
    let kind: &'static str = match path.strip_prefix("/api/v1/loyalty/") {
        Some(POINT_ROUTE) => POINT_ROUTE,
        Some(TIER_ROUTE) => TIER_ROUTE,
        _ => return err(404, "NOT_FOUND", "no such route"),
    };
    match method.as_str() {
        "GET" => {
            let list: Vec<Value> = inner
                .versions
                .iter()
                .filter(|v| v.kind == kind)
                .map(|v| v.dto.clone())
                .collect();
            HttpResponse::Ok().json(list)
        }
        "POST" => {
            inner.creates += 1;
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
            let dto = if kind == POINT_ROUTE {
                point_dto(obj, id)
            } else {
                tier_dto(obj, id)
            };
            let dto = match dto {
                Ok(dto) => dto,
                Err(m) => return err(400, "VALIDATION", &m),
            };
            // `id uuid PRIMARY KEY` + `UNIQUE (tenant_id, version)` ⇒ 23505 ⇒ 409 CONFLICT.
            let taken = inner
                .versions
                .iter()
                .any(|v| v.id == id || (v.kind == kind && v.dto["version"] == dto["version"]));
            if taken {
                return err(
                    409,
                    "CONFLICT",
                    &format!("point rule version {} already exists", dto["version"]),
                );
            }
            inner.versions.push(Version {
                kind,
                id,
                dto: dto.clone(),
            });
            if inner.mode == Mode::ApplyCreateThenFail {
                inner.mode = Mode::Up;
                return err(503, "UNAVAILABLE", "applied, answer lost");
            }
            HttpResponse::Created().json(dto)
        }
        _ => err(404, "NOT_FOUND", "no such route"),
    }
}

impl FakeLoyalty {
    fn start() -> Self {
        let inner = Arc::new(Mutex::new(Inner {
            mode: Mode::Up,
            versions: Vec::new(),
            creates: 0,
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

    fn versions(&self, kind: &str) -> Vec<Version> {
        self.inner
            .lock()
            .expect("fake state")
            .versions
            .iter()
            .filter(|v| v.kind == kind)
            .cloned()
            .collect()
    }

    fn creates(&self) -> usize {
        self.inner.lock().expect("fake state").creates
    }

    /// Stores a DRAFT directly in Loyalty (another writer, not this request).
    fn seed_point_draft(&self, fields: &Value) -> Uuid {
        let id = Uuid::new_v4();
        let dto = point_dto(fields.as_object().expect("object"), id).expect("valid point fields");
        self.inner
            .lock()
            .expect("fake state")
            .versions
            .push(Version {
                kind: POINT_ROUTE,
                id,
                dto,
            });
        id
    }
}

// ---------------------------------------------------------------- test data

fn point_fields(version: i64) -> Value {
    json!({
        "version": version,
        "basis": "SETTLE_THB",
        "pointsRatio": "0.25",
        "rounding": "FLOOR",
        "minimumBasisAmount": "200.00",
        "expiryMonths": 18,
        "validFrom": "2027-01-01T00:00:00Z",
        "validTo": null
    })
}

fn tier_fields(version: i64) -> Value {
    json!({
        "version": version,
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
        let schema = unique("ly6_2881");
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
}

// ---------------------------------------------------------------- callers

struct HeaderCallers;

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let executive = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| s == "EXECUTIVE");
        let user = executive.then(|| {
            Actor::new(
                "user-EXECUTIVE",
                ORG,
                vec![ScopedRole::new(
                    Role::Executive,
                    None,
                    chrono::DateTime::<Utc>::UNIX_EPOCH,
                    None,
                )],
                None,
            )
        });
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
}

async fn draft(room: &Room, kind: &str, fields: Value, key: &str) -> Answer {
    let env = vec![
        ("XC_LOYALTY_BASE_URL", room.loyalty.url.as_str()),
        ("XC_LOYALTY_TENANT_ID", TENANT),
        ("XC_LOYALTY_WEBHOOK_SECRET", SECRET),
        ("XC_LOYALTY_TIMEOUT_MS", "2000"),
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
        .uri(DRAFT)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((AS, "EXECUTIVE"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header(("idempotency-key", key.to_owned()))
        .set_payload(json!({ "kind": kind, "fields": fields }).to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// Rows 1 and 2: Loyalty stores the draft but the answer is lost; the retry with the same key and body answers 200
/// with the draft that exists, and Loyalty still holds exactly one version.
async fn lost_answer_then_retry(kind: &str, route: &'static str, fields: Value) {
    let room = Room::start().await;
    let k = key();

    room.loyalty.set_mode(Mode::ApplyCreateThenFail);
    let first = draft(&room, kind, fields.clone(), &k).await;
    assert_eq!(
        first.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "the first answer is lost ⇒ 503: {}",
        first.text
    );
    assert_eq!(first.code(), UNAVAILABLE, "{}", first.text);
    let stored = room.loyalty.versions(route);
    assert_eq!(stored.len(), 1, "Loyalty stored the draft: {stored:?}");
    let stored_id = stored[0].id.to_string();

    let retry = draft(&room, kind, fields, &k).await;
    assert_eq!(
        retry.status,
        StatusCode::OK,
        "a retry with the same key answers the draft Loyalty already holds, not {} {}",
        retry.status,
        retry.text
    );
    assert_eq!(
        retry.body["versionId"].as_str(),
        Some(stored_id.as_str()),
        "the retry names the draft created by the first try: {}",
        retry.text
    );
    assert_eq!(retry.body["status"], "DRAFT", "{}", retry.text);
    assert_eq!(
        room.loyalty.versions(route).len(),
        1,
        "the retry creates no second draft"
    );
    room.finish().await;
}

/// Row 1 (POINT). Red while: the retry answers 503 `loyaltyUnavailable` (409 mapped to `Unavailable`), or v3 sends
/// no stable `id`, so the retry cannot be matched to the stored draft.
#[actix_web::test]
async fn a_point_draft_retried_after_its_answer_was_lost_answers_the_stored_draft() {
    lost_answer_then_retry("POINT", POINT_ROUTE, point_fields(3)).await;
}

/// Row 2 (TIER): the same path through the tier route.
#[actix_web::test]
async fn a_tier_draft_retried_after_its_answer_was_lost_answers_the_stored_draft() {
    lost_answer_then_retry("TIER", TIER_ROUTE, tier_fields(2)).await;
}

/// Row 3: the version number is held by **another** draft (seeded directly in Loyalty) ⇒ 409 `staleVersion`.
/// v3 must not answer the other draft as if it were this one, and must not call it unavailable.
/// Red while: 503 `loyaltyUnavailable`; or 200 with the seeded draft's id.
#[actix_web::test]
async fn a_draft_whose_version_number_is_taken_by_another_draft_answers_stale_version() {
    let room = Room::start().await;
    let seeded = room.loyalty.seed_point_draft(&point_fields(3));

    let d = draft(&room, "POINT", point_fields(3), &key()).await;
    assert_eq!(d.status, StatusCode::CONFLICT, "{}", d.text);
    assert_eq!(d.code(), STALE, "{}", d.text);
    assert_ne!(
        d.body["versionId"].as_str(),
        Some(seeded.to_string().as_str()),
        "the other draft is not answered as this one"
    );
    let stored = room.loyalty.versions(POINT_ROUTE);
    assert_eq!(stored.len(), 1, "nothing new stored: {stored:?}");
    assert_eq!(stored[0].id, seeded);
    room.finish().await;
}

/// Row 4 (control, green before and after): two keys with two version numbers create two drafts with two ids.
/// Guards an id derivation that ignores the key (every draft would collide on one id).
#[actix_web::test]
async fn two_keys_with_two_version_numbers_create_two_drafts() {
    let room = Room::start().await;
    let a = draft(&room, "POINT", point_fields(3), &key()).await;
    let b = draft(&room, "POINT", point_fields(4), &key()).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(b.status, StatusCode::OK, "{}", b.text);
    assert_ne!(a.body["versionId"], b.body["versionId"]);
    assert_eq!(room.loyalty.versions(POINT_ROUTE).len(), 2);
    assert_eq!(room.loyalty.creates(), 2, "one create per key");
    room.finish().await;
}
