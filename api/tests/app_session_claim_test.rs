//! Ticket #998 (APP-7): Customer sign-in plus app shell — up-front tests for app session claim.
//! (design `docs/design/blueprint/customer-app.md` §9.1 row APP-7; FE lane controller ruling on #998).
//!
//! Written before the implementation by test-writer gemini/warut-6 on claude/5edf7bb5's order.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! What these tests pin (HTTP, black-box):
//! 1. An anonymous plannerDraftSave and an assistantAsk with one xc_app_sid, then OTP sign-in
//!    of an existing customer on that cookie, gives my_planner_drafts and my_assistant_conversation
//!    both showing them.
//! 2. The same flow for a new customer, with the claim at customerRegister (POST /api/app/register).
//! 3. A second sign-in on the same cookie moves none and duplicates nothing.
//! 4. A sign-in without the cookie claims nothing and does not fail.
//!
//! Red on `main`: The session claim on sign-in / registration is not yet wired in auth.rs or
//! customer_register.rs, so the customer's views do not show the anonymous session's drafts/conversations.
//!
//! Needs TEST_DATABASE_URL naming an existing PostgreSQL database ending in `_test` (never port 6381).

mod auth_http_common;
mod common;
mod identity_signin_common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::test;
use auth_http_common::Desk;
use chrono::Utc;
use identity_signin_common::{
    FakeIdentity, REFRESH_1, SESSION_COOKIE, SUB_A, VALID_A, VALID_B, bound_user, identity_env,
    login_body, organization, state_with,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;
use xc_app::domain::app_session::{APP_SESSION_COOKIE, AppSessionId};
use xc_customer::application::consent_service::ConsentService;
use xc_customer::domain::consent::Purpose;
use xchange::bootstrap::AppState;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

async fn publish_usd(pool: &PgPool) {
    let approval: Uuid = sqlx::query_scalar(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
         proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES (gen_random_uuid(), 'RATE_PUBLISH', $1, $2, 'h', 'p', '{}', 'test', now(), 'APPROVED', $2) \
         RETURNING id",
    )
    .bind("MAIN")
    .bind(unique("pub"))
    .fetch_one(pool)
    .await
    .expect("approved request for publication");

    let publication: Uuid = sqlx::query_scalar(
        "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
         VALUES (gen_random_uuid(), $1, 1, now(), $2, 'test') RETURNING id",
    )
    .bind("MAIN")
    .bind(approval)
    .fetch_one(pool)
    .await
    .expect("rate publication");

    sqlx::query(
        "INSERT INTO published_rate_lines (id, publication_id, currency_code, unit_size, buy_rate, sell_rate, source_snapshot) \
         VALUES (gen_random_uuid(), $1, 'USD', 1, 33.50, 34.00, $2)",
    )
    .bind(publication)
    .bind(json!({ "buy_mode": "AUTO", "sell_mode": "AUTO" }))
    .execute(pool)
    .await
    .expect("published USD line");

    sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
        .bind(publication)
        .bind("MAIN")
        .execute(pool)
        .await
        .expect("make publication current");
}

async fn publish_consent(pool: &PgPool, org: &str) {
    ConsentService::pg(pool.clone())
        .publish(
            org,
            None,
            Purpose::Terms,
            "th",
            "ข้อความยินยอม TERMS",
            Utc::now(),
        )
        .await
        .expect("publish terms");
    ConsentService::pg(pool.clone())
        .publish(
            org,
            None,
            Purpose::Marketing,
            "th",
            "ข้อความยินยอม MARKETING",
            Utc::now(),
        )
        .await
        .expect("publish marketing");
}

#[derive(Debug, Clone)]
struct Call {
    method: Method,
    path: String,
    body: Option<Value>,
    session: Option<String>,
    app_sid: Option<String>,
    key: Option<String>,
}

impl Call {
    fn new(method: Method, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            body: None,
            session: None,
            app_sid: None,
            key: None,
        }
    }

    fn body(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }

    fn session(mut self, token: impl Into<String>) -> Self {
        self.session = Some(token.into());
        self
    }

    fn app_sid(mut self, sid: impl Into<String>) -> Self {
        self.app_sid = Some(sid.into());
        self
    }

    fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }
}

struct Reply {
    status: StatusCode,
    cookies: HashMap<String, String>,
    body: Value,
    text: String,
}

impl Reply {
    fn session_token(&self) -> String {
        self.cookies
            .get(SESSION_COOKIE)
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "expected {SESSION_COOKIE} in Set-Cookie: {:?}",
                    self.cookies
                )
            })
    }
}

struct Shop {
    desk: Desk,
    identity: FakeIdentity,
    state: AppState,
    org: String,
}

impl Shop {
    async fn start() -> Self {
        common::install_test_pii_keys();
        let desk = Desk::start().await;
        let identity = FakeIdentity::start();
        let state = state_with(&desk.pool, &identity_env(&identity.url()));
        let org = organization(&desk.pool).await;

        publish_usd(&desk.pool).await;
        publish_consent(&desk.pool, &org).await;

        Self {
            desk,
            identity,
            state,
            org,
        }
    }

    async fn send(&self, c: Call) -> Reply {
        let state = self.state.clone();
        let app = test::init_service(
            actix_web::App::new().configure(move |cfg| xchange::bootstrap::configure(cfg, state)),
        )
        .await;

        let mut req = test::TestRequest::default()
            .method(c.method)
            .uri(&c.path)
            .insert_header((header::HOST, "desk.example"))
            .insert_header(("x-forwarded-proto", "https"))
            .insert_header(("x-real-ip", "203.0.113.7"))
            .insert_header((header::ORIGIN, "https://desk.example"))
            .insert_header(("x-xchange-request", "1"));

        let mut cookies = Vec::new();
        if let Some(s) = &c.session {
            cookies.push(format!("{SESSION_COOKIE}={s}"));
        }
        if let Some(sid) = &c.app_sid {
            cookies.push(format!("{APP_SESSION_COOKIE}={sid}"));
        }
        if !cookies.is_empty() {
            req = req.insert_header((header::COOKIE, cookies.join("; ")));
        }
        if let Some(k) = &c.key {
            req = req.insert_header(("idempotency-key", k.as_str()));
        }
        if let Some(b) = &c.body {
            req = req
                .insert_header((header::CONTENT_TYPE, "application/json"))
                .set_payload(b.to_string());
        }

        let response = test::call_service(&app, req.to_request()).await;
        let status = response.status();
        let mut resp_cookies = HashMap::new();
        for line in response.headers().get_all(header::SET_COOKIE) {
            if let Ok(line_str) = line.to_str() {
                let (pair, _) = line_str.split_once("; ").unwrap_or((line_str, ""));
                if let Some((name, val)) = pair.split_once('=') {
                    resp_cookies.insert(name.to_owned(), val.to_owned());
                }
            }
        }
        let bytes = test::read_body(response).await;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

        Reply {
            status,
            cookies: resp_cookies,
            body,
            text,
        }
    }

    async fn finish(self) {
        self.desk.drop_schema().await;
    }
}

// ------------------------------------------------------------ UP-FRONT TESTS

/// 1. An anonymous plannerDraftSave and an assistantAsk with one xc_app_sid, then OTP sign-in
/// of an existing customer on that cookie, gives my_planner_drafts and my_assistant_conversation
/// both showing them.
#[actix_web::test]
async fn otp_signin_claims_anonymous_drafts_and_assistant_conversations() {
    let shop = Shop::start().await;
    let sid = AppSessionId::generate().as_str().to_owned();

    // 1. Anonymous planner draft save with xc_app_sid
    let draft_body = json!({
        "branch": "MAIN",
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "500.0000" },
        "expectedVersion": null,
    });
    let draft_res = shop
        .send(
            Call::new(Method::POST, "/api/app/planner-drafts")
                .body(draft_body)
                .app_sid(&sid)
                .key(unique("draft-key-1")),
        )
        .await;
    assert_eq!(
        draft_res.status,
        StatusCode::OK,
        "anonymous draft save succeeded: {}",
        draft_res.text
    );
    let draft_id = draft_res.body["id"].as_str().expect("draft id").to_owned();

    // 2. Anonymous assistant ask with the same xc_app_sid
    let ask_body = json!({
        "branch": "MAIN",
        "text": "อัตราแลกเปลี่ยน USD วันนี้เท่าไหร่",
        "locale": "th",
    });
    let ask_res = shop
        .send(
            Call::new(Method::POST, "/api/app/assistant/messages")
                .body(ask_body)
                .app_sid(&sid)
                .key(unique("ask-key-1")),
        )
        .await;
    assert_eq!(
        ask_res.status,
        StatusCode::OK,
        "anonymous assistant ask succeeded: {}",
        ask_res.text
    );
    let conversation_id = ask_res.body["conversationId"]
        .as_str()
        .expect("conversationId")
        .to_owned();

    // 3. Existing customer user in database, linked to a customer record
    let user_id = bound_user(
        &shop.desk.pool,
        SUB_A,
        "ACTIVE",
        "Existing Customer",
        &[("CUSTOMER", None)],
    )
    .await;
    let customer_id = format!("CUST-{user_id}");
    sqlx::query(
        "INSERT INTO customers (id, organization_id, user_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2::uuid, $3::uuid, $1, 'Existing Customer', 'LIVE', now()) ON CONFLICT (id) DO NOTHING",
    )
    .bind(&customer_id)
    .bind(&shop.org)
    .bind(Uuid::parse_str(&user_id).unwrap())
    .execute(&shop.desk.pool)
    .await
    .expect("insert customer");

    // FakeIdentity answers OTP verify with VALID_A (which has sub SUB_A)
    shop.identity.answer(
        "/auth/otp/verify",
        200,
        &json!({ "verified": true, "login": login_body(VALID_A, REFRESH_1) }).to_string(),
    );

    // OTP sign-in carrying xc_app_sid
    let otp_res = shop
        .send(
            Call::new(Method::POST, "/api/auth/otp/verify")
                .body(json!({
                    "challengeId": "challenge-otp-1",
                    "code": "123456",
                }))
                .app_sid(&sid),
        )
        .await;
    assert_eq!(
        otp_res.status,
        StatusCode::OK,
        "otp sign-in succeeded: {}",
        otp_res.text
    );
    let cust_session = otp_res.session_token();

    // 4. View my_planner_drafts as the signed-in customer (with __Host-xc_session)
    let drafts_res = shop
        .send(Call::new(Method::GET, "/api/v3/views/my_planner_drafts").session(&cust_session))
        .await;
    assert_eq!(
        drafts_res.status,
        StatusCode::OK,
        "my_planner_drafts succeeded: {}",
        drafts_res.text
    );
    let drafts = drafts_res.body["data"]["drafts"]
        .as_array()
        .expect("drafts array in view");
    assert_eq!(
        drafts.len(),
        1,
        "customer owns the claimed draft (currently fails on main: drafts not claimed)"
    );
    assert_eq!(drafts[0]["id"].as_str(), Some(draft_id.as_str()));

    // 5. View my_assistant_conversation as the signed-in customer
    let conv_res = shop
        .send(
            Call::new(
                Method::GET,
                format!("/api/v3/views/my_assistant_conversation?conversationId={conversation_id}"),
            )
            .session(&cust_session),
        )
        .await;
    assert_eq!(
        conv_res.status,
        StatusCode::OK,
        "my_assistant_conversation succeeded: {}",
        conv_res.text
    );
    let conv = &conv_res.body["data"]["conversation"];
    assert!(
        !conv.is_null(),
        "customer owns the claimed conversation (currently fails on main: conversation not claimed)"
    );
    assert_eq!(conv["id"].as_str(), Some(conversation_id.as_str()));

    shop.finish().await;
}

/// 2. The same flow for a new customer, with the claim at customerRegister (POST /api/app/register).
#[actix_web::test]
async fn new_customer_register_claims_anonymous_drafts_and_assistant_conversations() {
    let shop = Shop::start().await;
    let sid = AppSessionId::generate().as_str().to_owned();

    // 1. Anonymous draft save with xc_app_sid
    let draft_res = shop
        .send(
            Call::new(Method::POST, "/api/app/planner-drafts")
                .body(json!({
                    "branch": "MAIN",
                    "currency": "USD",
                    "customerSide": "CUSTOMER_BUY",
                    "amount": { "quantity": "300.0000" },
                    "expectedVersion": null,
                }))
                .app_sid(&sid)
                .key(unique("draft-key-2")),
        )
        .await;
    assert_eq!(
        draft_res.status,
        StatusCode::OK,
        "draft save: {}",
        draft_res.text
    );
    let draft_id = draft_res.body["id"].as_str().expect("draft id").to_owned();

    // 2. Anonymous assistant ask with xc_app_sid
    let ask_res = shop
        .send(
            Call::new(Method::POST, "/api/app/assistant/messages")
                .body(json!({
                    "branch": "MAIN",
                    "text": "เรทเงิน USD วันนี้",
                    "locale": "th",
                }))
                .app_sid(&sid)
                .key(unique("ask-key-2")),
        )
        .await;
    assert_eq!(
        ask_res.status,
        StatusCode::OK,
        "assistant ask: {}",
        ask_res.text
    );
    let conversation_id = ask_res.body["conversationId"]
        .as_str()
        .expect("conversationId")
        .to_owned();

    // 3. New customer registers via customerRegister (POST /api/app/register) carrying xc_app_sid
    shop.identity
        .login_answers("/auth/login", VALID_B, REFRESH_1);

    let reg_res = shop
        .send(
            Call::new(Method::POST, "/api/app/register")
                .body(json!({
                    "identifier": "newcustomer@example.com",
                    "password": "Password123!",
                    "displayName": "New Customer",
                    "lang": "th",
                    "marketing": [],
                }))
                .app_sid(&sid),
        )
        .await;
    assert_eq!(
        reg_res.status,
        StatusCode::OK,
        "customer register succeeded: {}",
        reg_res.text
    );
    let cust_session = reg_res.session_token();

    // 4. View my_planner_drafts as the newly registered customer
    let drafts_res = shop
        .send(Call::new(Method::GET, "/api/v3/views/my_planner_drafts").session(&cust_session))
        .await;
    assert_eq!(
        drafts_res.status,
        StatusCode::OK,
        "my_planner_drafts: {}",
        drafts_res.text
    );
    let drafts = drafts_res.body["data"]["drafts"]
        .as_array()
        .expect("drafts array in view");
    assert_eq!(
        drafts.len(),
        1,
        "new customer owns the claimed draft (currently fails on main: drafts not claimed)"
    );
    assert_eq!(drafts[0]["id"].as_str(), Some(draft_id.as_str()));

    // 5. View my_assistant_conversation as the newly registered customer
    let conv_res = shop
        .send(
            Call::new(
                Method::GET,
                format!("/api/v3/views/my_assistant_conversation?conversationId={conversation_id}"),
            )
            .session(&cust_session),
        )
        .await;
    assert_eq!(
        conv_res.status,
        StatusCode::OK,
        "my_assistant_conversation: {}",
        conv_res.text
    );
    let conv = &conv_res.body["data"]["conversation"];
    assert!(
        !conv.is_null(),
        "new customer owns the claimed conversation (currently fails on main: conversation not claimed)"
    );
    assert_eq!(conv["id"].as_str(), Some(conversation_id.as_str()));

    shop.finish().await;
}

/// 3. A second sign-in on the same cookie moves none and duplicates nothing.
#[actix_web::test]
async fn second_signin_on_same_cookie_moves_none_and_duplicates_nothing() {
    let shop = Shop::start().await;
    let sid = AppSessionId::generate().as_str().to_owned();

    // Anonymous draft
    let draft_res = shop
        .send(
            Call::new(Method::POST, "/api/app/planner-drafts")
                .body(json!({
                    "branch": "MAIN",
                    "currency": "USD",
                    "customerSide": "CUSTOMER_BUY",
                    "amount": { "quantity": "100.0000" },
                    "expectedVersion": null,
                }))
                .app_sid(&sid)
                .key(unique("draft-key-3")),
        )
        .await;
    assert_eq!(draft_res.status, StatusCode::OK);

    // Setup customer
    let user_1 = bound_user(
        &shop.desk.pool,
        SUB_A,
        "ACTIVE",
        "Customer 1",
        &[("CUSTOMER", None)],
    )
    .await;
    let cust_1 = format!("CUST-{user_1}");
    sqlx::query(
        "INSERT INTO customers (id, organization_id, user_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2::uuid, $3::uuid, $1, 'Customer 1', 'LIVE', now()) ON CONFLICT (id) DO NOTHING",
    )
    .bind(&cust_1)
    .bind(&shop.org)
    .bind(Uuid::parse_str(&user_1).unwrap())
    .execute(&shop.desk.pool)
    .await
    .expect("insert customer");

    shop.identity.answer(
        "/auth/otp/verify",
        200,
        &json!({ "verified": true, "login": login_body(VALID_A, REFRESH_1) }).to_string(),
    );

    // First OTP sign-in with sid
    let signin_1 = shop
        .send(
            Call::new(Method::POST, "/api/auth/otp/verify")
                .body(json!({ "challengeId": "ch-1", "code": "123456" }))
                .app_sid(&sid),
        )
        .await;
    assert_eq!(signin_1.status, StatusCode::OK);
    let session_1 = signin_1.session_token();

    // After first sign-in, my_planner_drafts must show exactly 1 draft
    let drafts_res_1 = shop
        .send(Call::new(Method::GET, "/api/v3/views/my_planner_drafts").session(&session_1))
        .await;
    assert_eq!(drafts_res_1.status, StatusCode::OK);
    let drafts_1 = drafts_res_1.body["data"]["drafts"]
        .as_array()
        .expect("drafts array in view");
    assert_eq!(
        drafts_1.len(),
        1,
        "first sign-in claims the draft: expected exactly 1 draft"
    );

    // Second OTP sign-in with the exact same sid (e.g. repeated sign-in or concurrent tab)
    let signin_2 = shop
        .send(
            Call::new(Method::POST, "/api/auth/otp/verify")
                .body(json!({ "challengeId": "ch-2", "code": "123456" }))
                .app_sid(&sid),
        )
        .await;
    assert_eq!(
        signin_2.status,
        StatusCode::OK,
        "second sign-in succeeds cleanly"
    );
    let session_2 = signin_2.session_token();

    // Under session 2, my_planner_drafts shows exactly 1 draft (moves none and duplicates nothing)
    let drafts_res_2 = shop
        .send(Call::new(Method::GET, "/api/v3/views/my_planner_drafts").session(&session_2))
        .await;
    assert_eq!(drafts_res_2.status, StatusCode::OK);
    let drafts_2 = drafts_res_2.body["data"]["drafts"]
        .as_array()
        .expect("drafts array in view");
    assert_eq!(
        drafts_2.len(),
        1,
        "second sign-in moves none and duplicates nothing: expected exactly 1 draft"
    );

    shop.finish().await;
}

/// 4. A sign-in without the cookie claims nothing and does not fail.
#[actix_web::test]
async fn signin_without_cookie_claims_nothing_and_does_not_fail() {
    let shop = Shop::start().await;
    let sid = AppSessionId::generate().as_str().to_owned();

    // 1. Anonymous draft with sid
    let draft_res = shop
        .send(
            Call::new(Method::POST, "/api/app/planner-drafts")
                .body(json!({
                    "branch": "MAIN",
                    "currency": "USD",
                    "customerSide": "CUSTOMER_BUY",
                    "amount": { "quantity": "250.0000" },
                    "expectedVersion": null,
                }))
                .app_sid(&sid)
                .key(unique("draft-key-4")),
        )
        .await;
    assert_eq!(draft_res.status, StatusCode::OK);
    let draft_id = draft_res.body["id"].as_str().expect("draft id").to_owned();

    // 2. Existing customer signs in WITHOUT sending any xc_app_sid cookie
    let user_id = bound_user(
        &shop.desk.pool,
        SUB_A,
        "ACTIVE",
        "Customer No Sid",
        &[("CUSTOMER", None)],
    )
    .await;
    let customer_id = format!("CUST-{user_id}");
    sqlx::query(
        "INSERT INTO customers (id, organization_id, user_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2::uuid, $3::uuid, $1, 'Customer No Sid', 'LIVE', now()) ON CONFLICT (id) DO NOTHING",
    )
    .bind(&customer_id)
    .bind(&shop.org)
    .bind(Uuid::parse_str(&user_id).unwrap())
    .execute(&shop.desk.pool)
    .await
    .expect("insert customer");

    shop.identity.answer(
        "/auth/otp/verify",
        200,
        &json!({ "verified": true, "login": login_body(VALID_A, REFRESH_1) }).to_string(),
    );

    // OTP verify WITHOUT app_sid (app_sid is None)
    let otp_res = shop
        .send(Call::new(Method::POST, "/api/auth/otp/verify").body(json!({
            "challengeId": "challenge-otp-4",
            "code": "123456",
        })))
        .await;
    assert_eq!(
        otp_res.status,
        StatusCode::OK,
        "sign-in without cookie does not fail"
    );
    let cust_session = otp_res.session_token();

    // 3. Customer's my_planner_drafts is empty (nothing claimed)
    let cust_drafts_res = shop
        .send(Call::new(Method::GET, "/api/v3/views/my_planner_drafts").session(&cust_session))
        .await;
    assert_eq!(cust_drafts_res.status, StatusCode::OK);
    let cust_drafts = cust_drafts_res.body["data"]["drafts"]
        .as_array()
        .expect("drafts array");
    assert_eq!(cust_drafts.len(), 0, "customer without cookie has 0 drafts");

    // 4. Anonymous session still owns its draft
    let anon_drafts_res = shop
        .send(Call::new(Method::GET, "/api/v3/views/my_planner_drafts").app_sid(&sid))
        .await;
    assert_eq!(anon_drafts_res.status, StatusCode::OK);
    let anon_drafts = anon_drafts_res.body["data"]["drafts"]
        .as_array()
        .expect("drafts array");
    assert_eq!(
        anon_drafts.len(),
        1,
        "anonymous session still owns its draft"
    );
    assert_eq!(anon_drafts[0]["id"].as_str(), Some(draft_id.as_str()));

    shop.finish().await;
}
