//! LY-6 (#2944): verify that AppState holds the single LoyaltyRuleWriter instance,
//! that tests can inject a fake via `with_loyalty_rule_writer`, and that both the
//! view (`loyalty_rule_versions`) and the command (`loyaltyRuleDraft`) read from AppState.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use serde_json::{Map, Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_loyalty::application::rule_publisher::{
    LoyaltyRuleError, LoyaltyRuleWriter, RuleKind, RuleVersion,
};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const VIEW_ROUTE: &str = "/api/v3/views/loyalty_rule_versions";
const DRAFT_ROUTE: &str = "/api/v3/commands/loyalty_rule_draft";
const AS_HEADER: &str = "x-test-as";
const ORG_ID: &str = "00000000-0000-0000-0000-000000000001";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const TENANT_ID: &str = "3f7c2a10-6b4e-4d8a-9c15-2e7a8b9d0c41";
const WEBHOOK_SECRET: &str = "whsec-shared-writer-test";
const SERVICE_TOKEN: &str = "ly6-service-token-in-settings";

static NEXT_SCHEMA: AtomicU64 = AtomicU64::new(0);

fn unique_schema(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT_SCHEMA.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

#[derive(Clone, Default)]
struct CallCounts {
    create_draft: usize,
    list: usize,
}

#[derive(Clone)]
struct CountingFakeWriter {
    point_id: Uuid,
    tier_id: Uuid,
    draft_id: Uuid,
    counts: Arc<Mutex<CallCounts>>,
}

impl CountingFakeWriter {
    fn new() -> Self {
        Self {
            point_id: Uuid::new_v4(),
            tier_id: Uuid::new_v4(),
            draft_id: Uuid::new_v4(),
            counts: Arc::new(Mutex::new(CallCounts::default())),
        }
    }

    fn counts(&self) -> CallCounts {
        self.counts.lock().unwrap().clone()
    }
}

impl LoyaltyRuleWriter for CountingFakeWriter {
    fn create_draft<'a>(
        &'a self,
        kind: RuleKind,
        _fields: &'a Map<String, Value>,
    ) -> Pin<Box<dyn Future<Output = Result<RuleVersion, LoyaltyRuleError>> + Send + 'a>> {
        Box::pin(async move {
            self.counts.lock().unwrap().create_draft += 1;
            Ok(RuleVersion {
                id: self.draft_id,
                kind,
                status: "DRAFT".to_string(),
                approved_ref: None,
                content: json!({
                    "version": 1,
                    "basis": "SETTLE_THB",
                    "pointsRatio": "0.25",
                    "rounding": "FLOOR",
                    "minimumBasisAmount": "200.00",
                    "expiryMonths": 18,
                    "validFrom": "2027-01-01T00:00:00Z",
                    "validTo": null
                }),
            })
        })
    }

    fn submit<'a>(
        &'a self,
        _kind: RuleKind,
        _id: Uuid,
    ) -> Pin<Box<dyn Future<Output = Result<RuleVersion, LoyaltyRuleError>> + Send + 'a>> {
        unimplemented!()
    }

    fn approve<'a>(
        &'a self,
        _kind: RuleKind,
        _id: Uuid,
        _approved_ref: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<RuleVersion, LoyaltyRuleError>> + Send + 'a>> {
        unimplemented!()
    }

    fn reject<'a>(
        &'a self,
        _kind: RuleKind,
        _id: Uuid,
        _reason: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<RuleVersion, LoyaltyRuleError>> + Send + 'a>> {
        unimplemented!()
    }

    fn get<'a>(
        &'a self,
        _kind: RuleKind,
        _id: Uuid,
    ) -> Pin<Box<dyn Future<Output = Result<Option<RuleVersion>, LoyaltyRuleError>> + Send + 'a>>
    {
        unimplemented!()
    }

    fn list<'a>(
        &'a self,
        kind: RuleKind,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<RuleVersion>, LoyaltyRuleError>> + Send + 'a>> {
        Box::pin(async move {
            self.counts.lock().unwrap().list += 1;
            let version = match kind {
                RuleKind::Point => RuleVersion {
                    id: self.point_id,
                    kind: RuleKind::Point,
                    status: "APPROVED".to_string(),
                    approved_ref: None,
                    content: json!({
                        "version": 5,
                        "basis": "SETTLE_THB",
                        "pointsRatio": "1.0",
                        "rounding": "FLOOR",
                        "minimumBasisAmount": "100.00",
                        "expiryMonths": 12,
                        "validFrom": "2026-01-01T00:00:00Z",
                        "validTo": null,
                    }),
                },
                RuleKind::Tier => RuleVersion {
                    id: self.tier_id,
                    kind: RuleKind::Tier,
                    status: "APPROVED".to_string(),
                    approved_ref: None,
                    content: json!({
                        "version": 3,
                        "rollingMonths": 12,
                        "tiers": [{ "code": "GOLD", "minVolumeThb": "1000.00" }],
                        "grace": { "months": 1 },
                        "validFrom": "2026-01-01T00:00:00Z",
                        "validTo": null,
                    }),
                },
            };
            Ok(vec![version])
        })
    }
}

struct Room {
    pool: PgPool,
    schema: String,
    guard: Arc<AppSessionGuard>,
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
        let schema = unique_schema("ly6_shared");
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

fn user_of(spec: &str) -> Option<Actor> {
    let from = chrono::DateTime::<chrono::Utc>::UNIX_EPOCH;
    let (user_id, role) = match spec {
        "EXECUTIVE" => ("user-EXECUTIVE", Role::Executive),
        _ => return None,
    };
    Some(Actor::new(
        user_id,
        ORG_ID,
        vec![ScopedRole::new(role, None, from, None)],
        None,
    ))
}

struct HeaderCallers;

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS_HEADER)
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

fn test_settings() -> xchange::config::Settings {
    test_settings_at("http://127.0.0.1:9")
}

fn test_settings_at(base_url: &str) -> xchange::config::Settings {
    common::settings(&[
        ("XC_LOYALTY_BASE_URL", base_url),
        ("XC_LOYALTY_TENANT_ID", TENANT_ID),
        ("XC_LOYALTY_WEBHOOK_SECRET", WEBHOOK_SECRET),
        ("XC_LOYALTY_TIMEOUT_MS", "1000"),
        ("XC_LOYALTY_SERVICE_TOKEN", SERVICE_TOKEN),
    ])
}

fn draft_payload() -> Value {
    json!({
        "kind": "POINT",
        "fields": {
            "version": 1,
            "basis": "SETTLE_THB",
            "pointsRatio": "0.25",
            "rounding": "FLOOR",
            "minimumBasisAmount": "200.00",
            "expiryMonths": 18,
            "validFrom": "2027-01-01T00:00:00Z"
        }
    })
}

async fn call_get_view(room: &Room, state: AppState) -> Answer {
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
        .uri(VIEW_ROUTE)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((AS_HEADER, "EXECUTIVE"));
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn call_post_draft(room: &Room, state: AppState, body: Value) -> Answer {
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let guard = room.guard.clone();
    let app = atest::init_service(
        App::new()
            .app_data(web::Data::from(guard))
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let key = Uuid::new_v4().to_string();
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(DRAFT_ROUTE)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((AS_HEADER, "EXECUTIVE"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header(("idempotency-key", key))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// 4.3(a): GET /api/v3/views/loyalty_rule_versions reads from AppState.loyalty_rule_writer.
#[actix_web::test]
async fn test_view_reads_injected_shared_writer() {
    let room = Room::start().await;
    let fake = CountingFakeWriter::new();
    let state = AppState::new(Arc::new(test_settings()), room.pool.clone())
        .with_loyalty_rule_writer(Some(Arc::new(fake.clone())));

    let ans = call_get_view(&room, state).await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "view must return 200 with injected fake: {}",
        ans.text
    );
    assert_eq!(
        fake.counts().list,
        2,
        "fake writer list() must be called twice (points and tiers)"
    );
    assert_eq!(
        ans.body["pointVersions"][0]["versionId"],
        fake.point_id.to_string()
    );
    assert_eq!(
        ans.body["tierVersions"][0]["versionId"],
        fake.tier_id.to_string()
    );

    room.finish().await;
}

/// 4.3(b): loyaltyRuleDraft command uses AppState.loyalty_rule_writer.
#[actix_web::test]
async fn test_draft_command_uses_injected_shared_writer() {
    let room = Room::start().await;
    let fake = CountingFakeWriter::new();
    let state = AppState::new(Arc::new(test_settings()), room.pool.clone())
        .with_loyalty_rule_writer(Some(Arc::new(fake.clone())));

    let ans = call_post_draft(&room, state, draft_payload()).await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "draft command must return 200 with injected fake: {}",
        ans.text
    );
    assert_eq!(
        fake.counts().create_draft,
        1,
        "fake writer create_draft() must be called once"
    );
    assert_eq!(ans.body["versionId"], fake.draft_id.to_string());
    assert_eq!(ans.body["status"], "DRAFT");

    room.finish().await;
}

/// 4.3(c): with_loyalty_rule_writer(None) causes both view and command to answer 503 loyaltyUnavailable.
#[actix_web::test]
async fn test_none_writer_answers_503_loyalty_unavailable() {
    let room = Room::start().await;
    // The settings point at this listener: a writer built from settings would connect to it.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe listener");
    probe.set_nonblocking(true).expect("non-blocking probe");
    let settings = Arc::new(test_settings_at(&format!(
        "http://{}",
        probe.local_addr().unwrap()
    )));
    let state_view =
        AppState::new(settings.clone(), room.pool.clone()).with_loyalty_rule_writer(None);

    let view_ans = call_get_view(&room, state_view).await;
    assert_eq!(
        view_ans.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "view must answer 503 when loyalty_rule_writer is None: {}",
        view_ans.text
    );
    assert_eq!(view_ans.body["error"]["code"], "loyaltyUnavailable");

    let state_draft = AppState::new(settings, room.pool.clone()).with_loyalty_rule_writer(None);
    let draft_ans = call_post_draft(&room, state_draft, draft_payload()).await;
    assert_eq!(
        draft_ans.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "draft command must answer 503 when loyalty_rule_writer is None: {}",
        draft_ans.text
    );
    assert_eq!(draft_ans.body["error"]["code"], "loyaltyUnavailable");
    assert!(
        matches!(probe.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "with the writer set to None, nothing may call the loyalty service"
    );

    room.finish().await;
}
