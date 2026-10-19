//! Up-front tests for the v3 view `loyalty_rule_versions` (LY-6 read view for #2129) — written before the
//! implementation by claude/studio-claude-2 on the order of the xchange BE lane controller claude/56a74fcd.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `GET /api/v3/views/loyalty_rule_versions` lists the POINT and TIER rule versions that Loyalty holds for
//! xchange's tenant, newest `version` first, for a caller with `LoyaltyRulesManage` (EXECUTIVE). It reads Loyalty's
//! `GET /api/v1/loyalty/point-rule-versions` and `GET /api/v1/loyalty/tier-rule-versions` (108-platform-services
//! `crates/loyalty/src/presentation/http/point_rules.rs:21`, `tier_rules.rs:20` @ `d19249f8`) with the service
//! credential PR 2050 uses, and writes nothing anywhere.
//!
//! **Black-box**, in the style of `ly6_loyalty_rule_versions_test.rs` (PR 2050): the view only through its route;
//! Loyalty configured only through the `XC_LOYALTY_*` environment read by `Settings`; Loyalty is a local fake on
//! `127.0.0.1:0` that serves the two list routes with the DTOs of `rule_version_dto.rs` and records every request.
//! Nothing here names the implementation's types.
//!
//! Needs `TEST_DATABASE_URL` (a PostgreSQL database whose name ends in `_test`); nothing skips.

mod common;

use std::future::Future;
use std::net::TcpListener;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, HttpResponse, HttpServer, test as atest, web};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::http::v3::VIEWS;
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const VIEW: &str = "loyalty_rule_versions";
const ROUTE: &str = "/api/v3/views/loyalty_rule_versions";
const AS: &str = "x-test-as";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const MAIN: &str = "MAIN";
const GONE: &str = "XC-GONE";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const TENANT: &str = "3f7c2a10-6b4e-4d8a-9c15-2e7a8b9d0c41";
const SECRET: &str = "whsec-LY6-view-c0ffee";
const SERVICE_TOKEN: &str = "ly6-view-dummy-service-token";
const UNAVAILABLE: &str = "loyaltyUnavailable";

const POINT_KEYS: [&str; 10] = [
    "versionId",
    "version",
    "status",
    "basis",
    "pointsRatio",
    "rounding",
    "minimumBasisAmount",
    "expiryMonths",
    "validFrom",
    "validTo",
];
const TIER_KEYS: [&str; 8] = [
    "versionId",
    "version",
    "status",
    "rollingMonths",
    "tiers",
    "grace",
    "validFrom",
    "validTo",
];

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

// ---------------------------------------------------------------- the fake Loyalty (two list routes)

/// How the fake answers.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Up,
    /// Every request answers this status with an error body.
    Status(u16),
    /// Only the tier list answers this status; the point list is served.
    TierStatus(u16),
    /// The point list answers a JSON object, not an array.
    PointNotAnArray,
    /// Every request is answered after this long.
    Slow(Duration),
}

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    authorization: Option<String>,
}

#[derive(Debug)]
struct Inner {
    mode: Mode,
    point: Vec<Value>,
    tier: Vec<Value>,
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

async fn fake_handle(req: HttpRequest, state: web::Data<Arc<Mutex<Inner>>>) -> HttpResponse {
    let path = req.path().to_owned();
    let (mode, point, tier) = {
        let mut inner = state.lock().expect("fake state");
        inner.seen.push(Seen {
            method: req.method().as_str().to_owned(),
            path: path.clone(),
            authorization: req
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned),
        });
        (inner.mode.clone(), inner.point.clone(), inner.tier.clone())
    };
    if let Mode::Slow(d) = mode {
        actix_web::rt::time::sleep(d).await;
    }
    if let Mode::Status(s) = mode {
        return err(s, "UNAVAILABLE", "loyalty answered an error");
    }
    if req.method() != Method::GET {
        return err(405, "METHOD", "the fake serves only the list routes");
    }
    match path.as_str() {
        "/api/v1/loyalty/point-rule-versions" => {
            if mode == Mode::PointNotAnArray {
                return HttpResponse::Ok().json(json!({ "items": point }));
            }
            HttpResponse::Ok().json(point)
        }
        "/api/v1/loyalty/tier-rule-versions" => {
            if let Mode::TierStatus(s) = mode {
                return err(s, "UNAVAILABLE", "tier list failed");
            }
            HttpResponse::Ok().json(tier)
        }
        _ => err(404, "NOT_FOUND", "no such route"),
    }
}

impl FakeLoyalty {
    fn start() -> Self {
        let inner = Arc::new(Mutex::new(Inner {
            mode: Mode::Up,
            point: Vec::new(),
            tier: Vec::new(),
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

    fn hold(&self, point: Vec<Value>, tier: Vec<Value>) {
        let mut inner = self.inner.lock().expect("fake state");
        inner.point = point;
        inner.tier = tier;
    }

    fn seen(&self) -> Vec<Seen> {
        self.inner.lock().expect("fake state").seen.clone()
    }
}

/// A POINT version exactly as Loyalty's `PointRuleVersionDto` serialises it (camelCase, decimals as strings).
fn point_dto(version: i32, status: &str) -> Value {
    json!({
        "id": Uuid::new_v4(),
        "tenantId": TENANT,
        "version": version,
        "basis": "SETTLE_THB",
        "pointsRatio": format!("0.{version}5"),
        "rounding": "FLOOR",
        "minimumBasisAmount": "200.00",
        "expiryMonths": 18,
        "validFrom": format!("2027-0{version}-01T00:00:00Z"),
        "validTo": null,
        "status": status,
        "approvedRef": if status == "APPROVED" { json!("AP-1-ref") } else { Value::Null },
        "approvedAt": if status == "APPROVED" { json!("2026-10-01T00:00:00Z") } else { Value::Null },
        "createdBy": Uuid::new_v4(),
        "createdAt": "2026-10-01T00:00:00Z"
    })
}

/// A TIER version exactly as Loyalty's `TierRuleVersionDto` serialises it.
fn tier_dto(version: i32, status: &str) -> Value {
    json!({
        "id": Uuid::new_v4(),
        "tenantId": TENANT,
        "version": version,
        "rollingMonths": 12,
        "tiers": [
            { "tierCode": "T1", "rank": 1, "qualification": { "basis": "SETTLE_THB", "minValue": "1000.00" }, "benefits": {} },
            { "tierCode": "T2", "rank": 2, "qualification": { "basis": "SETTLE_THB", "minValue": format!("{version}0000.00") }, "benefits": {} }
        ],
        "grace": { "months": version },
        "validFrom": "2027-01-01T00:00:00Z",
        "validTo": "2027-12-31T00:00:00Z",
        "status": status,
        "approvedRef": null,
        "approvedAt": null,
        "createdBy": null,
        "createdAt": "2026-10-01T00:00:00Z"
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
        let schema = unique("ly6_view");
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

fn user_of(spec: &str) -> Option<Actor> {
    let from = chrono::DateTime::<chrono::Utc>::UNIX_EPOCH;
    let (user_id, role) = match spec {
        "EXECUTIVE" => ("user-EXECUTIVE", Role::Executive),
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

    fn list(&self, key: &str) -> Vec<Value> {
        self.body[key]
            .as_array()
            .unwrap_or_else(|| panic!("`{key}` is an array: {}", self.text))
            .clone()
    }
}

struct Env<'a> {
    base_url: &'a str,
    token: Option<&'a str>,
    timeout_ms: &'a str,
}

async fn get_with(room: &Room, who: &str, uri: &str, env: Env<'_>) -> Answer {
    let mut pairs = vec![
        ("XC_LOYALTY_BASE_URL", env.base_url),
        ("XC_LOYALTY_TENANT_ID", TENANT),
        ("XC_LOYALTY_WEBHOOK_SECRET", SECRET),
        ("XC_LOYALTY_TIMEOUT_MS", env.timeout_ms),
    ];
    if let Some(token) = env.token {
        pairs.push(("XC_LOYALTY_SERVICE_TOKEN", token));
    }
    let settings = common::settings(&pairs);
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
        .method(Method::GET)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((AS, who));
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn get(room: &Room, who: &str) -> Answer {
    get_with(
        room,
        who,
        ROUTE,
        Env {
            base_url: &room.loyalty.url,
            token: Some(SERVICE_TOKEN),
            timeout_ms: "2000",
        },
    )
    .await
}

fn keys_of(item: &Value) -> Vec<String> {
    let mut keys: Vec<String> = item
        .as_object()
        .unwrap_or_else(|| panic!("an object: {item}"))
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

fn sorted(keys: &[&str]) -> Vec<String> {
    let mut keys: Vec<String> = keys.iter().map(|k| (*k).to_owned()).collect();
    keys.sort();
    keys
}

// ================================================================ rows 1–4: the list

/// Row 1. Both lists, newest `version` first, whatever order Loyalty returns them in; every status is listed.
#[actix_web::test]
async fn an_executive_sees_both_lists_newest_version_first() {
    let room = Room::start().await;
    room.loyalty.hold(
        vec![
            point_dto(1, "APPROVED"),
            point_dto(3, "DRAFT"),
            point_dto(2, "PENDING_APPROVAL"),
        ],
        vec![tier_dto(1, "APPROVED"), tier_dto(2, "REJECTED")],
    );
    let a = get(&room, "EXECUTIVE").await;
    a.ok("EXECUTIVE");
    let versions = |key: &str| -> Vec<i64> {
        a.list(key)
            .iter()
            .map(|v| v["version"].as_i64().expect("version is a number"))
            .collect()
    };
    assert_eq!(versions("pointVersions"), vec![3, 2, 1], "{}", a.text);
    assert_eq!(versions("tierVersions"), vec![2, 1], "{}", a.text);
    let statuses: Vec<&str> = a.body["pointVersions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["status"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(
        statuses,
        vec!["DRAFT", "PENDING_APPROVAL", "APPROVED"],
        "{}",
        a.text
    );
    room.finish().await;
}

/// Row 2. A POINT item carries exactly the [required] keys; the values are Loyalty's, as Loyalty sent them.
#[actix_web::test]
async fn a_point_item_carries_exactly_the_forwarded_fields() {
    let room = Room::start().await;
    let dto = point_dto(4, "APPROVED");
    room.loyalty.hold(vec![dto.clone()], vec![]);
    let a = get(&room, "EXECUTIVE").await;
    a.ok("EXECUTIVE");
    let items = a.list("pointVersions");
    assert_eq!(items.len(), 1, "{}", a.text);
    let item = &items[0];
    assert_eq!(keys_of(item), sorted(&POINT_KEYS), "{}", a.text);
    assert_eq!(item["versionId"], dto["id"], "versionId is Loyalty's id");
    for k in &POINT_KEYS[1..] {
        assert_eq!(
            item[*k], dto[*k],
            "{k} is forwarded as Loyalty sent it: {}",
            a.text
        );
    }
    room.finish().await;
}

/// Row 3. A TIER item carries exactly the [required] keys, `tiers` and `grace` forwarded as JSON.
#[actix_web::test]
async fn a_tier_item_carries_exactly_the_forwarded_fields() {
    let room = Room::start().await;
    let dto = tier_dto(5, "PENDING_APPROVAL");
    room.loyalty.hold(vec![], vec![dto.clone()]);
    let a = get(&room, "EXECUTIVE").await;
    a.ok("EXECUTIVE");
    let items = a.list("tierVersions");
    assert_eq!(items.len(), 1, "{}", a.text);
    let item = &items[0];
    assert_eq!(keys_of(item), sorted(&TIER_KEYS), "{}", a.text);
    assert_eq!(item["versionId"], dto["id"], "versionId is Loyalty's id");
    for k in &TIER_KEYS[1..] {
        assert_eq!(
            item[*k], dto[*k],
            "{k} is forwarded as Loyalty sent it: {}",
            a.text
        );
    }
    room.finish().await;
}

/// Row 4. `status` is a free string (#2129 G4): a value v3 does not know is passed through, not refused.
#[actix_web::test]
async fn an_unknown_status_is_passed_through() {
    let room = Room::start().await;
    room.loyalty.hold(
        vec![point_dto(1, "ARCHIVED")],
        vec![tier_dto(1, "SOMETHING_NEW")],
    );
    let a = get(&room, "EXECUTIVE").await;
    a.ok("EXECUTIVE");
    assert_eq!(
        a.body["pointVersions"][0]["status"], "ARCHIVED",
        "{}",
        a.text
    );
    assert_eq!(
        a.body["tierVersions"][0]["status"], "SOMETHING_NEW",
        "{}",
        a.text
    );
    room.finish().await;
}

// ================================================================ row 5: empty

/// Row 5. Loyalty holds nothing: 200 with two empty arrays, and nothing else in the answer.
#[actix_web::test]
async fn no_versions_is_200_with_two_empty_lists() {
    let room = Room::start().await;
    let a = get(&room, "EXECUTIVE").await;
    a.ok("EXECUTIVE");
    assert_eq!(
        a.body,
        json!({ "pointVersions": [], "tierVersions": [] }),
        "{}",
        a.text
    );
    room.finish().await;
}

/// Row 5b. One list empty, the other not: each list stands on its own.
#[actix_web::test]
async fn one_empty_list_does_not_empty_the_other() {
    let room = Room::start().await;
    room.loyalty.hold(vec![], vec![tier_dto(1, "APPROVED")]);
    let a = get(&room, "EXECUTIVE").await;
    a.ok("EXECUTIVE");
    assert_eq!(a.list("pointVersions").len(), 0, "{}", a.text);
    assert_eq!(a.list("tierVersions").len(), 1, "{}", a.text);
    room.finish().await;
}

// ================================================================ rows 6–7: who may read

/// Row 6. Every role but EXECUTIVE is refused by the policy with its own `400` answer for a missing permission (as
/// PR 2050 pins for the two commands, `ly6_loyalty_rule_versions_test.rs:935-937`), and Loyalty is never asked.
#[actix_web::test]
async fn roles_without_loyalty_rules_manage_are_refused_and_loyalty_is_not_asked() {
    let room = Room::start().await;
    room.loyalty.hold(vec![point_dto(1, "APPROVED")], vec![]);
    for who in [
        "ADMIN",
        "FINANCE",
        "MARKETING",
        "RATE_MANAGER",
        "BRANCH_MANAGER",
        "TELLER",
    ] {
        let a = get(&room, who).await;
        assert_eq!(a.status, StatusCode::BAD_REQUEST, "{who}: {}", a.text);
        // `roleNotAllowed` on `EntryPath::StaffRpc`, `roleNotAllowedHere` on `EntryPath::Rpc`: either is the
        // policy's own refusal; which path the view inherits is the ticket's [proposed] §7, not pinned here.
        assert!(
            ["roleNotAllowed", "roleNotAllowedHere"].contains(&a.code().as_str()),
            "{who}: the policy's refusal, got {}",
            a.text
        );
        assert!(
            a.body.get("pointVersions").is_none(),
            "{who} sees no list: {}",
            a.text
        );
    }
    assert!(
        room.loyalty.seen().is_empty(),
        "Loyalty was asked: {:?}",
        room.loyalty.seen()
    );
    room.finish().await;
}

/// Row 7. Signed out: `401 signIn`, Loyalty never asked.
#[actix_web::test]
async fn a_signed_out_caller_is_asked_to_sign_in() {
    let room = Room::start().await;
    let a = get(&room, "nobody").await;
    assert_eq!(a.status, StatusCode::UNAUTHORIZED, "{}", a.text);
    assert_eq!(a.code(), "signIn", "{}", a.text);
    assert!(
        room.loyalty.seen().is_empty(),
        "Loyalty was asked: {:?}",
        room.loyalty.seen()
    );
    room.finish().await;
}

// ================================================================ rows 8–13: Loyalty fails ⇒ 503 loyaltyUnavailable

/// Row 8. Loyalty answers an error status on both lists ⇒ `503 loyaltyUnavailable` (R1, as #2127 pins for the
/// approve path). 401/404 count too: the view cannot read either way.
#[actix_web::test]
async fn loyalty_answering_an_error_is_503_loyalty_unavailable() {
    let room = Room::start().await;
    room.loyalty.hold(
        vec![point_dto(1, "APPROVED")],
        vec![tier_dto(1, "APPROVED")],
    );
    for status in [500, 502, 503, 401, 404] {
        room.loyalty.set_mode(Mode::Status(status));
        let a = get(&room, "EXECUTIVE").await;
        a.unavailable(&format!("Loyalty {status}"));
    }
    room.finish().await;
}

/// Row 9. Only the tier list fails ⇒ the whole view is 503; no half answer.
#[actix_web::test]
async fn one_list_failing_fails_the_whole_view() {
    let room = Room::start().await;
    room.loyalty.hold(
        vec![point_dto(1, "APPROVED")],
        vec![tier_dto(1, "APPROVED")],
    );
    room.loyalty.set_mode(Mode::TierStatus(500));
    let a = get(&room, "EXECUTIVE").await;
    a.unavailable("tier list 500");
    assert!(
        a.body.get("pointVersions").is_none(),
        "no half answer: {}",
        a.text
    );
    room.finish().await;
}

/// Row 10. Nothing listens at `XC_LOYALTY_BASE_URL` ⇒ 503.
#[actix_web::test]
async fn loyalty_unreachable_is_503() {
    let room = Room::start().await;
    let closed = {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = l.local_addr().expect("addr");
        drop(l);
        format!("http://{addr}")
    };
    let a = get_with(
        &room,
        "EXECUTIVE",
        ROUTE,
        Env {
            base_url: &closed,
            token: Some(SERVICE_TOKEN),
            timeout_ms: "2000",
        },
    )
    .await;
    a.unavailable("unreachable");
    room.finish().await;
}

/// Row 11. Loyalty slower than `XC_LOYALTY_TIMEOUT_MS` ⇒ 503 (the timeout comes from `LoyaltyConfig`).
#[actix_web::test]
async fn loyalty_timing_out_is_503() {
    let room = Room::start().await;
    room.loyalty
        .set_mode(Mode::Slow(Duration::from_millis(1500)));
    let a = get_with(
        &room,
        "EXECUTIVE",
        ROUTE,
        Env {
            base_url: &room.loyalty.url,
            token: Some(SERVICE_TOKEN),
            timeout_ms: "300",
        },
    )
    .await;
    a.unavailable("timeout");
    room.finish().await;
}

/// Row 12. A malformed Loyalty answer (not an array) ⇒ 503, not 200 with a guess and not 500.
#[actix_web::test]
async fn a_malformed_loyalty_answer_is_503() {
    let room = Room::start().await;
    room.loyalty.hold(vec![point_dto(1, "APPROVED")], vec![]);
    room.loyalty.set_mode(Mode::PointNotAnArray);
    let a = get(&room, "EXECUTIVE").await;
    a.unavailable("not an array");
    room.finish().await;
}

/// Row 12b. An item without a usable `id` or `version` ⇒ 503.
#[actix_web::test]
async fn an_item_without_id_or_version_is_503() {
    let room = Room::start().await;
    let mut no_id = point_dto(1, "APPROVED");
    no_id.as_object_mut().unwrap().remove("id");
    room.loyalty.hold(vec![no_id], vec![]);
    get(&room, "EXECUTIVE").await.unavailable("item without id");

    let mut no_version = tier_dto(1, "APPROVED");
    no_version.as_object_mut().unwrap().remove("version");
    room.loyalty.hold(vec![], vec![no_version]);
    get(&room, "EXECUTIVE")
        .await
        .unavailable("item without version");
    room.finish().await;
}

/// Row 13. `XC_LOYALTY_SERVICE_TOKEN` unset ⇒ 503 before any call (R3), Loyalty never asked.
#[actix_web::test]
async fn no_service_token_is_503_and_loyalty_is_not_asked() {
    let room = Room::start().await;
    room.loyalty.hold(vec![point_dto(1, "APPROVED")], vec![]);
    let a = get_with(
        &room,
        "EXECUTIVE",
        ROUTE,
        Env {
            base_url: &room.loyalty.url,
            token: None,
            timeout_ms: "2000",
        },
    )
    .await;
    a.unavailable("no token");
    assert!(
        room.loyalty.seen().is_empty(),
        "Loyalty was asked: {:?}",
        room.loyalty.seen()
    );
    room.finish().await;
}

// ================================================================ row 14: what goes out

/// Row 14. The view reads only: one GET per list route, each with `Authorization: Bearer <token>`; no write.
#[actix_web::test]
async fn the_view_sends_two_authorised_gets_and_nothing_else() {
    let room = Room::start().await;
    room.loyalty.hold(
        vec![point_dto(1, "APPROVED")],
        vec![tier_dto(1, "APPROVED")],
    );
    get(&room, "EXECUTIVE").await.ok("EXECUTIVE");
    let mut seen: Vec<(String, String)> = room
        .loyalty
        .seen()
        .iter()
        .map(|s| (s.method.clone(), s.path.clone()))
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        vec![
            (
                "GET".to_owned(),
                "/api/v1/loyalty/point-rule-versions".to_owned()
            ),
            (
                "GET".to_owned(),
                "/api/v1/loyalty/tier-rule-versions".to_owned()
            ),
        ],
        "{:?}",
        room.loyalty.seen()
    );
    for s in room.loyalty.seen() {
        assert_eq!(
            s.authorization.as_deref(),
            Some(format!("Bearer {SERVICE_TOKEN}").as_str()),
            "{s:?}"
        );
    }
    room.finish().await;
}

// ================================================================ row 15: the v3 view seam

/// Row 15. Like every v3 view (#500), a named branch that does not exist is `400 noSuchBranch`, before Loyalty.
#[actix_web::test]
async fn an_unknown_branch_is_no_such_branch() {
    let room = Room::start().await;
    let a = get_with(
        &room,
        "EXECUTIVE",
        &format!("{ROUTE}?branch={GONE}"),
        Env {
            base_url: &room.loyalty.url,
            token: Some(SERVICE_TOKEN),
            timeout_ms: "2000",
        },
    )
    .await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{}", a.text);
    assert_eq!(a.code(), "noSuchBranch", "{}", a.text);
    assert!(
        room.loyalty.seen().is_empty(),
        "Loyalty was asked: {:?}",
        room.loyalty.seen()
    );
    room.finish().await;
}

// ================================================================ row 16: the generated type

struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn exported() -> String {
    let view = VIEWS.iter().find(|v| v.name == VIEW).unwrap_or_else(|| {
        panic!("{VIEW} is a generated view (a file in src/presentation/http/v3/views/)")
    });
    let scratch = Scratch(std::env::temp_dir().join(unique("ly6_view_ts")));
    std::fs::create_dir_all(&scratch.0).expect("scratch dir");
    (view.export_types)(&scratch.0).unwrap_or_else(|e| panic!("export {VIEW}: {e}"));
    std::fs::read_to_string(scratch.0.join(format!("{VIEW}.ts")))
        .unwrap_or_else(|e| panic!("{VIEW} writes {VIEW}.ts: {e}"))
}

/// Row 16. The view exports `loyalty_rule_versions.ts` naming both lists and the item keys, and that file is
/// committed under `web/src/shared/api/generated/`.
#[test]
fn the_generated_type_is_exported_and_committed() {
    let ts = exported();
    for k in ["pointVersions", "tierVersions"]
        .iter()
        .chain(POINT_KEYS.iter())
        .chain(TIER_KEYS.iter())
    {
        assert!(ts.contains(k), "{k} in {VIEW}.ts:\n{ts}");
    }
    assert!(!ts.contains("bigint"), "no bigint:\n{ts}");
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../web/src/shared/api/generated/{VIEW}.ts"));
    let committed = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("web/src/shared/api/generated/{VIEW}.ts: {e}"));
    assert_eq!(committed, ts, "{VIEW}.ts is what the view generates");
}
