//! OR-3 (#550 design `docs/design/blueprint/org-roles-branches.md` §5, §10): shared by
//! `identity_signin_test.rs`. A fake Identity over real HTTP, fixed test tokens, the server's own route
//! table, and one request.
//!
//! Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or weaken
//! this file; add your own helpers in another file.
//!
//! **The tokens below are fixed strings**, so this file needs no signing library. They were minted once with
//! Ed25519 (RFC 8032) from the 32-byte seed `[7; 32]` (the "good" key, published by the fake JWKS under kid
//! `xc-test-1`) or `[9; 32]` (`BAD_SIG_A` only: same kid, other key). Header `{"alg":"EdDSA","typ":"JWT",
//! "kid":"xc-test-1"}`; claims `{"sub", "iss":"auth-service", "aud", "iat":1760000000, "exp"}` with
//! `aud = "xchange"` and `exp = 4102444800` (2100-01-01) unless the name says otherwise. Decode the middle
//! part (base64url) to read them.
#![allow(dead_code)]

use std::collections::HashMap;
use std::future::Future;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::http::{Method, header};
use actix_web::{Error, test};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use xchange::bootstrap::AppState;

pub const HOST: &str = "desk.example";
pub const ORIGIN: &str = "https://desk.example";
pub const BROWSER_ADDRESS: &str = "203.0.113.7";

pub const SESSION_COOKIE: &str = "__Host-xc_session";
pub const OWNER_COOKIE: &str = "__Host-xc_owner";
pub const STAFF_COOKIE: &str = "__Host-xc_staff";
pub const SESSION_SECONDS: i64 = 43_200;

pub const CLIENT_ID: &str = "e06ca64f-0000-4000-8000-00000000c11e";
pub const CLIENT_SECRET: &str = "test-client-secret-never-in-a-browser-0001";
pub const API_KEY: &str = "test-identity-api-key-0001";
pub const AUDIENCE: &str = "xchange";
pub const ISSUER: &str = "auth-service";

/// The Identity subjects the tokens below carry.
pub const SUB_A: &str = "11111111-aaaa-4aaa-8aaa-000000000001";
pub const SUB_B: &str = "11111111-aaaa-4aaa-8aaa-000000000002";

/// The public half of the good key, as the fake JWKS publishes it.
pub const GOOD_X: &str = "6kpsY-KcUgq-9VB7Ey7F-ZVHdq6-vnuSQh7qaRRG0iw";

pub const VALID_A: &str = "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCIsImtpZCI6InhjLXRlc3QtMSJ9.eyJzdWIiOiIxMTExMTExMS1hYWFhLTRhYWEtOGFhYS0wMDAwMDAwMDAwMDEiLCJpc3MiOiJhdXRoLXNlcnZpY2UiLCJhdWQiOiJ4Y2hhbmdlIiwiaWF0IjoxNzYwMDAwMDAwLCJleHAiOjQxMDI0NDQ4MDB9.12K5WNhSB8h6XhjUpILAVFQvWTMGUP4WrckTtwBHdkHgSvHzboL8tfj7MX0BhQwzPPsONJAlW-GrkkZd90y8AA";
pub const VALID_B: &str = "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCIsImtpZCI6InhjLXRlc3QtMSJ9.eyJzdWIiOiIxMTExMTExMS1hYWFhLTRhYWEtOGFhYS0wMDAwMDAwMDAwMDIiLCJpc3MiOiJhdXRoLXNlcnZpY2UiLCJhdWQiOiJ4Y2hhbmdlIiwiaWF0IjoxNzYwMDAwMDAwLCJleHAiOjQxMDI0NDQ4MDB9.rOsw4lFLmqlAFIZ-4m8LyuTUgqnOLTs3pdvLtDNJPqkwgX8x7UipaBpitIA7iEOsyhE27DfOmUwIeslFPcxoDw";
/// `aud = "jobs"`.
pub const WRONG_AUD_A: &str = "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCIsImtpZCI6InhjLXRlc3QtMSJ9.eyJzdWIiOiIxMTExMTExMS1hYWFhLTRhYWEtOGFhYS0wMDAwMDAwMDAwMDEiLCJpc3MiOiJhdXRoLXNlcnZpY2UiLCJhdWQiOiJqb2JzIiwiaWF0IjoxNzYwMDAwMDAwLCJleHAiOjQxMDI0NDQ4MDB9.UWeSsb3cFkPuxWlyqA0tISDjhAkmM1gTb7__ZquRuFZ7ydUkC3IDj-fFmBtLUxB7bL9CNmS4d2oj7HKyXNrpDQ";
/// `exp = 1700000000` (2023-11-14).
pub const EXPIRED_A: &str = "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCIsImtpZCI6InhjLXRlc3QtMSJ9.eyJzdWIiOiIxMTExMTExMS1hYWFhLTRhYWEtOGFhYS0wMDAwMDAwMDAwMDEiLCJpc3MiOiJhdXRoLXNlcnZpY2UiLCJhdWQiOiJ4Y2hhbmdlIiwiaWF0IjoxNzYwMDAwMDAwLCJleHAiOjE3MDAwMDAwMDB9.SfTak4W4awtcGU1DfOS8RD0XfivPXhJssggyyCez5qPOkeKP7lQ3Lc58XMO0aGzlg0G8kSIDAX0mzn647OhmAw";
/// Signed by the `[9; 32]` key under the good key's kid.
pub const BAD_SIG_A: &str = "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCIsImtpZCI6InhjLXRlc3QtMSJ9.eyJzdWIiOiIxMTExMTExMS1hYWFhLTRhYWEtOGFhYS0wMDAwMDAwMDAwMDEiLCJpc3MiOiJhdXRoLXNlcnZpY2UiLCJhdWQiOiJ4Y2hhbmdlIiwiaWF0IjoxNzYwMDAwMDAwLCJleHAiOjQxMDI0NDQ4MDB9.dpt5lLBT-T5nE8leAk81gBTEjn1HZ-te2weM50cx7ycxDul-gtH8Su5Af5rMehegH-kOP1qZuDT3M7TawGB8Cw";

/// The refresh tokens the fake hands out (Identity's `<token id>.<secret>` form).
pub const REFRESH_1: &str = "7b0c4c1e-1111-4111-8111-000000000001.refresh-secret-one-0123456789";
pub const REFRESH_2: &str = "7b0c4c1e-2222-4222-8222-000000000002.refresh-secret-two-0123456789";

/// The v3 refusal sentence when Identity cannot be used (unknown code, caller-side code, unreadable
/// body, 5xx, unreachable, a token that does not verify).
pub const UNAVAILABLE: &str = "ระบบยืนยันตัวตนไม่พร้อมใช้งาน กรุณาลองใหม่ภายหลัง";

// ── the fake Identity ─────────────────────────────────────────────────────

/// One request the fake received.
#[derive(Debug, Clone)]
pub struct Received {
    pub method: String,
    pub path: String,
    /// Lower-case names.
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl Received {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("Identity was sent JSON ({e}): {:?}", self.body))
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

/// Identity over real HTTP on `127.0.0.1:<port>`. Every path answers what `answer` last set for it
/// (default: 404 `{"error":"not_found"}`); `/.well-known/jwks.json` publishes the good key.
pub struct FakeIdentity {
    pub port: u16,
    answers: Arc<Mutex<HashMap<String, (u16, String)>>>,
    received: Arc<Mutex<Vec<Received>>>,
}

impl FakeIdentity {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the fake Identity");
        let port = listener.local_addr().expect("its address").port();
        let answers: Arc<Mutex<HashMap<String, (u16, String)>>> = Arc::default();
        answers.lock().unwrap().insert(
            "/.well-known/jwks.json".to_owned(),
            (
                200,
                json!({ "keys": [{ "kid": "xc-test-1", "kty": "OKP", "crv": "Ed25519",
                                   "alg": "EdDSA", "use": "sig", "x": GOOD_X }] })
                .to_string(),
            ),
        );
        let received: Arc<Mutex<Vec<Received>>> = Arc::default();
        let (a, r) = (answers.clone(), received.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let (a, r) = (a.clone(), r.clone());
                std::thread::spawn(move || serve(stream, &a, &r));
            }
        });
        Self {
            port,
            answers,
            received,
        }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// From now on `path` answers `status` with `body` (sent as is; `""` = an empty body).
    pub fn answer(&self, path: &str, status: u16, body: &str) {
        self.answers
            .lock()
            .unwrap()
            .insert(path.to_owned(), (status, body.to_owned()));
    }

    /// What Identity's `POST /auth/login` returns on success (`108-platform-services`
    /// `crates/auth/src/contract/mod.rs` `LoginResponse`, body transport).
    pub fn login_answers(&self, path: &str, access_token: &str, refresh_token: &str) {
        self.answer(
            path,
            200,
            &login_body(access_token, refresh_token).to_string(),
        );
    }

    /// Every request to `path`, oldest first.
    pub fn received(&self, path: &str) -> Vec<Received> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.path == path)
            .cloned()
            .collect()
    }

    /// Every request to any path except the JWKS.
    pub fn calls(&self) -> Vec<Received> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.path != "/.well-known/jwks.json")
            .cloned()
            .collect()
    }
}

pub fn login_body(access_token: &str, refresh_token: &str) -> Value {
    json!({
        "identityId": SUB_A,
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": 900,
        "refresh_token": refresh_token,
        "phone": "+66812345678",
        "emailVerified": false,
        "phoneVerified": true,
        "mustChangePassword": false
    })
}

fn serve(
    mut stream: TcpStream,
    answers: &Mutex<HashMap<String, (u16, String)>>,
    received: &Mutex<Vec<Received>>,
) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone the stream"));
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let path = parts
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .to_owned();
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some((name, value)) = h.trim_end().split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    let _ = reader.read_exact(&mut body);
    received.lock().unwrap().push(Received {
        method,
        path: path.clone(),
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    });
    let (status, text) = answers
        .lock()
        .unwrap()
        .get(&path)
        .cloned()
        .unwrap_or((404, json!({ "error": "not_found" }).to_string()));
    let reply = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
    let _ = stream.write_all(reply.as_bytes());
    let _ = stream.flush();
}

/// A port nothing listens on.
pub fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("address").port();
    drop(listener);
    port
}

/// The five variables `/etc/xc-v3/identity-client.env` carries (design §5.2), plus the optional API key,
/// pointed at `base_url`.
pub fn identity_env(base_url: &str) -> Vec<(&'static str, String)> {
    vec![
        ("IDENTITY_BASE_URL", base_url.to_owned()),
        ("IDENTITY_CLIENT_ID", CLIENT_ID.to_owned()),
        ("IDENTITY_CLIENT_SECRET", CLIENT_SECRET.to_owned()),
        ("IDENTITY_AUDIENCE", AUDIENCE.to_owned()),
        ("IDENTITY_ISSUER", ISSUER.to_owned()),
        ("IDENTITY_API_KEY", API_KEY.to_owned()),
    ]
}

/// The server's state with exactly this environment (on top of the committed YAML).
pub fn state_with(pool: &PgPool, env: &[(&str, String)]) -> AppState {
    let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    AppState::new(Arc::new(super::common::settings(&pairs)), pool.clone())
}

// ── the database ──────────────────────────────────────────────────────────

pub async fn organization(pool: &PgPool) -> String {
    sqlx::query_scalar("SELECT id::text FROM organizations WHERE code = 'XC'")
        .fetch_one(pool)
        .await
        .expect("OR-1's organisation")
}

/// A user bound to `subject` with `status` and these `(role, branch)` rows; its `users.id`.
pub async fn bound_user(
    pool: &PgPool,
    subject: &str,
    status: &str,
    name: &str,
    roles: &[(&str, Option<&str>)],
) -> String {
    let org = organization(pool).await;
    let id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, auth_subject, name, status, version, created_by) \
         VALUES ($1::uuid, $2, $3, $4, 1, 'test') RETURNING id::text",
    )
    .bind(&org)
    .bind(subject)
    .bind(name)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("insert a user");
    for (role, branch) in roles {
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
             VALUES ($1::uuid, $2, $3::uuid, $4, now() - interval '1 day')",
        )
        .bind(&id)
        .bind(role)
        .bind(&org)
        .bind(branch)
        .execute(pool)
        .await
        .expect("insert a role row");
    }
    id
}

pub async fn sessions_of(pool: &PgPool, user_id: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM user_sessions WHERE user_id = $1::uuid")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("count sessions")
}

pub async fn all_sessions(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM user_sessions")
        .fetch_one(pool)
        .await
        .expect("count sessions")
}

pub fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ── one request ───────────────────────────────────────────────────────────

/// One `Set-Cookie` line, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetCookie {
    pub value: String,
    pub attributes: String,
    pub max_age: i64,
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub cache_control: Option<String>,
    /// Cookie name → what this reply did to it.
    pub cookies: HashMap<String, SetCookie>,
    pub text: String,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.text)
            .unwrap_or_else(|e| panic!("the body is JSON ({e}): {:?}", self.text))
    }

    /// The v3 refusal `{"error":{"code","message"}}`: (code, message).
    pub fn refusal(&self) -> (String, String) {
        let body = self.json();
        let code = body["error"]["code"]
            .as_str()
            .unwrap_or_else(|| panic!("a v3 refusal with error.code: {}", self.text))
            .to_owned();
        let message = body["error"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("a v3 refusal with error.message: {}", self.text))
            .to_owned();
        (code, message)
    }

    /// The session cookie this reply set (non-empty value).
    pub fn session(&self) -> String {
        match self.cookies.get(SESSION_COOKIE) {
            Some(c) if !c.value.is_empty() => c.value.clone(),
            other => panic!("the reply sets {SESSION_COOKIE}: {other:?} / {}", self.text),
        }
    }

    pub fn sets_no_session(&self) -> bool {
        self.cookies
            .get(SESSION_COOKIE)
            .is_none_or(|c| c.value.is_empty())
    }
}

#[derive(Debug, Clone)]
pub struct Req {
    pub path: String,
    pub body: String,
    pub cookie: Option<String>,
    pub same_origin: bool,
}

impl Req {
    /// A POST as the web pages send it: same origin, the marker header, behind the proxy, from
    /// `BROWSER_ADDRESS`.
    pub fn post(path: &str, body: Value) -> Self {
        Self {
            path: path.to_owned(),
            body: body.to_string(),
            cookie: None,
            same_origin: true,
        }
    }

    pub fn with_session(mut self, token: &str) -> Self {
        self.cookie = Some(format!("{SESSION_COOKIE}={token}"));
        self
    }

    pub fn cross_site(mut self) -> Self {
        self.same_origin = false;
        self
    }
}

type CallFn = dyn Fn(Req) -> Pin<Box<dyn Future<Output = Reply>>>;

/// The app under test with its request type hidden.
pub struct App {
    call: Box<CallFn>,
}

impl App {
    pub async fn send(&self, req: Req) -> Reply {
        (self.call)(req).await
    }
}

/// The server's whole route table (`bootstrap::configure`) on `state`.
pub async fn app(state: AppState) -> App {
    let service = test::init_service(
        actix_web::App::new().configure(move |cfg| xchange::bootstrap::configure(cfg, state)),
    )
    .await;
    wrap(service, |t| t.to_request())
}

fn wrap<S, R, B>(service: S, to_request: fn(test::TestRequest) -> R) -> App
where
    S: Service<R, Response = ServiceResponse<B>, Error = Error> + 'static,
    R: 'static,
    B: MessageBody + 'static,
{
    let service = std::rc::Rc::new(service);
    App {
        call: Box::new(move |req| {
            let service = service.clone();
            Box::pin(async move { exchange(&*service, to_request, req).await })
        }),
    }
}

async fn exchange<S, R, B>(app: &S, to_request: fn(test::TestRequest) -> R, req: Req) -> Reply
where
    S: Service<R, Response = ServiceResponse<B>, Error = Error>,
    B: MessageBody,
{
    let mut request = test::TestRequest::default()
        .method(Method::POST)
        .uri(&req.path)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-forwarded-proto", "https"))
        .insert_header(("x-real-ip", BROWSER_ADDRESS))
        .insert_header((header::CONTENT_TYPE, "application/json"));
    if req.same_origin {
        request = request
            .insert_header((header::ORIGIN, ORIGIN))
            .insert_header(("x-xchange-request", "1"));
    }
    if let Some(cookie) = &req.cookie {
        request = request.insert_header((header::COOKIE, cookie.as_str()));
    }
    request = request.set_payload(req.body.clone());
    let response = test::call_service(app, to_request(request)).await;
    let status = response.status().as_u16();
    let cache_control = response
        .headers()
        .get(header::CACHE_CONTROL)
        .map(|v| v.to_str().expect("ASCII").to_owned());
    let mut cookies = HashMap::new();
    for line in response.headers().get_all(header::SET_COOKIE) {
        let line = line.to_str().expect("an ASCII cookie");
        let (pair, rest) = line.split_once("; ").unwrap_or((line, ""));
        let (name, value) = pair.split_once('=').expect("name=value");
        let (attributes, max_age) = rest.rsplit_once("; Max-Age=").unwrap_or((rest, "-1"));
        let previous = cookies.insert(
            name.to_owned(),
            SetCookie {
                value: value.to_owned(),
                attributes: attributes.to_owned(),
                max_age: max_age.parse().unwrap_or(-1),
            },
        );
        assert!(previous.is_none(), "{name} is set once per reply");
    }
    let bytes = test::read_body(response).await;
    Reply {
        status,
        cache_control,
        cookies,
        text: String::from_utf8(bytes.to_vec()).expect("a UTF-8 body"),
    }
}
