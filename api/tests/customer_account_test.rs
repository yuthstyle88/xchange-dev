//! CI-7 (#654): the customer account. An Identity user registers as a customer, records consent, updates the
//! profile and reads it; a session carries its `customer_id`. Design `docs/design/blueprint/customer-identity-consent.md`
//! (K5, K7, K8, K10, §4.3, §4.4, §9.1 row CI-7), the lane controller's rulings and 39-row behaviour table on #654
//! (issuecomment of 2026-10-10T05:21Z, rows R/C/P/V/S below), and the PHONE scope addition of 07:42Z (#559 D-16,
//! owner P10; rows H).
//!
//! Written before the implementation by the ticket author (claude/709ec749, on the lane controller's order). Do not
//! edit, delete or weaken these tests; add your own in another file.
//!
//! [required] by these tests:
//! - `POST /api/app/register`, a plain route like `/api/auth/invite/claim` (ruling 4; not a `CommandAction`): same
//!   origin; body `{"identifier", "password", "displayName", "lang": "th"|"en"|"zh", "marketing": ["SMS"|"EMAIL"|"PUSH"]}`;
//!   success `200 {"user": …, "customerId": "<id>"}` with the three sign-in cookies (`__Host-xc_session` set, the old
//!   two cleared), as the claim answers.
//! - `xchange::presentation::extractors::session_actor::resolve_session_actor(&SessionService, &PgPool, token, now)
//!   -> Option<access::Actor>` (ruling 5) and `xc_customer::application::customer_link::customer_of_user(&PgPool,
//!   user_id) -> AppResult<Option<String>>`. Both are scaffolded with `todo!()` in the commit before this file.
//! - A PHONE identity (07:42Z): `customer_identities` row `type 'PHONE'`, `status 'ACTIVE'`, `verified_at` set; its
//!   `encrypted_value`/`key_id` are `PiiCipher::encrypt(aad = customer_id ‖ "PHONE", plain = the E.164 text)`, and
//!   `lookup_hash = LookupHasher(PHONE, normalize(PHONE, phone))`. It is written only when Identity's login answer has a
//!   phone and `phoneVerified: true`.
//! - The commands `consentRecord` (`POST /api/app/consents`) and `profileUpdate` (`PUT /api/app/profile`) and the view
//!   `my_profile` (`GET /api/v3/views/my_profile`), behind `(EntryPath::Rpc, "customerSelf")` → `CustomerSelf`.
//!
//! Choices made here (the ticket records them; [proposed] in the table):
//! - The body of every refusal is v3's `{"error": {"code", "field"?, "message"}}`; the tests read `code` and `field`.
//! - A register that would bind a PHONE already ACTIVE on another customer of the organisation is refused
//!   `409 phoneInUse` and writes nothing (#554's one-active index decides it inside the same transaction).
//! - The command and view tests call without `?branch=`: a customer has no branch. Whatever the command seam does
//!   with a branchless CUSTOMER after OR-4b #767 is #767's; these tests pin what the customer sees.
//!
//! **Red until #767 merges, for the right reason:** every C/P/V test, because on `main` a CUSTOMER session still
//! converts through `legacy_actor` and answers `signIn` (rulings of 05:21Z). The R, H and S tests do not need #767.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use std::collections::HashMap;
use std::sync::Arc;

use actix_web::http::{Method, header};
use actix_web::test;
use auth_http_common::Desk;
use chrono::Utc;
use identity_signin_common::{
    BAD_SIG_A, EXPIRED_A, FakeIdentity, ORIGIN, REFRESH_1, SESSION_COOKIE, SUB_A, SUB_B, VALID_A,
    WRONG_AUD_A, bound_user, closed_port, identity_env, login_body, organization, state_with,
};
use serde_json::{Value, json};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::application::session::SessionService;
use xc_customer::application::consent_service::{ConsentService, Registration};
use xc_customer::application::customer_link::customer_of_user;
use xc_customer::domain::consent::{Channel, Purpose};
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, PiiKeys, Sealed, normalize};
use xchange::bootstrap::AppState;
use xchange::presentation::extractors::session_actor::resolve_session_actor;

const HOST: &str = "desk.example";
const PASSWORD: &str = "pw-from-the-app-0001";
const PHONE: &str = "+66812345678";

// ── one request ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Reply {
    status: u16,
    /// cookie name → (value, max-age)
    cookies: HashMap<String, (String, i64)>,
    text: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or_else(|e| {
            panic!("a JSON body ({e}), status {}: {:?}", self.status, self.text)
        })
    }
    /// (code, field) of a v3 refusal.
    fn refusal(&self) -> (String, Option<String>) {
        let body = self.json();
        let code = body["error"]["code"]
            .as_str()
            .unwrap_or_else(|| {
                panic!(
                    "a refusal with error.code, status {}: {}",
                    self.status, self.text
                )
            })
            .to_owned();
        (code, body["error"]["field"].as_str().map(str::to_owned))
    }
    fn session(&self) -> String {
        match self.cookies.get(SESSION_COOKIE) {
            Some((v, _)) if !v.is_empty() => v.clone(),
            other => panic!(
                "the reply sets {SESSION_COOKIE}: {other:?}, status {}: {}",
                self.status, self.text
            ),
        }
    }
    fn sets_no_session(&self) -> bool {
        self.cookies
            .get(SESSION_COOKIE)
            .is_none_or(|(v, _)| v.is_empty())
    }
}

#[derive(Debug, Clone)]
struct Call {
    method: Method,
    path: String,
    body: Option<Value>,
    session: Option<String>,
    key: Option<String>,
    same_origin: bool,
}

fn call(method: Method, path: &str, body: Option<Value>) -> Call {
    Call {
        method,
        path: path.to_owned(),
        body,
        session: None,
        key: None,
        same_origin: true,
    }
}

impl Call {
    fn by(mut self, session: &str) -> Self {
        self.session = Some(session.to_owned());
        self
    }
    fn key(mut self, key: &str) -> Self {
        self.key = Some(key.to_owned());
        self
    }
    fn cross_site(mut self) -> Self {
        self.same_origin = false;
        self
    }
}

/// One request through the server's whole route table, as a browser on the same origin sends it.
async fn send(state: &AppState, c: Call) -> Reply {
    let state = state.clone();
    let app = test::init_service(
        actix_web::App::new().configure(move |cfg| xchange::bootstrap::configure(cfg, state)),
    )
    .await;
    let mut req = test::TestRequest::default()
        .method(c.method.clone())
        .uri(&c.path)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-forwarded-proto", "https"))
        .insert_header(("x-real-ip", "203.0.113.7"))
        .insert_header((header::CONTENT_TYPE, "application/json"));
    if c.same_origin {
        req = req
            .insert_header((header::ORIGIN, ORIGIN))
            .insert_header(("x-xchange-request", "1"));
    }
    if let Some(s) = &c.session {
        req = req.insert_header((header::COOKIE, format!("{SESSION_COOKIE}={s}")));
    }
    if let Some(k) = &c.key {
        req = req.insert_header(("idempotency-key", k.as_str()));
    }
    if let Some(b) = &c.body {
        req = req.set_payload(b.to_string());
    }
    let response = test::call_service(&app, req.to_request()).await;
    let status = response.status().as_u16();
    let mut cookies = HashMap::new();
    for line in response.headers().get_all(header::SET_COOKIE) {
        let line = line.to_str().expect("an ASCII cookie");
        let (pair, rest) = line.split_once("; ").unwrap_or((line, ""));
        let (name, value) = pair.split_once('=').expect("name=value");
        let max_age = rest
            .rsplit_once("Max-Age=")
            .and_then(|(_, a)| a.split(';').next())
            .and_then(|a| a.trim().parse().ok())
            .unwrap_or(-1);
        cookies.insert(name.to_owned(), (value.to_owned(), max_age));
    }
    let bytes = test::read_body(response).await;
    Reply {
        status,
        cookies,
        text: String::from_utf8(bytes.to_vec()).expect("a UTF-8 body"),
    }
}

// ── the shop ──────────────────────────────────────────────────────────────

struct Shop {
    desk: Desk,
    identity: FakeIdentity,
    state: AppState,
    org: String,
}

impl Shop {
    /// A fresh schema, a fake Identity that logs `SUB_A` in with a verified `PHONE`, and TERMS + MARKETING
    /// published in `th`.
    async fn start() -> Self {
        common::install_test_pii_keys();
        let desk = Desk::start().await;
        let identity = FakeIdentity::start();
        identity.login_answers("/auth/login", VALID_A, REFRESH_1);
        let state = state_with(&desk.pool, &identity_env(&identity.url()));
        let org = organization(&desk.pool).await;
        let shop = Self {
            desk,
            identity,
            state,
            org,
        };
        shop.publish(Purpose::Terms, "th").await;
        shop.publish(Purpose::Marketing, "th").await;
        shop
    }

    fn pool(&self) -> &PgPool {
        &self.desk.pool
    }

    async fn publish(&self, purpose: Purpose, lang: &str) -> i32 {
        ConsentService::pg(self.pool().clone())
            .publish(&self.org, None, purpose, lang, "ข้อความยินยอม", Utc::now())
            .await
            .expect("publish a consent text")
            .version
    }

    async fn send(&self, c: Call) -> Reply {
        send(&self.state, c).await
    }

    async fn register(&self, body: Value) -> Reply {
        self.send(call(Method::POST, "/api/app/register", Some(body)))
            .await
    }

    /// Register `SUB_A` (the fake's default login) and answer (customer id, session token).
    async fn registered(&self) -> (String, String) {
        let reply = self.register(register_body("th", &[])).await;
        assert_eq!(reply.status, 200, "{}", reply.text);
        let id = reply.json()["customerId"]
            .as_str()
            .unwrap_or_else(|| panic!("customerId: {}", reply.text))
            .to_owned();
        (id, reply.session())
    }

    /// Sign `subject` in through `/api/auth/login` (the fake answers `token` for it); the session token.
    async fn signed_in(&self, token: &str, refresh: &str) -> String {
        self.identity.login_answers("/auth/login", token, refresh);
        let reply = self
            .send(call(
                Method::POST,
                "/api/auth/login",
                Some(json!({ "identifier": "someone", "password": PASSWORD })),
            ))
            .await;
        assert_eq!(reply.status, 200, "sign-in: {}", reply.text);
        reply.session()
    }

    /// A customer of the organisation made the CI-6 way (a bound user with the CUSTOMER role, then
    /// `ConsentService::register`); (user id, customer id).
    async fn customer_of(&self, subject: &str, name: &str) -> (String, String) {
        let user = bound_user(self.pool(), subject, "ACTIVE", name, &[("CUSTOMER", None)]).await;
        let customer = ConsentService::pg(self.pool().clone())
            .register(
                &self.org,
                Registration {
                    user_id: user.clone(),
                    display_name: name.to_owned(),
                    lang: "th".to_owned(),
                    marketing: vec![],
                    evidence_ref: "test".to_owned(),
                },
                Utc::now(),
            )
            .await
            .expect("CI-6 register");
        (user, customer)
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(self.pool())
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// (users, CUSTOMER user_roles, customers, consents, user_sessions, PHONE identities)
    async fn written(&self) -> (i64, i64, i64, i64, i64, i64) {
        (
            self.count("SELECT count(*) FROM users").await,
            self.count("SELECT count(*) FROM user_roles WHERE role_code = 'CUSTOMER'")
                .await,
            self.count("SELECT count(*) FROM customers").await,
            self.count("SELECT count(*) FROM consents").await,
            self.count("SELECT count(*) FROM user_sessions").await,
            self.count("SELECT count(*) FROM customer_identities WHERE type = 'PHONE'")
                .await,
        )
    }

    async fn finish(self) {
        self.desk.drop_schema().await;
    }
}

fn register_body(lang: &str, marketing: &[&str]) -> Value {
    json!({
        "identifier": "somchai@example.com",
        "password": PASSWORD,
        "displayName": "Somchai J.",
        "lang": lang,
        "marketing": marketing,
    })
}

fn keys() -> Arc<PiiKeys> {
    xc_customer::get_pii_keys().expect("the test PII key is installed")
}

fn phone_hash(phone: &str) -> Vec<u8> {
    let e164 = normalize(IdentityType::Phone, phone, None).expect("a phone that normalises");
    LookupHasher::new(&keys())
        .hash(IdentityType::Phone, &e164)
        .to_vec()
}

/// What the fake's `/auth/login` answers for `SUB_A`, with the phone fields replaced.
fn login_with_phone(phone: Option<&str>, verified: Option<bool>) -> String {
    let mut body = login_body(VALID_A, REFRESH_1);
    let obj = body.as_object_mut().expect("an object");
    obj.remove("phone");
    obj.remove("phoneVerified");
    if let Some(p) = phone {
        obj.insert("phone".to_owned(), json!(p));
    }
    if let Some(v) = verified {
        obj.insert("phoneVerified".to_owned(), json!(v));
    }
    body.to_string()
}

// ── R: customerRegister ───────────────────────────────────────────────────

/// R1 (§9.1 CI-7, §4.4): one user (ACTIVE, `auth_subject`, the organisation), one org-wide CUSTOMER role, one
/// customer linked to it with the name and language, one TERMS consent on the newest `th` version, one session.
#[actix_web::test]
async fn a_verified_identity_user_registers_once_and_becomes_customer_with_terms_consent() {
    let shop = Shop::start().await;
    let newest = shop.publish(Purpose::Terms, "th").await;
    let reply = shop.register(register_body("th", &[])).await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    let customer = reply.json()["customerId"]
        .as_str()
        .expect("customerId")
        .to_owned();
    assert!(
        reply.json()["user"].is_object(),
        "the claim's {{user}}: {}",
        reply.text
    );
    let _ = reply.session();
    // OR-7.m w2 F-b (#1027 issuecomment-6305469252): the claim's sign-in no longer clears the v2
    // cookies; it sends no `__Host-xc_owner` / `__Host-xc_staff` header at all.
    for old in ["__Host-xc_owner", "__Host-xc_staff"] {
        assert_eq!(
            reply.cookies.get(old).map(|(_, a)| *a),
            None,
            "{old} is not sent"
        );
    }

    let user: (String, String, String) = sqlx::query_as(
        "SELECT id::text, organization_id::text, status FROM users WHERE auth_subject = $1",
    )
    .bind(SUB_A)
    .fetch_one(shop.pool())
    .await
    .expect("one user bound to the Identity subject");
    assert_eq!(
        (user.1.as_str(), user.2.as_str()),
        (shop.org.as_str(), "ACTIVE")
    );
    assert_eq!(
        shop.count(&format!(
            "SELECT count(*) FROM user_roles WHERE user_id = '{}' AND role_code = 'CUSTOMER' \
             AND branch_id IS NULL AND organization_id = '{}'",
            user.0, shop.org
        ))
        .await,
        1,
        "one organisation-wide CUSTOMER role"
    );
    let row: (String, String, String, String) = sqlx::query_as(
        "SELECT id, user_id::text, display_name, preferred_language FROM customers WHERE user_id = $1::uuid",
    )
    .bind(&user.0)
    .fetch_one(shop.pool())
    .await
    .expect("one customer linked to the user");
    assert_eq!(
        (row.0.as_str(), row.2.as_str(), row.3.as_str()),
        (customer.as_str(), "Somchai J.", "th")
    );
    let terms: (i32, bool, String) = sqlx::query_as(
        "SELECT policy_version, granted, lang FROM consents WHERE customer_id = $1 AND purpose = 'TERMS'",
    )
    .bind(&customer)
    .fetch_one(shop.pool())
    .await
    .expect("one TERMS consent");
    assert_eq!(
        terms,
        (newest, true, "th".to_owned()),
        "the newest th TERMS, granted"
    );
    assert_eq!(shop.written().await.4, 1, "one session");
    shop.finish().await;
}

/// R2 (D:§11 C14): each chosen marketing channel is one MARKETING consent row.
#[actix_web::test]
async fn registering_with_marketing_records_each_chosen_channel() {
    let shop = Shop::start().await;
    let reply = shop.register(register_body("th", &["SMS", "EMAIL"])).await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    let customer = reply.json()["customerId"]
        .as_str()
        .expect("customerId")
        .to_owned();
    let channels: Vec<String> = sqlx::query_scalar(
        "SELECT channel FROM consents WHERE customer_id = $1 AND purpose = 'MARKETING' AND granted ORDER BY channel",
    )
    .bind(&customer)
    .fetch_all(shop.pool())
    .await
    .expect("marketing rows");
    assert_eq!(channels, ["EMAIL", "SMS"]);
    shop.finish().await;
}

/// R3: the same Identity user registers twice → 409 `alreadyRegistered`, nothing new, no session.
#[actix_web::test]
async fn a_second_register_by_the_same_identity_user_is_refused_and_writes_nothing() {
    let shop = Shop::start().await;
    shop.registered().await;
    let before = shop.written().await;
    let reply = shop.register(register_body("th", &[])).await;
    assert_eq!(
        (reply.status, reply.refusal().0.as_str()),
        (409, "alreadyRegistered"),
        "{}",
        reply.text
    );
    assert!(reply.sets_no_session());
    assert_eq!(shop.written().await, before);
    shop.finish().await;
}

/// R4 [proposed, owner]: an Identity user already bound to staff cannot register as a customer.
#[actix_web::test]
async fn an_identity_user_bound_to_staff_cannot_register_as_a_customer() {
    let shop = Shop::start().await;
    bound_user(
        shop.pool(),
        SUB_A,
        "ACTIVE",
        "Nok",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    let before = shop.written().await;
    let reply = shop.register(register_body("th", &[])).await;
    assert_eq!(
        (reply.status, reply.refusal().0.as_str()),
        (409, "alreadyRegistered"),
        "{}",
        reply.text
    );
    assert!(reply.sets_no_session());
    assert_eq!(shop.written().await, before);
    shop.finish().await;
}

/// R5 (§4.3): no TERMS text in the request's language → 409 `consentTextMissing` (`field: lang`), and no user,
/// role, customer, consent, session or PHONE row.
#[actix_web::test]
async fn registering_without_terms_in_the_request_language_writes_no_user_and_no_customer() {
    let shop = Shop::start().await;
    let before = shop.written().await;
    let reply = shop.register(register_body("en", &[])).await;
    assert_eq!(reply.status, 409, "{}", reply.text);
    assert_eq!(
        reply.refusal(),
        ("consentTextMissing".to_owned(), Some("lang".to_owned()))
    );
    assert!(reply.sets_no_session());
    assert_eq!(shop.written().await, before, "all-or-nothing");
    shop.finish().await;
}

/// R6: marketing chosen, no MARKETING text in that language → 409 `consentTextMissing`, nothing written.
#[actix_web::test]
async fn registering_with_marketing_but_no_marketing_text_writes_nothing() {
    let shop = Shop::start().await;
    shop.publish(Purpose::Terms, "en").await;
    let before = shop.written().await;
    let reply = shop.register(register_body("en", &["SMS"])).await;
    assert_eq!(
        (reply.status, reply.refusal().0.as_str()),
        (409, "consentTextMissing"),
        "{}",
        reply.text
    );
    assert_eq!(shop.written().await, before);
    shop.finish().await;
}

/// R7: Identity refuses the credentials → 401 `invalidCredentials`, nothing written.
#[actix_web::test]
async fn wrong_identity_credentials_are_refused_and_write_nothing() {
    let shop = Shop::start().await;
    shop.identity
        .answer("/auth/login", 401, r#"{"error":"invalid_credentials"}"#);
    let before = shop.written().await;
    let reply = shop.register(register_body("th", &[])).await;
    assert_eq!(
        (reply.status, reply.refusal().0.as_str()),
        (401, "invalidCredentials"),
        "{}",
        reply.text
    );
    assert_eq!(shop.written().await, before);
    shop.finish().await;
}

/// R8 [proposed]: a token the verifier refuses (wrong audience, wrong signature, expired) → 503
/// `identityUnavailable`, as every BFF route answers it (`identity_signin_test.rs:197-249`); nothing written.
#[actix_web::test]
async fn a_token_the_verifier_refuses_is_503_and_writes_nothing() {
    let shop = Shop::start().await;
    for token in [WRONG_AUD_A, BAD_SIG_A, EXPIRED_A] {
        shop.identity.login_answers("/auth/login", token, REFRESH_1);
        let before = shop.written().await;
        let reply = shop.register(register_body("th", &[])).await;
        assert_eq!(
            (reply.status, reply.refusal().0.as_str()),
            (503, "identityUnavailable"),
            "{}",
            reply.text
        );
        assert_eq!(shop.written().await, before);
    }
    shop.finish().await;
}

/// R9: Identity unreachable → 503 `identityUnavailable`, nothing written.
#[actix_web::test]
async fn identity_unavailable_is_503_and_writes_nothing() {
    let shop = Shop::start().await;
    let state = state_with(
        shop.pool(),
        &identity_env(&format!("http://127.0.0.1:{}", closed_port())),
    );
    let before = shop.written().await;
    let reply = send(
        &state,
        call(
            Method::POST,
            "/api/app/register",
            Some(register_body("th", &[])),
        ),
    )
    .await;
    assert_eq!(
        (reply.status, reply.refusal().0.as_str()),
        (503, "identityUnavailable"),
        "{}",
        reply.text
    );
    assert_eq!(shop.written().await, before);
    shop.finish().await;
}

/// R10 [proposed]: a missing `identifier`/`password`/`displayName`, or a body that is not JSON → 400
/// `invalidRequest` before Identity is asked.
#[actix_web::test]
async fn a_register_missing_a_field_is_invalid_request_before_identity() {
    let shop = Shop::start().await;
    let before = shop.written().await;
    for drop in ["identifier", "password", "displayName"] {
        let mut body = register_body("th", &[]);
        body.as_object_mut().unwrap().remove(drop);
        let reply = shop.register(body).await;
        assert_eq!(
            (reply.status, reply.refusal().0.as_str()),
            (400, "invalidRequest"),
            "without {drop}: {}",
            reply.text
        );
    }
    let reply = shop
        .send(Call {
            body: None,
            ..call(Method::POST, "/api/app/register", None)
        })
        .await;
    assert_eq!(reply.status, 400, "an empty body: {}", reply.text);
    assert!(shop.identity.calls().is_empty(), "Identity is never asked");
    assert_eq!(shop.written().await, before, "nothing written");
    shop.finish().await;
}

/// R11 [proposed]: a language that is not th/en/zh, a blank display name, or an unknown marketing channel → 400
/// `validationFailed` naming the field, before Identity is asked.
#[actix_web::test]
async fn register_validation_failures_name_their_field() {
    let shop = Shop::start().await;
    let mut blank = register_body("th", &[]);
    blank["displayName"] = json!("   ");
    for (body, field) in [
        (register_body("fr", &[]), "lang"),
        (blank, "displayName"),
        (register_body("th", &["FAX"]), "marketing"),
    ] {
        let reply = shop.register(body).await;
        assert_eq!(reply.status, 400, "{field}: {}", reply.text);
        assert_eq!(
            reply.refusal(),
            ("validationFailed".to_owned(), Some(field.to_owned()))
        );
    }
    assert!(shop.identity.calls().is_empty(), "Identity is never asked");
    shop.finish().await;
}

/// R12: a cross-site register → 400 `invalidRequest`, before Identity is asked.
#[actix_web::test]
async fn a_cross_site_register_is_refused_before_identity() {
    let shop = Shop::start().await;
    let reply = shop
        .send(
            call(
                Method::POST,
                "/api/app/register",
                Some(register_body("th", &[])),
            )
            .cross_site(),
        )
        .await;
    assert_eq!(
        (reply.status, reply.refusal().0.as_str()),
        (400, "invalidRequest"),
        "{}",
        reply.text
    );
    assert!(shop.identity.calls().is_empty());
    shop.finish().await;
}

/// R13 [proposed]: two registers of one Identity user at once → one 200, one 409 `alreadyRegistered`; one user, one
/// customer.
#[actix_web::test]
async fn two_concurrent_registers_of_one_identity_make_one_customer() {
    let shop = Shop::start().await;
    let (users0, roles0, customers0, ..) = shop.written().await;
    let (a, b) = tokio::join!(
        send(
            &shop.state,
            call(
                Method::POST,
                "/api/app/register",
                Some(register_body("th", &[]))
            )
        ),
        send(
            &shop.state,
            call(
                Method::POST,
                "/api/app/register",
                Some(register_body("th", &[]))
            )
        ),
    );
    let mut statuses = [a.status, b.status];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 409], "{} / {}", a.text, b.text);
    let refused = if a.status == 409 { &a } else { &b };
    assert_eq!(refused.refusal().0, "alreadyRegistered");
    let (users, roles, customers, ..) = shop.written().await;
    assert_eq!(
        (users - users0, roles - roles0, customers - customers0),
        (1, 1, 1),
        "one new user, CUSTOMER role and customer"
    );
    shop.finish().await;
}

/// R14 (K8, K10): no answer echoes the password, the identifier, the Identity token or the phone; no stored user or
/// customer column holds them.
#[actix_web::test]
async fn register_never_logs_or_echoes_the_identifier_or_secret() {
    let shop = Shop::start().await;
    let ok = shop.register(register_body("th", &[])).await;
    assert_eq!(ok.status, 200, "{}", ok.text);
    let again = shop.register(register_body("th", &[])).await;
    let mut bad = register_body("th", &[]);
    bad["lang"] = json!("fr");
    let invalid = shop.register(bad).await;
    for reply in [&ok, &again, &invalid] {
        for secret in [
            PASSWORD,
            "somchai@example.com",
            VALID_A,
            REFRESH_1,
            PHONE,
            "812345678",
        ] {
            assert!(
                !reply.text.contains(secret),
                "the answer echoes {secret:?}: {}",
                reply.text
            );
        }
    }
    let stored: String = sqlx::query_scalar(
        "SELECT coalesce(string_agg(u::text, '|'), '') || coalesce((SELECT string_agg(c::text, '|') FROM customers c), '') \
         FROM users u",
    )
    .fetch_one(shop.pool())
    .await
    .expect("the stored rows as text");
    for secret in [PASSWORD, "somchai@example.com", PHONE, "812345678"] {
        assert!(
            !stored.contains(secret),
            "a users/customers column holds {secret:?}"
        );
    }
    shop.finish().await;
}

// ── H: the verified PHONE identity (07:42Z addition) ──────────────────────

/// (encrypted_value, key_id, lookup_hash, verified_at IS NOT NULL, status) of a PHONE identity.
type PhoneRow = (Vec<u8>, String, Vec<u8>, bool, String);

/// H1: Identity reports a verified phone → exactly one ACTIVE PHONE identity, `verified_at` set, encrypted with AAD
/// `customer_id ‖ "PHONE"` to the E.164 text, `lookup_hash` = `LookupHasher(PHONE, e164)`; no column holds the phone.
#[actix_web::test]
async fn registering_with_a_verified_phone_writes_one_active_encrypted_phone_identity() {
    let shop = Shop::start().await;
    let (customer, _) = shop.registered().await;
    let rows: Vec<PhoneRow> = sqlx::query_as(
        "SELECT encrypted_value, key_id, lookup_hash, verified_at IS NOT NULL, status \
         FROM customer_identities WHERE customer_id = $1 AND type = 'PHONE'",
    )
    .bind(&customer)
    .fetch_all(shop.pool())
    .await
    .expect("PHONE rows");
    assert_eq!(rows.len(), 1, "one PHONE identity");
    let (bytes, key_id, hash, verified, status) = rows.into_iter().next().unwrap();
    assert!(verified, "verified_at is set");
    assert_eq!(status, "ACTIVE");
    assert_eq!(
        hash,
        phone_hash(PHONE),
        "lookup_hash = LookupHasher(PHONE, E.164)"
    );
    let plain = PiiCipher::new(keys())
        .decrypt(
            format!("{customer}PHONE").as_bytes(),
            &Sealed {
                key_id,
                bytes: bytes.clone(),
            },
        )
        .expect("decrypts with AAD customer_id ‖ \"PHONE\"");
    assert_eq!(
        String::from_utf8(plain).unwrap(),
        PHONE,
        "the plaintext is the E.164 phone"
    );
    assert!(
        !bytes.windows(PHONE.len()).any(|w| w == PHONE.as_bytes()),
        "the stored value is not the phone in plain text"
    );
    shop.finish().await;
}

/// H2: no phone, or a phone Identity does not call verified → no PHONE identity (the register still succeeds).
#[actix_web::test]
async fn a_login_without_a_verified_phone_writes_no_phone_identity() {
    for (phone, verified) in [
        (None, None),
        (Some(PHONE), Some(false)),
        (Some(PHONE), None),
    ] {
        let shop = Shop::start().await;
        shop.identity
            .answer("/auth/login", 200, &login_with_phone(phone, verified));
        let reply = shop.register(register_body("th", &[])).await;
        assert_eq!(reply.status, 200, "{phone:?}/{verified:?}: {}", reply.text);
        assert_eq!(
            shop.written().await.5,
            0,
            "{phone:?}/{verified:?}: no PHONE row"
        );
        shop.finish().await;
    }
}

/// H3 [proposed]: the verified phone is already ACTIVE on another customer of the organisation → 409 `phoneInUse`,
/// and nothing is written (no user, role, customer, consent, session or PHONE row).
#[actix_web::test]
async fn a_phone_active_on_another_customer_is_refused_and_writes_nothing() {
    let shop = Shop::start().await;
    let (_, other) = shop.customer_of(SUB_B, "Other").await;
    let sealed =
        PiiCipher::new(keys()).encrypt(format!("{other}PHONE").as_bytes(), PHONE.as_bytes());
    sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, \
             lookup_hash, verified_at, status) \
         VALUES (gen_random_uuid(), $1::uuid, $2, 'PHONE', $3, $4, $5, now(), 'ACTIVE')",
    )
    .bind(&shop.org)
    .bind(&other)
    .bind(&sealed.bytes)
    .bind(&sealed.key_id)
    .bind(phone_hash(PHONE))
    .execute(shop.pool())
    .await
    .expect("the other customer's PHONE");
    let before = shop.written().await;
    let reply = shop.register(register_body("th", &[])).await;
    assert_eq!(
        (reply.status, reply.refusal().0.as_str()),
        (409, "phoneInUse"),
        "{}",
        reply.text
    );
    assert!(reply.sets_no_session());
    assert_eq!(shop.written().await, before, "all-or-nothing");
    shop.finish().await;
}

// ── S: the session's customer_id (ruling 5) ───────────────────────────────

fn sessions(shop: &Shop) -> SessionService {
    SessionService::new(shop.pool().clone(), None, None, None)
}

/// `customer_of_user`: the linked customer's id, or `None`.
#[actix_web::test]
async fn customer_of_user_answers_the_linked_customer_or_none() {
    let shop = Shop::start().await;
    let (user, customer) = shop.customer_of(SUB_A, "Somchai").await;
    let staff = bound_user(
        shop.pool(),
        SUB_B,
        "ACTIVE",
        "Nok",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    assert_eq!(
        customer_of_user(shop.pool(), &user).await.unwrap(),
        Some(customer)
    );
    assert_eq!(customer_of_user(shop.pool(), &staff).await.unwrap(), None);
    shop.finish().await;
}

/// S1 (K7, §9.1): a CUSTOMER user's session carries `Actor.customer_id = Some(customers.id)`.
#[actix_web::test]
async fn a_customer_session_carries_its_customer_id() {
    let shop = Shop::start().await;
    let (user, customer) = shop.customer_of(SUB_A, "Somchai").await;
    let token = shop.signed_in(VALID_A, REFRESH_1).await;
    let actor = resolve_session_actor(&sessions(&shop), shop.pool(), &token, Utc::now())
        .await
        .expect("a live session");
    assert_eq!(actor.user_id, user);
    assert_eq!(actor.customer_id, Some(customer));
    shop.finish().await;
}

/// S2 (§9.1): staff, the owner and a MARKETING-only user carry no `customer_id`; nor does a CUSTOMER role with no
/// linked customer (S3); an unknown token is no actor.
#[actix_web::test]
async fn staff_owner_and_marketing_sessions_carry_no_customer_id() {
    for roles in [
        &[("TELLER", Some("MAIN"))][..],
        &[("EXECUTIVE", None), ("ADMIN", None)][..],
        &[("MARKETING", None)][..],
        &[("CUSTOMER", None)][..],
    ] {
        let shop = Shop::start().await;
        bound_user(shop.pool(), SUB_A, "ACTIVE", "Someone", roles).await;
        let token = shop.signed_in(VALID_A, REFRESH_1).await;
        let actor = resolve_session_actor(&sessions(&shop), shop.pool(), &token, Utc::now())
            .await
            .unwrap_or_else(|| panic!("{roles:?}: a live session"));
        assert_eq!(actor.customer_id, None, "{roles:?}");
        assert!(
            resolve_session_actor(&sessions(&shop), shop.pool(), "not-a-session", Utc::now())
                .await
                .is_none()
        );
        shop.finish().await;
    }
}

// ── C: consentRecord (red until #767) ─────────────────────────────────────

fn consent(body: Value) -> Call {
    call(Method::POST, "/api/app/consents", Some(body))
}

async fn marketing_allowed(shop: &Shop, customer: &str, channel: Channel) -> bool {
    ConsentService::pg(shop.pool().clone())
        .marketing_allowed(customer, channel)
        .await
        .expect("marketing_allowed")
}

/// C1 (K5): withdrawing SMS stops SMS; EMAIL is unchanged. One row, by the customer's own user.
#[actix_web::test]
async fn a_customer_withdraws_sms_marketing_and_email_stays() {
    let shop = Shop::start().await;
    let reply = shop.register(register_body("th", &["SMS", "EMAIL"])).await;
    let customer = reply.json()["customerId"]
        .as_str()
        .expect("customerId")
        .to_owned();
    let token = reply.session();
    let before = shop.count("SELECT count(*) FROM consents").await;
    let r = shop
        .send(
            consent(
                json!({ "purpose": "MARKETING", "channel": "SMS", "granted": false, "lang": "th" }),
            )
            .by(&token)
            .key("c1-key"),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(r.json()["granted"], json!(false), "{}", r.text);
    assert_eq!(
        shop.count("SELECT count(*) FROM consents").await,
        before + 1
    );
    assert!(!marketing_allowed(&shop, &customer, Channel::Sms).await);
    assert!(
        marketing_allowed(&shop, &customer, Channel::Email).await,
        "EMAIL stays"
    );
    let by: Option<String> = sqlx::query_scalar(
        "SELECT recorded_by::text FROM consents WHERE customer_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&customer)
    .fetch_one(shop.pool())
    .await
    .unwrap();
    let own: String = sqlx::query_scalar("SELECT user_id::text FROM customers WHERE id = $1")
        .bind(&customer)
        .fetch_one(shop.pool())
        .await
        .unwrap();
    assert_eq!(
        by.as_deref(),
        Some(own.as_str()),
        "recorded by the customer's own user"
    );
    shop.finish().await;
}

/// C2: granting EMAIL makes EMAIL eligible.
#[actix_web::test]
async fn a_customer_grants_email_marketing() {
    let shop = Shop::start().await;
    let (customer, token) = shop.registered().await;
    let r = shop
        .send(
            consent(json!({ "purpose": "MARKETING", "channel": "EMAIL", "granted": true, "lang": "th" }))
                .by(&token)
                .key("c2-key"),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert!(marketing_allowed(&shop, &customer, Channel::Email).await);
    shop.finish().await;
}

/// C3 (§4.3): no text of that purpose in the language → 409 `consentTextMissing` (`field: lang`), no row.
#[actix_web::test]
async fn recording_in_a_language_with_no_text_is_refused_and_writes_nothing() {
    let shop = Shop::start().await;
    let (_, token) = shop.registered().await;
    let before = shop.count("SELECT count(*) FROM consents").await;
    let r = shop
        .send(
            consent(
                json!({ "purpose": "MARKETING", "channel": "SMS", "granted": true, "lang": "zh" }),
            )
            .by(&token)
            .key("c3-key"),
        )
        .await;
    assert_eq!(r.status, 409, "{}", r.text);
    assert_eq!(
        r.refusal(),
        ("consentTextMissing".to_owned(), Some("lang".to_owned()))
    );
    assert_eq!(shop.count("SELECT count(*) FROM consents").await, before);
    shop.finish().await;
}

/// C4, C5, C6 [proposed]: an unknown language, MARKETING without a channel, TERMS with a channel, and withdrawing
/// TERMS (owner) → 400 `validationFailed` naming the field; no row.
#[actix_web::test]
async fn consent_validation_failures_name_their_field() {
    let shop = Shop::start().await;
    let (_, token) = shop.registered().await;
    let before = shop.count("SELECT count(*) FROM consents").await;
    for (n, body, field) in [
        (
            1,
            json!({ "purpose": "MARKETING", "channel": "SMS", "granted": true, "lang": "fr" }),
            "lang",
        ),
        (
            2,
            json!({ "purpose": "MARKETING", "granted": true, "lang": "th" }),
            "channel",
        ),
        (
            3,
            json!({ "purpose": "TERMS", "channel": "SMS", "granted": true, "lang": "th" }),
            "channel",
        ),
        (
            4,
            json!({ "purpose": "TERMS", "granted": false, "lang": "th" }),
            "granted",
        ),
    ] {
        let r = shop
            .send(consent(body).by(&token).key(&format!("c4-key-{n}")))
            .await;
        assert_eq!(r.status, 400, "{field}: {}", r.text);
        assert_eq!(
            r.refusal(),
            ("validationFailed".to_owned(), Some(field.to_owned()))
        );
    }
    assert_eq!(shop.count("SELECT count(*) FROM consents").await, before);
    shop.finish().await;
}

/// C7, C8: the same key and body answers the same, once; the same key with another body → 409
/// `idempotencyConflict`.
#[actix_web::test]
async fn repeating_a_consent_with_the_same_key_records_it_once() {
    let shop = Shop::start().await;
    let (_, token) = shop.registered().await;
    let body = json!({ "purpose": "MARKETING", "channel": "PUSH", "granted": true, "lang": "th" });
    let before = shop.count("SELECT count(*) FROM consents").await;
    let first = shop
        .send(consent(body.clone()).by(&token).key("c7-key"))
        .await;
    let second = shop.send(consent(body).by(&token).key("c7-key")).await;
    assert_eq!(first.status, 200, "{}", first.text);
    assert_eq!(
        (second.status, second.json()),
        (200, first.json()),
        "a replay"
    );
    assert_eq!(
        shop.count("SELECT count(*) FROM consents").await,
        before + 1,
        "recorded once"
    );
    let other = shop
        .send(
            consent(json!({ "purpose": "MARKETING", "channel": "PUSH", "granted": false, "lang": "th" }))
                .by(&token)
                .key("c7-key"),
        )
        .await;
    assert_eq!(
        (other.status, other.refusal().0.as_str()),
        (409, "idempotencyConflict"),
        "{}",
        other.text
    );
    shop.finish().await;
}

/// Every CI-7 route behind a session (C9-C11, P5, V4-V5).
fn customer_routes() -> Vec<Call> {
    vec![
        consent(json!({ "purpose": "MARKETING", "channel": "SMS", "granted": true, "lang": "th" })),
        call(
            Method::PUT,
            "/api/app/profile",
            Some(json!({ "displayName": "X" })),
        ),
        call(Method::GET, "/api/v3/views/my_profile", None),
    ]
}

/// C9: no session → 401 `signIn` on every CI-7 route.
#[actix_web::test]
async fn no_session_is_sign_in() {
    let shop = Shop::start().await;
    for (n, c) in customer_routes().into_iter().enumerate() {
        let path = c.path.clone();
        let r = shop.send(c.key(&format!("c9-{n}"))).await;
        assert_eq!(
            (r.status, r.refusal().0.as_str()),
            (401, "signIn"),
            "{path}: {}",
            r.text
        );
    }
    shop.finish().await;
}

/// C10 [proposed]: a staff or owner session → 403 `roleNotAllowedHere` on every CI-7 route; nothing written.
#[actix_web::test]
async fn a_staff_or_owner_session_cannot_use_the_customer_routes() {
    for roles in [
        &[("TELLER", Some("MAIN"))][..],
        &[("EXECUTIVE", None), ("ADMIN", None)][..],
    ] {
        let shop = Shop::start().await;
        bound_user(shop.pool(), SUB_A, "ACTIVE", "Staff", roles).await;
        let token = shop.signed_in(VALID_A, REFRESH_1).await;
        for (n, c) in customer_routes().into_iter().enumerate() {
            let path = c.path.clone();
            let r = shop.send(c.by(&token).key(&format!("c10-{n}"))).await;
            assert_eq!(
                (r.status, r.refusal().0.as_str()),
                (400, "roleNotAllowedHere"),
                "{roles:?} {path}: {}",
                r.text
            );
        }
        assert_eq!(shop.count("SELECT count(*) FROM consents").await, 0);
        shop.finish().await;
    }
}

/// C11: a CUSTOMER role with no linked customer → 403 `outOfScope` on every CI-7 route.
#[actix_web::test]
async fn a_customer_role_without_a_customer_is_out_of_scope() {
    let shop = Shop::start().await;
    bound_user(
        shop.pool(),
        SUB_A,
        "ACTIVE",
        "Lonely",
        &[("CUSTOMER", None)],
    )
    .await;
    let token = shop.signed_in(VALID_A, REFRESH_1).await;
    for (n, c) in customer_routes().into_iter().enumerate() {
        let path = c.path.clone();
        let r = shop.send(c.by(&token).key(&format!("c11-{n}"))).await;
        assert_eq!(
            (r.status, r.refusal().0.as_str()),
            (403, "outOfScope"),
            "{path}: {}",
            r.text
        );
    }
    shop.finish().await;
}

// ── P: profileUpdate (red until #767) ─────────────────────────────────────

fn profile(body: Value) -> Call {
    call(Method::PUT, "/api/app/profile", Some(body))
}

/// P1 (§4.4): the customer changes their own name and language; nothing else changes, no consent row.
#[actix_web::test]
async fn a_customer_changes_their_own_name_and_language() {
    let shop = Shop::start().await;
    shop.publish(Purpose::Terms, "en").await;
    let (customer, token) = shop.registered().await;
    let consents = shop.count("SELECT count(*) FROM consents").await;
    let r = shop
        .send(
            profile(json!({ "displayName": "Somchai Jaidee", "preferredLanguage": "en" }))
                .by(&token)
                .key("p1-key"),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(
        r.json(),
        json!({ "customerId": customer, "displayName": "Somchai Jaidee", "preferredLanguage": "en" })
    );
    let row: (String, String, bool) = sqlx::query_as(
        "SELECT display_name, preferred_language, updated_at IS NOT NULL FROM customers WHERE id = $1",
    )
    .bind(&customer)
    .fetch_one(shop.pool())
    .await
    .unwrap();
    assert_eq!(row, ("Somchai Jaidee".to_owned(), "en".to_owned(), true));
    assert_eq!(shop.count("SELECT count(*) FROM consents").await, consents);
    shop.finish().await;
}

/// P2 [proposed]: a blank name, an unknown language, or neither field → 400 `validationFailed` naming the field.
#[actix_web::test]
async fn profile_validation_failures_name_their_field() {
    let shop = Shop::start().await;
    let (customer, token) = shop.registered().await;
    for (n, body, field) in [
        (1, json!({ "displayName": "  " }), "displayName"),
        (2, json!({ "preferredLanguage": "fr" }), "preferredLanguage"),
    ] {
        let r = shop
            .send(profile(body).by(&token).key(&format!("p2-{n}")))
            .await;
        assert_eq!(r.status, 400, "{field}: {}", r.text);
        assert_eq!(
            r.refusal(),
            ("validationFailed".to_owned(), Some(field.to_owned()))
        );
    }
    let r = shop.send(profile(json!({})).by(&token).key("p2-3")).await;
    assert_eq!(
        (r.status, r.refusal().0.as_str()),
        (400, "validationFailed"),
        "neither field: {}",
        r.text
    );
    let name: String = sqlx::query_scalar("SELECT display_name FROM customers WHERE id = $1")
        .bind(&customer)
        .fetch_one(shop.pool())
        .await
        .unwrap();
    assert_eq!(name, "Somchai J.", "unchanged");
    shop.finish().await;
}

/// P3 [proposed]: a language with no TERMS text → 409 `consentTextMissing` (`field: preferredLanguage`), unchanged.
#[actix_web::test]
async fn a_language_without_terms_text_cannot_be_chosen() {
    let shop = Shop::start().await;
    let (customer, token) = shop.registered().await;
    let r = shop
        .send(
            profile(json!({ "preferredLanguage": "zh" }))
                .by(&token)
                .key("p3-key"),
        )
        .await;
    assert_eq!(r.status, 409, "{}", r.text);
    assert_eq!(
        r.refusal(),
        (
            "consentTextMissing".to_owned(),
            Some("preferredLanguage".to_owned())
        )
    );
    let lang: String = sqlx::query_scalar("SELECT preferred_language FROM customers WHERE id = $1")
        .bind(&customer)
        .fetch_one(shop.pool())
        .await
        .unwrap();
    assert_eq!(lang, "th");
    shop.finish().await;
}

/// P4 [proposed]: a body that names a `customerId` → 400 `invalidRequest`; no customer changes.
#[actix_web::test]
async fn a_profile_update_naming_a_customer_id_is_refused() {
    let shop = Shop::start().await;
    let (_, other) = shop.customer_of(SUB_B, "Other").await;
    let (_, token) = shop.registered().await;
    let r = shop
        .send(
            profile(json!({ "customerId": other, "displayName": "Hijacked" }))
                .by(&token)
                .key("p4-key"),
        )
        .await;
    assert_eq!(
        (r.status, r.refusal().0.as_str()),
        (400, "invalidRequest"),
        "{}",
        r.text
    );
    assert_eq!(
        shop.count("SELECT count(*) FROM customers WHERE display_name = 'Hijacked'")
            .await,
        0
    );
    shop.finish().await;
}

// ── V: my_profile (red until #767) ────────────────────────────────────────

/// V1 (§4.4, K8): the customer reads their own profile and current consents; no identity value, hash or last 4.
#[actix_web::test]
async fn a_customer_reads_their_own_profile_and_current_consents() {
    let shop = Shop::start().await;
    let reply = shop.register(register_body("th", &["SMS"])).await;
    let customer = reply.json()["customerId"]
        .as_str()
        .expect("customerId")
        .to_owned();
    let token = reply.session();
    let r = shop
        .send(call(Method::GET, "/api/v3/views/my_profile", None).by(&token))
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let data = &r.json()["data"];
    assert_eq!(data["customerId"], json!(customer));
    assert_eq!(data["displayName"], json!("Somchai J."));
    assert_eq!(data["preferredLanguage"], json!("th"));
    assert_eq!(data["status"], json!("ACTIVE"));
    assert!(data["customerNo"].is_string(), "{}", r.text);
    let consents = data["consents"]
        .as_array()
        .unwrap_or_else(|| panic!("consents: {}", r.text));
    let pairs: Vec<(String, Option<String>, bool)> = consents
        .iter()
        .map(|c| {
            (
                c["purpose"].as_str().unwrap_or("").to_owned(),
                c["channel"].as_str().map(str::to_owned),
                c["granted"].as_bool().unwrap_or(false),
            )
        })
        .collect();
    assert_eq!(
        pairs,
        [
            ("MARKETING".to_owned(), Some("SMS".to_owned()), true),
            ("TERMS".to_owned(), None, true),
        ],
        "the newest row per (purpose, channel), by purpose then channel"
    );
    for secret in [PHONE, "812345678", "5678"] {
        assert!(!r.text.contains(secret), "the profile shows {secret:?}");
    }
    assert!(!r.text.contains("lookup"), "no lookup hash");
    shop.finish().await;
}

/// V2, V3 (§9.1): `?customer=` of another customer → 403 `outOfScope`; of one's own → 200.
#[actix_web::test]
async fn a_customer_reading_another_customers_profile_is_out_of_scope() {
    let shop = Shop::start().await;
    let (_, other) = shop.customer_of(SUB_B, "Other").await;
    let (own, token) = shop.registered().await;
    let r = shop
        .send(
            call(
                Method::GET,
                &format!("/api/v3/views/my_profile?customer={other}"),
                None,
            )
            .by(&token),
        )
        .await;
    assert_eq!(
        (r.status, r.refusal().0.as_str()),
        (403, "outOfScope"),
        "{}",
        r.text
    );
    let r = shop
        .send(
            call(
                Method::GET,
                &format!("/api/v3/views/my_profile?customer={own}"),
                None,
            )
            .by(&token),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(r.json()["data"]["customerId"], json!(own));
    shop.finish().await;
}
