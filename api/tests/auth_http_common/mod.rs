//! Shared by the T4c tests: the server's own route table on a PostgreSQL schema
//! of the test's own, a fake of the custody port, and the replay that holds the
//! routes to what v2 answered.
//! Provided with ticket T4c — do not edit.
#![allow(dead_code)]

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::http::{Method, header};
use actix_web::{Error, test};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::application::account::service::AccountService;
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xchange::bootstrap::AppState;

use crate::common::settings;

pub const HOST: &str = "desk.example";
pub const ORIGIN: &str = "https://desk.example";
pub const OWNER_PASSWORD: &str = "owner-password-01";

pub const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
pub const WRONG: &str = "ชื่อบัญชีหรือรหัสผ่านไม่ถูกต้อง";
pub const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
pub const OWNER_ONLY: &str = "เฉพาะเจ้าของระบบเท่านั้น";
pub const AUTH_REQUIRED: &str = "EMPLOYEE_AUTH_REQUIRED";
pub const EMPLOYEE_DATA: &str = "ข้อมูลพนักงานไม่ครบหรือไม่ถูกต้อง";
pub const SERVER_ERROR: &str = "เซิร์ฟเวอร์ขัดข้อง";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

// ── the custody port, faked ──────────────────────────────────────────────

#[derive(Default)]
pub struct FakeCustody {
    pub operations: Mutex<HashMap<String, Value>>,
    pub calls: Mutex<Vec<Value>>,
    /// When set, every method fails with this sentence.
    pub refuse_with: Mutex<Option<String>>,
}

// ── the database and the app ─────────────────────────────────────────────

pub struct Desk {
    pub pool: PgPool,
    pub schema: String,
    pub custody: Arc<FakeCustody>,
}

impl Desk {
    /// A fresh schema with `api/migrations/` applied and the owner `boss` created.
    ///
    /// `TEST_DATABASE_URL` must name an existing database whose name ends in
    /// `_test`. When it is absent the test FAILS; nothing here skips.
    pub async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The auth route tests need a real PostgreSQL and do not skip. Point it at an \
                 existing database whose name ends in `_test`, e.g.\n\
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
        let schema = unique("t4c");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(30))
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(16)
            .acquire_timeout(Duration::from_secs(30))
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()))
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");

        // The owner, as after `xchange create-owner --username boss` (ticket T4b).
        AccountService::new(
            Arc::new(PgAccountRepository::new(pool.clone())),
            PasswordHasher::with_cost(8, 1, 1),
        )
        .create_owner("boss", OWNER_PASSWORD, SystemTime::now().into())
        .await
        .expect("create the owner");
        // LG-4 (#929, 1:1): a shift's opening now leaves MAIN's ledger SAFE (design §4.4).
        lg4_fund::lg4_fund_safe(&pool, "MAIN").await;

        Self {
            pool,
            schema,
            custody: Arc::new(FakeCustody::default()),
        }
    }

    /// A branch besides `MAIN` (which the migrations of ticket T5 create).
    pub async fn branch(&self, id: &str) {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1) ON CONFLICT (id) DO NOTHING")
            .bind(id)
            .execute(&self.pool)
            .await
            .expect("insert a branch");
    }

    pub fn state(&self) -> AppState {
        AppState::new(Arc::new(settings(&[])), self.pool.clone())
    }

    pub async fn employee_id(&self, username: &str) -> String {
        sqlx::query_scalar("SELECT id::text FROM employees WHERE username = $1")
            .bind(username)
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("the employee {username}: {e}"))
    }

    pub async fn owner_session(&self, name: &str) -> String {
        let mut conn = self.pool.acquire().await.expect("a connection");
        let valid_from = chrono::DateTime::<chrono::Utc>::UNIX_EPOCH;
        let id = xc_testdb::people::seed_person(
            &mut conn,
            xchange::presentation::pages::SEEDED_XC_ORG_ID,
            name,
            &[
                xc_testdb::people::RoleRow {
                    role_code: "EXECUTIVE",
                    branch_id: None,
                    valid_from,
                    valid_to: None,
                },
                xc_testdb::people::RoleRow {
                    role_code: "ADMIN",
                    branch_id: None,
                    valid_from,
                    valid_to: None,
                },
            ],
        )
        .await
        .unwrap_or_else(|e| panic!("seed owner {name}: {e}"));
        xc_testdb::sessions::seed_session(
            &mut conn,
            &id,
            None,
            chrono::Utc::now() + chrono::Duration::hours(12),
        )
        .await
        .unwrap_or_else(|e| panic!("seed owner session {name}: {e}"))
    }

    pub async fn staff_session(&self, name: &str, role_code: &str, branch: &str) -> String {
        let mut conn = self.pool.acquire().await.expect("a connection");
        let valid_from = chrono::DateTime::<chrono::Utc>::UNIX_EPOCH;
        let id = xc_testdb::people::seed_person(
            &mut conn,
            xchange::presentation::pages::SEEDED_XC_ORG_ID,
            name,
            &[xc_testdb::people::RoleRow {
                role_code,
                branch_id: Some(branch),
                valid_from,
                valid_to: None,
            }],
        )
        .await
        .unwrap_or_else(|e| panic!("seed staff {name}: {e}"));
        xc_testdb::sessions::seed_session(
            &mut conn,
            &id,
            Some(branch),
            chrono::Utc::now() + chrono::Duration::hours(12),
        )
        .await
        .unwrap_or_else(|e| panic!("seed staff session {name}: {e}"))
    }

    pub async fn drop_schema(self) {
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.pool)
        .await
        .expect("drop the test schema");
        self.pool.close().await;
    }
}

/// The server's whole route table (`bootstrap::configure`).
#[macro_export]
macro_rules! desk_app {
    ($desk:expr) => {{
        let state = $desk.state();
        $crate::auth_http_common::TestApp::new(
            actix_web::test::init_service(
                actix_web::App::new()
                    .configure(move |cfg| xchange::bootstrap::configure(cfg, state)),
            )
            .await,
            |t| t.to_request(),
        )
    }};
}

/// The same without any custody port: what the server is until ticket T11a registers one.
#[macro_export]
macro_rules! desk_app_without_custody {
    ($desk:expr) => {{
        let state = $desk.state();
        $crate::auth_http_common::TestApp::new(
            actix_web::test::init_service(
                actix_web::App::new()
                    .configure(move |cfg| xchange::bootstrap::configure(cfg, state)),
            )
            .await,
            |t| t.to_request(),
        )
    }};
}

// ── one request ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct Call {
    pub method: Option<Method>,
    pub path: String,
    /// `None` = no Origin header.
    pub origin: Option<String>,
    /// `None` = no X-XChange-Request header.
    pub marker: Option<String>,
    /// `None` = no X-Forwarded-Proto header.
    pub proto: Option<String>,
    pub cookie: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub content_type: Option<String>,
}

impl Call {
    /// A POST as the kept pages send it: same origin, the marker header, behind the proxy.
    pub fn post(path: &str, body: Value) -> Self {
        Self {
            path: path.to_owned(),
            origin: Some(ORIGIN.to_owned()),
            marker: Some("1".to_owned()),
            proto: Some("https".to_owned()),
            body: body.to_string(),
            content_type: Some("application/json".to_owned()),
            ..Self::default()
        }
    }

    pub fn raw(path: &str, body: &str) -> Self {
        let mut call = Self::post(path, Value::Null);
        call.body = body.to_owned();
        call.content_type = Some("text/plain;charset=UTF-8".to_owned());
        call
    }

    pub fn owner(mut self, token: &str) -> Self {
        self.add_cookie("__Host-xc_owner", token);
        self
    }

    pub fn staff(mut self, token: &str) -> Self {
        self.add_cookie("__Host-xc_staff", token);
        self
    }

    pub fn session(mut self, token: &str) -> Self {
        self.add_cookie(xc_testdb::sessions::SESSION_COOKIE, token);
        self
    }

    fn add_cookie(&mut self, name: &str, value: &str) {
        let pair = format!("{name}={value}");
        self.cookie = Some(match self.cookie.take() {
            Some(existing) => format!("{existing}; {pair}"),
            None => pair,
        });
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn method(mut self, method: Method) -> Self {
        self.method = Some(method);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CookieChange {
    Set { token: String, max_age: u64 },
    Cleared { max_age: u64 },
}

#[derive(Debug, Clone)]
pub struct Answer {
    pub status: u16,
    pub content_type: String,
    pub cache_control: Option<String>,
    /// "owner" / "staff" → what the answer did to that cookie.
    pub cookies: HashMap<String, CookieChange>,
    pub text: String,
}

impl Answer {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.text)
            .unwrap_or_else(|e| panic!("the body is JSON ({e}): {:?}", self.text))
    }

    pub fn error(&self) -> String {
        self.json()["error"]
            .as_str()
            .unwrap_or_else(|| panic!("an error answer: {}", self.text))
            .to_owned()
    }

    pub fn token(&self, which: &str) -> String {
        match self.cookies.get(which) {
            Some(CookieChange::Set { token, .. }) => token.clone(),
            other => panic!(
                "the answer sets the {which} cookie: {other:?} / {}",
                self.text
            ),
        }
    }
}

/// v2's cookie line, exactly (`server.mjs:7811`, `7501`): the attributes, their order and spelling.
fn parse_set_cookie(line: &str) -> (String, CookieChange) {
    let (pair, rest) = line
        .split_once("; ")
        .unwrap_or_else(|| panic!("a Set-Cookie line with attributes: {line:?}"));
    let (attributes, max_age) = rest
        .rsplit_once("; Max-Age=")
        .unwrap_or_else(|| panic!("Max-Age is the last attribute: {line:?}"));
    assert_eq!(
        attributes, "Path=/; HttpOnly; Secure; SameSite=Strict",
        "the cookie attributes are v2's, in v2's order: {line:?}"
    );
    let max_age: u64 = max_age
        .parse()
        .unwrap_or_else(|_| panic!("Max-Age is a number: {line:?}"));
    let (name, value) = pair.split_once('=').expect("name=value");
    let which = match name {
        "__Host-xc_owner" => "owner",
        "__Host-xc_staff" => "staff",
        other => panic!("an unknown cookie {other:?} in {line:?}"),
    };
    let change = if value.is_empty() {
        CookieChange::Cleared { max_age }
    } else {
        assert!(
            value.len() == 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "a session cookie is 64 lower-case hex characters: {line:?}"
        );
        CookieChange::Set {
            token: value.to_owned(),
            max_age,
        }
    };
    (which.to_owned(), change)
}

type CallFn = dyn Fn(Call) -> Pin<Box<dyn Future<Output = Answer>>>;

/// The app under test with its request type hidden, so that helpers need not name it.
pub struct TestApp {
    call: Box<CallFn>,
}

impl TestApp {
    /// `to_request` is always `|t| t.to_request()`; it is passed in so that the request type
    /// is inferred where the app is built.
    pub fn new<S, R, B>(service: S, to_request: fn(test::TestRequest) -> R) -> Self
    where
        S: Service<R, Response = ServiceResponse<B>, Error = Error> + 'static,
        R: 'static,
        B: MessageBody + 'static,
    {
        let service = std::rc::Rc::new(service);
        Self {
            call: Box::new(move |call| {
                let service = service.clone();
                Box::pin(async move { exchange(&*service, to_request, call).await })
            }),
        }
    }
}

pub async fn send(app: &TestApp, call: Call) -> Answer {
    (app.call)(call).await
}

async fn exchange<S, R, B>(app: &S, to_request: fn(test::TestRequest) -> R, call: Call) -> Answer
where
    S: Service<R, Response = ServiceResponse<B>, Error = Error>,
    B: MessageBody,
{
    let method = call.method.clone().unwrap_or(Method::POST);
    let mut request = test::TestRequest::default()
        .method(method.clone())
        .uri(&call.path)
        .insert_header((header::HOST, HOST));
    if let Some(proto) = &call.proto {
        request = request.insert_header(("x-forwarded-proto", proto.as_str()));
    }
    if let Some(origin) = &call.origin {
        request = request.insert_header((header::ORIGIN, origin.as_str()));
    }
    if let Some(marker) = &call.marker {
        request = request.insert_header(("x-xchange-request", marker.as_str()));
    }
    if let Some(cookie) = &call.cookie {
        request = request.insert_header((header::COOKIE, cookie.as_str()));
    }
    for (name, value) in &call.headers {
        request = request.insert_header((name.as_str(), value.as_str()));
    }
    if method != Method::GET {
        if let Some(content_type) = &call.content_type {
            request = request.insert_header((header::CONTENT_TYPE, content_type.as_str()));
        }
        request = request.set_payload(call.body.clone());
    }
    let response = test::call_service(app, to_request(request)).await;
    let status = response.status().as_u16();
    let text_of = |name: header::HeaderName| {
        response
            .headers()
            .get(name)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
    };
    let content_type = text_of(header::CONTENT_TYPE).unwrap_or_default();
    let cache_control = text_of(header::CACHE_CONTROL);
    let mut cookies = HashMap::new();
    for line in response.headers().get_all(header::SET_COOKIE) {
        let (which, change) = parse_set_cookie(line.to_str().expect("an ASCII cookie"));
        assert!(
            cookies.insert(which.clone(), change).is_none(),
            "the {which} cookie is set once per answer"
        );
    }
    let bytes = test::read_body(response).await;
    Answer {
        status,
        content_type,
        cache_control,
        cookies,
        text: String::from_utf8(bytes.to_vec()).expect("a UTF-8 body"),
    }
}

/// Signs the owner in at `/api/auth` and gives back the owner token.
pub async fn owner_signs_in(app: &TestApp) -> String {
    let answer = send(
        app,
        Call::post(
            "/api/auth",
            json!({ "action": "login", "payload": { "username": "boss", "password": OWNER_PASSWORD } }),
        )
        .header("x-real-ip", &unique("owner-address")),
    )
    .await;
    assert_eq!(answer.status, 200, "the owner signs in: {}", answer.text);
    answer.token("owner")
}

/// Creates an employee and takes them past the first password,
/// through the staff door. Gives back (employee id, staff token).
pub async fn working_employee(
    app: &TestApp,
    desk: &Desk,
    _owner: &str,
    username: &str,
    role: &str,
    branch: &str,
) -> (String, String) {
    let first = "first-password-1";
    let password = "second-password-2";
    AccountService::new(
        Arc::new(PgAccountRepository::new(desk.pool.clone())),
        PasswordHasher::with_cost(8, 1, 1),
    )
    .employees(
        "create",
        &json!({
            "username": username, "name": format!("คุณ {username}"), "role": role,
            "branchId": branch, "password": first
        }),
        "owner",
        SystemTime::now().into(),
    )
    .await
    .unwrap_or_else(|e| panic!("create {username}: {e}"));
    let address = unique("staff-address");
    let login = send(
        app,
        Call::post(
            "/api/staff/login",
            json!({ "payload": { "username": username, "password": first } }),
        )
        .header("x-real-ip", &address),
    )
    .await;
    assert_eq!(
        login.status, 200,
        "first sign-in of {username}: {}",
        login.text
    );
    let changed = send(
        app,
        Call::post(
            "/api/staff/password",
            json!({ "payload": { "password": password } }),
        )
        .staff(&login.token("staff")),
    )
    .await;
    assert_eq!(
        changed.status, 200,
        "first password of {username}: {}",
        changed.text
    );
    let again = send(
        app,
        Call::post(
            "/api/staff/login",
            json!({ "payload": { "username": username, "password": password } }),
        )
        .header("x-real-ip", &address),
    )
    .await;
    assert_eq!(
        again.status, 200,
        "second sign-in of {username}: {}",
        again.text
    );
    (desk.employee_id(username).await, again.token("staff"))
}

// ── the v2 fixture ───────────────────────────────────────────────────────

pub fn fixture() -> Value {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/auth/v2_auth_http_cases.json");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("the fixture is JSON")
}

pub fn story(fixture: &Value, name: &str) -> Value {
    fixture["stories"]
        .as_array()
        .expect("stories")
        .iter()
        .find(|s| s["name"] == name)
        .unwrap_or_else(|| panic!("the fixture has no story '{name}'"))
        .clone()
}

/// The attempt limit counts in quarter-hour windows of the wall clock, which these routes read
/// themselves. A story that counts attempts must not straddle a window: when the next one
/// starts in less than `margin`, wait for it.
pub async fn stay_inside_one_attempt_window(margin: Duration) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let left = Duration::from_secs(900) - Duration::from_millis((now.as_millis() % 900_000) as u64);
    if left < margin {
        tokio::time::sleep(left + Duration::from_secs(1)).await;
    }
}

#[derive(Default)]
pub struct Symbols {
    tokens: HashMap<String, String>,
    employees: Vec<String>,
}

impl Symbols {
    fn token(&self, value: &Value) -> String {
        let text = value.as_str().expect("a cookie value");
        if text.starts_with("$t") {
            self.tokens
                .get(text)
                .unwrap_or_else(|| panic!("the fixture uses {text} before v2 made it"))
                .clone()
        } else {
            text.to_owned()
        }
    }

    /// `$eN` → the id v3 gave that employee.
    fn resolve(&self, value: &Value) -> Value {
        match value {
            Value::String(s) => {
                let real = s
                    .strip_prefix("$e")
                    .and_then(|n| n.parse::<usize>().ok())
                    .and_then(|n| self.employees.get(n.checked_sub(1)?));
                Value::String(real.cloned().unwrap_or_else(|| s.clone()))
            }
            Value::Array(items) => Value::Array(items.iter().map(|v| self.resolve(v)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), self.resolve(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// The id v3 gave an employee → `$eN`.
    fn symbolise(&self, value: &Value) -> Value {
        match value {
            Value::String(s) => match self.employees.iter().position(|id| id == s) {
                Some(i) => Value::String(format!("$e{}", i + 1)),
                None => value.clone(),
            },
            Value::Array(items) => Value::Array(items.iter().map(|v| self.symbolise(v)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), self.symbolise(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

/// Replays one story of the fixture through the app and compares every answer.
pub async fn replay(app: &TestApp, desk: &Desk, story: &Value) {
    for id in story["branches"].as_object().expect("branches").keys() {
        desk.branch(id).await;
    }
    let mut symbols = Symbols::default();
    for (index, step) in story["steps"].as_array().expect("steps").iter().enumerate() {
        let label = format!(
            "story '{}', step {index}: {}",
            story["name"].as_str().unwrap(),
            step["note"].as_str().unwrap()
        );
        // route deleted by #1027 (OR-7.m T6)
        if step["request"]["path"] == "/api/employees" {
            if step["request"]["json"]["action"] == "create" && step["response"]["status"] == 200 {
                let payload = symbols.resolve(&step["request"]["json"]["payload"]);
                AccountService::new(
                    Arc::new(PgAccountRepository::new(desk.pool.clone())),
                    PasswordHasher::with_cost(8, 1, 1),
                )
                .employees("create", &payload, "owner", SystemTime::now().into())
                .await
                .expect("seed employee from v2 fixture");
                let username = payload["username"].as_str().unwrap().trim().to_lowercase();
                symbols.employees.push(desk.employee_id(&username).await);
            }
            continue;
        }
        if let Some(custody) = step.get("custody") {
            desk.custody.operations.lock().unwrap().insert(
                custody["branch"].as_str().unwrap().to_owned(),
                symbols.resolve(&custody["operations"]),
            );
            continue;
        }

        let request = &step["request"];
        let body = match (request.get("raw"), request.get("json")) {
            (Some(Value::String(raw)), _) => raw.clone(),
            (Some(repeat), _) => repeat["repeat"]
                .as_str()
                .unwrap()
                .repeat(repeat["times"].as_u64().unwrap() as usize),
            (None, Some(body)) => symbols.resolve(body).to_string(),
            (None, None) => String::new(),
        };
        let mut call = Call {
            method: Some(Method::from_str(request["method"].as_str().unwrap()).unwrap()),
            path: request["path"].as_str().unwrap().to_owned(),
            origin: request["origin"].as_str().map(str::to_owned),
            marker: request["marker"].as_str().map(str::to_owned),
            proto: Some("https".to_owned()),
            body,
            // v2 never looks at the content type; a raw body goes out as the browser's default.
            content_type: Some(
                if request.get("raw").is_some() {
                    "text/plain;charset=UTF-8"
                } else {
                    "application/json"
                }
                .to_owned(),
            ),
            ..Call::default()
        };
        if let Some(cookies) = request.get("cookies") {
            if !cookies["owner"].is_null() {
                call = call.owner(&symbols.token(&cookies["owner"]));
            }
            if !cookies["staff"].is_null() {
                call = call.staff(&symbols.token(&cookies["staff"]));
            }
        }
        if let Some(headers) = request.get("headers").and_then(Value::as_object) {
            for (name, value) in headers {
                call = call.header(name, value.as_str().unwrap());
            }
        }

        let calls_before = desk.custody.calls.lock().unwrap().len();
        let answer = send(app, call).await;
        let want = &step["response"];

        assert_eq!(
            answer.status, want["status"],
            "{label}: status — v3 answered {}",
            answer.text
        );
        if want.get("text").is_some() {
            // the 404 of a route that is not there is `pages::fallback`'s (tickets T1b, T1c)
            assert_eq!(answer.text, want["text"].as_str().unwrap(), "{label}: body");
        } else {
            assert_eq!(
                answer.content_type, "application/json",
                "{label}: Content-Type"
            );
            assert_eq!(
                answer.cache_control.as_deref(),
                want["cacheControl"].as_str(),
                "{label}: Cache-Control"
            );
        }

        // cookies: which, set or cleared, for how long
        let mut expected = HashMap::new();
        for (which, change) in want["setCookies"].as_object().unwrap() {
            let max_age = change["maxAge"].as_u64().unwrap();
            if change["cleared"] == true {
                expected.insert(which.clone(), CookieChange::Cleared { max_age });
            } else {
                let symbol = change["token"].as_str().unwrap().to_owned();
                let token = answer.token(which);
                assert!(
                    !symbols.tokens.values().any(|t| t == &token),
                    "{label}: every sign-in gets a token of its own"
                );
                symbols.tokens.insert(symbol, token.clone());
                expected.insert(which.clone(), CookieChange::Set { token, max_age });
            }
        }
        assert_eq!(answer.cookies, expected, "{label}: Set-Cookie");

        if let Some(want_json) = want.get("json") {
            // an employee the step created gets the next symbol
            if request["path"] == "/api/employees"
                && request["json"]["action"] == "create"
                && answer.status == 200
            {
                let username = request["json"]["payload"]["username"]
                    .as_str()
                    .unwrap()
                    .trim()
                    .to_lowercase();
                symbols.employees.push(desk.employee_id(&username).await);
            }
            assert_eq!(
                symbols.symbolise(&answer.json()),
                *want_json,
                "{label}: body"
            );
        }

        let made: Vec<Value> = desk.custody.calls.lock().unwrap()[calls_before..]
            .iter()
            .map(|c| symbols.symbolise(c))
            .collect();
        let want_calls = step
            .get("custodyCalls")
            .cloned()
            .unwrap_or_else(|| json!([]));
        assert_eq!(
            Value::Array(made),
            want_calls,
            "{label}: what custody was asked to do"
        );
    }
}

#[path = "../lg4_fund/mod.rs"]
mod lg4_fund;
