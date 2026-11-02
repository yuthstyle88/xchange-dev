//! `POST /api/app/register` creates the Identity login for an unknown phone, then binds it (design
//! `docs/design/blueprint/customer-identity-consent.md` §4.6, rows S1–S8 of §4.6.2).
//!
//! Root cause pinned here: a real visitor could not sign up. `register_customer` signed in an *existing* Identity
//! user (`customer_signup.rs:93-96`) and nothing created one for a visitor, so `/app/sign-up` answered
//! `401 invalidCredentials` (#564 issuecomment-6571785796).
//!
//! Written before the implementation by the ticket author claude/b091f5e0. Do not edit, delete or weaken; add tests
//! in a file of your own. `identity_signin_common` answers each path with one fixed reply, and its header forbids
//! editing it, so this file brings its own fake Identity that can answer a path differently in turn
//! ([`SeqIdentity`]).
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use auth_http_common::Desk;
use chrono::Utc;
use identity_signin_common::*;
use serde_json::{Value, json};
use sqlx::{AssertSqlSafe, PgPool};
use xc_customer::application::consent_service::ConsentService;
use xc_customer::domain::consent::Purpose;

const PHONE: &str = "+66812345678";
const PASSWORD: &str = "Visitor-pw-2026x";
const INVALID: &str = r#"{"error":"invalid_credentials"}"#;

// ---------------------------------------------------------------- a fake Identity that answers in turn

type Answer = (u16, String);

/// Identity over real HTTP. A path answers its queued replies first, oldest first, then its standing reply.
#[derive(Clone)]
struct SeqIdentity {
    port: u16,
    queued: Arc<Mutex<HashMap<String, VecDeque<Answer>>>>,
    standing: Arc<Mutex<HashMap<String, Answer>>>,
    received: Arc<Mutex<Vec<(String, String)>>>,
}

impl SeqIdentity {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the fake Identity");
        let port = listener.local_addr().expect("its address").port();
        let me = Self {
            port,
            queued: Arc::default(),
            standing: Arc::default(),
            received: Arc::default(),
        };
        me.always(
            "/.well-known/jwks.json",
            200,
            &json!({ "keys": [{ "kid": "xc-test-1", "kty": "OKP", "crv": "Ed25519",
                               "alg": "EdDSA", "use": "sig", "x": GOOD_X }] })
            .to_string(),
        );
        let served = me.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let served = served.clone();
                std::thread::spawn(move || served.serve(stream));
            }
        });
        me
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// From now on `path` answers this, once its queued replies are used up.
    fn always(&self, path: &str, status: u16, body: &str) {
        self.standing
            .lock()
            .unwrap()
            .insert(path.to_owned(), (status, body.to_owned()));
    }

    /// The next request to `path` answers this, once.
    fn then(&self, path: &str, status: u16, body: &str) {
        self.queued
            .lock()
            .unwrap()
            .entry(path.to_owned())
            .or_default()
            .push_back((status, body.to_owned()));
    }

    /// The JSON bodies sent to `path`, oldest first.
    fn received(&self, path: &str) -> Vec<Value> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p == path)
            .map(|(_, b)| serde_json::from_str(b).unwrap_or(Value::Null))
            .collect()
    }

    fn serve(&self, mut stream: TcpStream) {
        let mut reader = BufReader::new(stream.try_clone().expect("clone the stream"));
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let path = line
            .split_whitespace()
            .nth(1)
            .unwrap_or("")
            .split('?')
            .next()
            .unwrap_or("")
            .to_owned();
        let mut length = 0usize;
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" || h == "\n" {
                break;
            }
            if let Some((name, value)) = h.trim_end().split_once(':') {
                if name.trim().eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
        }
        let mut body = vec![0u8; length];
        let _ = reader.read_exact(&mut body);
        self.received
            .lock()
            .unwrap()
            .push((path.clone(), String::from_utf8_lossy(&body).into_owned()));
        let queued = self
            .queued
            .lock()
            .unwrap()
            .get_mut(&path)
            .and_then(VecDeque::pop_front);
        let (status, text) = queued
            .or_else(|| self.standing.lock().unwrap().get(&path).cloned())
            .unwrap_or((404, json!({ "error": "not_found" }).to_string()));
        let reply = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
            text.len()
        );
        let _ = stream.write_all(reply.as_bytes());
        let _ = stream.flush();
    }
}

/// Identity's login answer for the login it has just created: the phone is **not** verified (no OTP was sent).
fn new_login_body() -> String {
    json!({
        "identityId": SUB_A,
        "access_token": VALID_A,
        "token_type": "Bearer",
        "expires_in": 900,
        "refresh_token": REFRESH_1,
        "phone": PHONE,
        "emailVerified": false,
        "phoneVerified": false,
        "mustChangePassword": false
    })
    .to_string()
}

fn registered_body() -> String {
    json!({ "identityId": SUB_A, "registered": true }).to_string()
}

// ---------------------------------------------------------------- the shop

struct Visit {
    desk: Desk,
    identity: SeqIdentity,
    app: App,
}

impl Visit {
    /// A fresh schema, TERMS published in `th`, and an Identity that knows nobody yet.
    async fn start() -> Self {
        common::install_test_pii_keys();
        let desk = Desk::start().await;
        let identity = SeqIdentity::start();
        let org = organization(&desk.pool).await;
        ConsentService::pg(desk.pool.clone())
            .publish(&org, None, Purpose::Terms, "th", "ข้อความยินยอม", Utc::now())
            .await
            .expect("publish TERMS");
        let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
        Self {
            desk,
            identity,
            app,
        }
    }

    /// Identity knows nobody: the first login is refused, register creates the login, and every later login
    /// succeeds.
    fn identity_knows_nobody(&self) {
        self.identity.then("/auth/login", 401, INVALID);
        self.identity
            .always("/auth/register", 201, &registered_body());
        self.identity.always("/auth/login", 200, &new_login_body());
    }

    async fn sign_up(&self, identifier: &str, lang: &str) -> Reply {
        self.app
            .send(Req::post(
                "/api/app/register",
                json!({ "identifier": identifier, "password": PASSWORD,
                        "displayName": "Visitor One", "lang": lang, "marketing": [] }),
            ))
            .await
    }

    fn pool(&self) -> &PgPool {
        &self.desk.pool
    }

    async fn count(&self, sql: &'static str) -> i64 {
        sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql))
            .fetch_one(self.pool())
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// (users, CUSTOMER user_roles, customers, TERMS consents, user_sessions, PHONE identities)
    async fn written(&self) -> (i64, i64, i64, i64, i64, i64) {
        (
            self.count("SELECT count(*) FROM users").await,
            self.count("SELECT count(*) FROM user_roles WHERE role_code = 'CUSTOMER'")
                .await,
            self.count("SELECT count(*) FROM customers").await,
            self.count("SELECT count(*) FROM consents WHERE purpose = 'TERMS'")
                .await,
            self.count("SELECT count(*) FROM user_sessions").await,
            self.count("SELECT count(*) FROM customer_identities WHERE type = 'PHONE'")
                .await,
        )
    }

    async fn finish(self) {
        self.desk.drop_schema().await;
    }
}

fn plus(
    a: (i64, i64, i64, i64, i64, i64),
    b: (i64, i64, i64, i64, i64, i64),
) -> (i64, i64, i64, i64, i64, i64) {
    (
        a.0 + b.0,
        a.1 + b.1,
        a.2 + b.2,
        a.3 + b.3,
        a.4 + b.4,
        a.5 + b.5,
    )
}

// ---------------------------------------------------------------- S1–S8

/// S1: an unknown phone gets an Identity login, one CUSTOMER user, one customer and one TERMS consent, and is
/// signed in. Identity has not verified the phone, so no PHONE identity is written.
#[actix_web::test]
async fn s1_an_unknown_phone_signs_up_with_one_login_and_one_customer() {
    let v = Visit::start().await;
    v.identity_knows_nobody();
    let before = v.written().await;

    let reply = v.sign_up(PHONE, "th").await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    assert!(
        reply.json()["customerId"].as_str().is_some(),
        "{}",
        reply.text
    );
    reply.session();

    assert_eq!(v.written().await, plus(before, (1, 1, 1, 1, 1, 0)));
    let registered = v.identity.received("/auth/register");
    assert_eq!(registered.len(), 1, "one Identity login created");
    assert_eq!(registered[0]["phone"], json!(PHONE));
    assert_eq!(registered[0]["password"], json!(PASSWORD));
    assert_eq!(registered[0]["mustChangePassword"], json!(false));
    assert!(registered[0].get("username").is_none() && registered[0].get("email").is_none());
    assert_eq!(
        v.identity.received("/auth/login").len(),
        2,
        "login, register, login"
    );
    v.finish().await;
}

/// S2: after S1 the visitor signs in with the same phone and password and gets a session of that CUSTOMER user.
#[actix_web::test]
async fn s2_the_new_customer_then_signs_in() {
    let v = Visit::start().await;
    v.identity_knows_nobody();
    let signed_up = v.sign_up(PHONE, "th").await;
    assert_eq!(signed_up.status, 200, "{}", signed_up.text);
    let user_id = signed_up.json()["user"]["id"].clone();

    let reply = v
        .app
        .send(Req::post(
            "/api/auth/login",
            json!({ "identifier": PHONE, "password": PASSWORD }),
        ))
        .await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    reply.session();
    assert_eq!(reply.json()["user"]["id"], user_id, "{}", reply.text);
    assert_eq!(
        v.count("SELECT count(*) FROM customers").await,
        1,
        "signing in makes no second customer"
    );
    assert_eq!(v.identity.received("/auth/register").len(), 1);
    v.finish().await;
}

/// S3: a phone that already has a login, with a wrong password, is told it is already registered; nothing is
/// written and no second login is tried.
#[actix_web::test]
async fn s3_a_known_phone_answers_already_registered() {
    let v = Visit::start().await;
    v.identity.always("/auth/login", 401, INVALID);
    v.identity
        .always("/auth/register", 409, r#"{"error":"phone_taken"}"#);
    let before = v.written().await;

    let reply = v.sign_up(PHONE, "th").await;
    assert_eq!(
        (reply.status, reply.refusal().0.as_str()),
        (409, "alreadyRegistered"),
        "{}",
        reply.text
    );
    assert!(reply.sets_no_session());
    assert_eq!(v.written().await, before);
    assert_eq!(v.identity.received("/auth/register").len(), 1);
    assert_eq!(
        v.identity.received("/auth/login").len(),
        1,
        "no login after a refused register"
    );
    v.finish().await;
}

/// S4: Identity's password policy refuses the password: `422 weakPassword`, nothing written.
#[actix_web::test]
async fn s4_a_password_the_policy_refuses_is_weak_password() {
    let v = Visit::start().await;
    v.identity.always("/auth/login", 401, INVALID);
    v.identity.always(
        "/auth/register",
        422,
        r#"{"error":"password_policy_violation"}"#,
    );
    let before = v.written().await;

    let reply = v.sign_up(PHONE, "th").await;
    assert_eq!(
        (reply.status, reply.refusal().0.as_str()),
        (422, "weakPassword"),
        "{}",
        reply.text
    );
    assert_eq!(v.written().await, before);
    v.finish().await;
}

/// S5: no TERMS text in the request's language: `409 consentTextMissing` before any login is created.
#[actix_web::test]
async fn s5_no_terms_text_creates_no_login() {
    let v = Visit::start().await;
    v.identity_knows_nobody();
    let before = v.written().await;

    let reply = v.sign_up(PHONE, "en").await;
    assert_eq!(
        (reply.status, reply.refusal().0.as_str()),
        (409, "consentTextMissing"),
        "{}",
        reply.text
    );
    assert_eq!(v.written().await, before);
    assert_eq!(
        v.identity.received("/auth/register").len(),
        0,
        "Identity is not asked to create a login for a sign-up that cannot finish"
    );
    v.finish().await;
}

/// S6: an unknown e-mail keeps today's `401 invalidCredentials`; Identity's register needs a phone, so it is not
/// asked.
#[actix_web::test]
async fn s6_an_unknown_email_is_invalid_credentials_and_creates_no_login() {
    let v = Visit::start().await;
    v.identity_knows_nobody();
    let before = v.written().await;

    let reply = v.sign_up("visitor@example.com", "th").await;
    assert_eq!(
        (reply.status, reply.refusal().0.as_str()),
        (401, "invalidCredentials"),
        "{}",
        reply.text
    );
    assert_eq!(v.written().await, before);
    assert_eq!(v.identity.received("/auth/register").len(), 0);
    v.finish().await;
}

/// S7: the login made, but the login after it fails: `503`, nothing written; the retry binds with one login.
#[actix_web::test]
async fn s7_a_failed_login_after_register_is_finished_by_the_retry() {
    let v = Visit::start().await;
    v.identity.then("/auth/login", 401, INVALID);
    v.identity.always("/auth/register", 201, &registered_body());
    v.identity
        .always("/auth/login", 503, r#"{"error":"unavailable"}"#);
    let before = v.written().await;

    let first = v.sign_up(PHONE, "th").await;
    assert_eq!(
        (first.status, first.refusal().0.as_str()),
        (503, "identityUnavailable"),
        "{}",
        first.text
    );
    assert_eq!(v.written().await, before);

    v.identity.always("/auth/login", 200, &new_login_body());
    let retry = v.sign_up(PHONE, "th").await;
    assert_eq!(retry.status, 200, "{}", retry.text);
    assert_eq!(v.written().await, plus(before, (1, 1, 1, 1, 1, 0)));
    assert_eq!(
        v.identity.received("/auth/register").len(),
        1,
        "the retry signs in to the login already made"
    );
    v.finish().await;
}

/// S8: a phone typed the Thai way is sent to Identity in E.164.
#[actix_web::test]
async fn s8_a_thai_formatted_phone_is_registered_in_e164() {
    let v = Visit::start().await;
    v.identity_knows_nobody();

    let reply = v.sign_up("081-234-5678", "th").await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    let registered = v.identity.received("/auth/register");
    assert_eq!(registered.len(), 1);
    assert_eq!(registered[0]["phone"], json!(PHONE));
    v.finish().await;
}
