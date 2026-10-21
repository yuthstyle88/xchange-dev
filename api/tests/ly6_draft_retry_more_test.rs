//! #2947 (§9 implementer rows) — `loyaltyRuleDraft` retry edge cases:
//! - client sends its own `fields.id`: XC forwards it unchanged and does not look it up on 409 ⇒ 409 `staleVersion`;
//! - the same key in another branch derives a different draft id;
//! - `get` after the 409 fails (Loyalty down) ⇒ 503 `loyaltyUnavailable`.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, HttpResponse, HttpServer, test as atest, web};
use chrono::Utc;
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
const SECRET: &str = "whsec-2947-draft-retry-more";
const DRAFT: &str = "/api/v3/commands/loyalty_rule_draft";
const SERVICE_TOKEN: &str = "ly6-2947-dummy-service-token";
const UNAVAILABLE: &str = "loyaltyUnavailable";
const STALE: &str = "staleVersion";
const POINT_ROUTE: &str = "point-rule-versions";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn key() -> String {
    unique("ly6-2947-key")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Up,
    ApplyCreateThenFail,
    FailGet,
}

#[derive(Debug, Clone)]
struct Version {
    id: Uuid,
    dto: Value,
}

#[derive(Debug)]
struct Inner {
    mode: Mode,
    versions: Vec<Version>,
    gets: usize,
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

fn point_dto(body: &Map<String, Value>, id: Uuid) -> Value {
    json!({
        "id": id,
        "tenantId": TENANT,
        "version": body.get("version").and_then(Value::as_i64).unwrap_or(1),
        "basis": body.get("basis").and_then(Value::as_str).unwrap_or("SETTLE_THB"),
        "pointsRatio": body.get("pointsRatio").and_then(Value::as_str).unwrap_or("0.25"),
        "rounding": body.get("rounding").and_then(Value::as_str).unwrap_or("FLOOR"),
        "minimumBasisAmount": body.get("minimumBasisAmount").and_then(Value::as_str).unwrap_or("200.00"),
        "expiryMonths": body.get("expiryMonths").cloned().unwrap_or(Value::Null),
        "validFrom": body.get("validFrom").and_then(Value::as_str).unwrap_or("2027-01-01T00:00:00Z"),
        "validTo": body.get("validTo").cloned().unwrap_or(Value::Null),
        "status": "DRAFT",
        "approvedRef": null,
        "approvedAt": null,
        "createdBy": null,
        "createdAt": Utc::now().to_rfc3339(),
    })
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
    if path.strip_prefix("/api/v1/loyalty/") != Some(POINT_ROUTE) {
        return err(404, "NOT_FOUND", "no such route");
    }
    match method.as_str() {
        "GET" => {
            inner.gets += 1;
            if inner.mode == Mode::FailGet {
                return err(503, "UNAVAILABLE", "loyalty list down");
            }
            let list: Vec<Value> = inner.versions.iter().map(|v| v.dto.clone()).collect();
            HttpResponse::Ok().json(list)
        }
        "POST" => {
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
            let dto = point_dto(obj, id);
            let taken = inner
                .versions
                .iter()
                .any(|v| v.id == id || v.dto["version"] == dto["version"]);
            if taken {
                return err(409, "CONFLICT", "rule version already exists");
            }
            inner.versions.push(Version {
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
            gets: 0,
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

    fn gets(&self) -> usize {
        self.inner.lock().expect("fake state").gets
    }
}

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

struct Room {
    pool: PgPool,
    schema: String,
    guard: Arc<AppSessionGuard>,
    loyalty: FakeLoyalty,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL must be set");
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("ly6_2947_more");
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

async fn draft_with_payload(room: &Room, payload: Value, key: &str) -> Answer {
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
        .set_payload(payload.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn draft(room: &Room, kind: &str, fields: Value, key: &str) -> Answer {
    draft_with_payload(room, json!({ "kind": kind, "fields": fields }), key).await
}

/// §9 row 5: client sends its own `fields.id` ⇒ XC sends it unchanged and does **not** look it up
/// on 409 ⇒ 409 `staleVersion` (a client id proves nothing about who made the row).
#[actix_web::test]
async fn client_supplied_id_is_forwarded_unchanged_and_not_looked_up_on_409() {
    let room = Room::start().await;
    let client_id = Uuid::new_v4();
    let mut fields = point_fields(3);
    fields["id"] = Value::String(client_id.to_string());

    room.loyalty.set_mode(Mode::ApplyCreateThenFail);
    let first = draft(&room, "POINT", fields.clone(), &key()).await;
    assert_eq!(
        first.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        first.text
    );

    // Retry with a client-supplied id: Loyalty answers 409 because `client_id` is stored,
    // and XC must NOT call `get` or claim that draft as its own.
    let retry = draft(&room, "POINT", fields, &key()).await;
    assert_eq!(retry.status, StatusCode::CONFLICT, "{}", retry.text);
    assert_eq!(retry.code(), STALE, "{}", retry.text);
    assert_eq!(
        room.loyalty.gets(),
        0,
        "no get lookup when client supplied fields.id"
    );

    // And when a fresh version with a client-supplied id succeeds, XC forwards that id unchanged.
    let client_id_2 = Uuid::new_v4();
    let mut fields_2 = point_fields(4);
    fields_2["id"] = Value::String(client_id_2.to_string());
    let ok = draft(&room, "POINT", fields_2, &key()).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    assert_eq!(
        ok.body["versionId"].as_str(),
        Some(client_id_2.to_string().as_str())
    );
    room.finish().await;
}

/// §9 row 6: the same key in another branch produces a different derived id (unit test on the id helper).
#[test]
fn same_key_in_another_branch_derives_a_different_id() {
    use xc_kernel::ids::BranchId;
    use xc_loyalty::application::rule_publisher::RuleKind;
    use xchange::infrastructure::idempotency::store::IdempotencyKey;
    use xchange::presentation::http::v3::loyalty_rule_draft::derive_draft_id;

    let idem_key = IdempotencyKey::new(key());
    let branch_hq = BranchId::new("hq");
    let branch_siam = BranchId::new("siam");

    let id_hq = derive_draft_id(RuleKind::Point, &branch_hq, &idem_key);
    let id_hq_again = derive_draft_id(RuleKind::Point, &branch_hq, &idem_key);
    let id_siam = derive_draft_id(RuleKind::Point, &branch_siam, &idem_key);
    let id_tier = derive_draft_id(RuleKind::Tier, &branch_hq, &idem_key);
    assert_eq!(id_hq, id_hq_again);
    assert_ne!(
        id_hq, id_siam,
        "same key in another branch must derive a different id"
    );
    assert_ne!(
        id_hq, id_tier,
        "same key for another kind must derive a different id"
    );
}

/// §9 row 7: `get` after the 409 fails (Loyalty down) ⇒ 503 `loyaltyUnavailable`.
#[actix_web::test]
async fn get_failure_after_409_answers_503_loyalty_unavailable() {
    let room = Room::start().await;
    let k = key();
    let fields = point_fields(5);

    room.loyalty.set_mode(Mode::ApplyCreateThenFail);
    let first = draft(&room, "POINT", fields.clone(), &k).await;
    assert_eq!(
        first.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        first.text
    );

    room.loyalty.set_mode(Mode::FailGet);
    let retry = draft(&room, "POINT", fields, &k).await;
    assert_eq!(
        retry.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "when get after 409 fails, XC answers 503: {}",
        retry.text
    );
    assert_eq!(retry.code(), UNAVAILABLE, "{}", retry.text);
    assert_eq!(room.loyalty.gets(), 1, "attempted get after 409");
    room.finish().await;
}
