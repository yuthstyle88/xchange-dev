//! KWQ-C5..C9 — the web request's routes end to end through the route table on a real PostgreSQL
//! (design `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D2, D3, D8, D12; §4.3a W1–W5;
//! §4.4 B2–B4, B11, B11a, B11b, B12). One file, one harness; each test name starts with its
//! ticket: `c5_` `webRequestCreate`, `c6_` `webRequestQrReissue`, `c7_` `webRequestCancel`,
//! `c8_` `my_web_request`, `c9_` `queue-rates`. Run one ticket's with
//! `cargo test -p xchange --test all_<n> kwq_c58_web_request_routes_test::c5_`.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! The wire shapes are the `[required]` headers of `commands/web_request_{create,qr,cancel}.rs`,
//! `views/my_web_request.rs` and `public/queue_rates.rs`. The harness (database, header sessions,
//! customers Alice and Bob) is `s7_queue_ticket_take_test.rs`'s; the staff scan goes through
//! `POST /api/kiosk issueByToken` (slice a).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

//! branch's organisation (`ORG`), so no take can be refused for the fixture's sake. Needs `TEST_DATABASE_URL`.

mod common;

#[path = "rates_re4c_common/mod.rs"]
mod rates_re4c_common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const TELLER_ID: &str = "00000000-0000-0000-0000-000000000010";
const MANAGER_ID: &str = "00000000-0000-0000-0000-000000000020";
const FINANCE_ID: &str = "00000000-0000-0000-0000-000000000030";
const CUSTOMER_ALICE_ID: &str = "00000000-0000-0000-0000-000000000040";
const CUSTOMER_BOB_ID: &str = "00000000-0000-0000-0000-000000000050";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

// ------------------------------------------------------------ database

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("kwqc58");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");

        sqlx::query(
            "INSERT INTO branches (id, name, organization_id, code) VALUES ($1, 'Branch 1', $2::uuid, $1)",
        )
        .bind(B01)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert branch");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES \
               ($1::uuid, $6::uuid, 'Teller 1', 'ACTIVE', 'test'), \
               ($2::uuid, $6::uuid, 'Manager 1', 'ACTIVE', 'test'), \
               ($3::uuid, $6::uuid, 'Finance 1', 'ACTIVE', 'test'), \
               ($4::uuid, $6::uuid, 'Customer Alice', 'ACTIVE', 'test'), \
               ($5::uuid, $6::uuid, 'Customer Bob', 'ACTIVE', 'test')",
        )
        .bind(TELLER_ID)
        .bind(MANAGER_ID)
        .bind(FINANCE_ID)
        .bind(CUSTOMER_ALICE_ID)
        .bind(CUSTOMER_BOB_ID)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert test users");

        // Shift for teller
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('shift-1', $1, 'OPEN', now(), 'test', 'req-open', $2::uuid)",
        )
        .bind(B01)
        .bind(TELLER_ID)
        .execute(&pool)
        .await
        .expect("insert shift");

        // Counter for teller
        sqlx::query(
            "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
             VALUES ($1::uuid, $2, $3::uuid, 'ACTIVE') \
             ON CONFLICT (branch_id, teller_id) DO NOTHING",
        )
        .bind(ORG)
        .bind(B01)
        .bind(TELLER_ID)
        .execute(&pool)
        .await
        .expect("insert counter");

        // Ensure branch_service_status exists and is open with 1 active counter
        sqlx::query(
            "INSERT INTO branch_service_status (branch_id, organization_id, queue_open, active_counters, queue_version) \
             VALUES ($1, $2::uuid, true, 1, 1) \
             ON CONFLICT (branch_id) DO UPDATE SET queue_open = true, active_counters = 1, queue_version = 1",
        )
        .bind(B01)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("upsert branch_service_status");

        // Seed counter rate for USD per RE-7
        rates_re4c_common::set_counter_rate(&pool, B01, "USD", "35.0", "36.0").await;

        Self { pool, schema }
    }

    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ callers

struct HeaderUsers;

fn user_id_for_role(role_name: &str) -> String {
    match role_name {
        "TELLER" => TELLER_ID.to_string(),
        "BRANCH_MANAGER" => MANAGER_ID.to_string(),
        "FINANCE" => FINANCE_ID.to_string(),
        "CUSTOMER" => CUSTOMER_ALICE_ID.to_string(),
        _ => Uuid::new_v4().to_string(),
    }
}

fn user_of(spec: &str) -> Option<User> {
    let (role_part, target_part) = spec.split_once('@')?;
    let role = match role_part {
        "FINANCE" => Role::Finance,
        "ADMIN" => Role::Admin,
        "EXECUTIVE" => Role::Executive,
        "BRANCH_MANAGER" => Role::BranchManager,
        "TELLER" => Role::Teller,
        "CUSTOMER" => Role::Customer,
        other => panic!("unknown role {other}"),
    };

    if role == Role::Customer {
        let uid = if target_part == "customer-alice" {
            CUSTOMER_ALICE_ID.to_string()
        } else if target_part == "customer-bob" {
            CUSTOMER_BOB_ID.to_string()
        } else {
            Uuid::new_v4().to_string()
        };
        return Some(User::new(
            &uid,
            ORG,
            vec![ScopedRole::new(
                Role::Customer,
                None,
                DateTime::<Utc>::UNIX_EPOCH,
                None,
            )],
            Some(target_part),
        ));
    }

    let uid = user_id_for_role(role_part);
    let branch = (target_part != "*").then(|| BranchId::new(target_part));
    Some(User::new(
        &uid,
        ORG,
        vec![ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
}

fn spec_of(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get(AS)
        .and_then(|v| v.to_str().ok().map(|s| s.to_owned()))
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            match spec {
                Some(s) => {
                    let user = user_of(&s);
                    Callers {
                        user: user.clone(),
                        staff_user: user,
                    }
                }
                None => Callers {
                    user: None,
                    staff_user: None,
                },
            }
        })
    }
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(
    db: &TestDb,
    who: Option<&str>,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: Option<&str>,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST));

    if let Some(who) = who {
        req = req.insert_header((AS, who));
    }

    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .set_payload(body.to_string());
    }

    if let Some(k) = key {
        req = req.insert_header(("idempotency-key", k));
    }

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);

    Answer { status, text, body }
}

// ------------------------------------------------------------ the web request's helpers

const ALICE: &str = "CUSTOMER@customer-alice";
const BOB: &str = "CUSTOMER@customer-bob";
const TELLER: &str = "TELLER@XC-B01";

fn key() -> String {
    Uuid::new_v4().to_string()
}

async fn create(db: &TestDb, who: Option<&str>, body: Value, k: &str) -> Answer {
    call(db, who, Method::POST, "/api/app/web-requests", Some(body), Some(k)).await
}

fn usd100() -> Value {
    json!({ "branch": B01, "type": "BUY", "currency": "USD", "amount": 100 })
}

/// Alice's web request at B01: (id, token from `requestQrUrl`).
async fn alices_request(db: &TestDb) -> (String, String) {
    let ans = create(db, Some(ALICE), usd100(), &key()).await;
    assert_eq!(ans.status, StatusCode::OK, "create: {}", ans.text);
    let id = ans.body["requestId"].as_str().expect("requestId").to_owned();
    let token = token_of(&ans.body);
    (id, token)
}

fn token_of(body: &Value) -> String {
    body["requestQrUrl"]
        .as_str()
        .and_then(|u| u.strip_prefix("/q#"))
        .unwrap_or_else(|| panic!("requestQrUrl is /q#<token>: {body}"))
        .to_owned()
}

async fn reissue(db: &TestDb, who: &str, id: &str) -> Answer {
    call(
        db,
        Some(who),
        Method::POST,
        &format!("/api/app/web-requests/{id}/qr"),
        Some(json!({ "branch": B01 })),
        Some(&key()),
    )
    .await
}

async fn cancel(db: &TestDb, who: &str, id: &str) -> Answer {
    call(
        db,
        Some(who),
        Method::POST,
        &format!("/api/app/web-requests/{id}/cancel"),
        Some(json!({ "branch": B01 })),
        Some(&key()),
    )
    .await
}

async fn view(db: &TestDb, who: Option<&str>, id: &str, since: Option<&str>) -> Answer {
    let mut uri = format!("/api/v3/views/my_web_request?branch={B01}&id={id}");
    if let Some(s) = since {
        uri.push_str(&format!("&since={s}"));
    }
    call(db, who, Method::GET, &uri, None, None).await
}

/// The staff scan at the queue machine (slice a).
async fn scan(db: &TestDb, token: &str) -> Answer {
    call(
        db,
        Some(TELLER),
        Method::POST,
        "/api/kiosk",
        Some(json!({ "action": "issueByToken", "payload": { "branchId": B01, "token": token } })),
        None,
    )
    .await
}

async fn token_matches(db: &TestDb, id: &str, token: &str) -> bool {
    sqlx::query_scalar(
        "SELECT cancel_token_hash = encode(sha256(convert_to($2, 'UTF8')), 'hex') \
         FROM kiosk_requests WHERE id = $1",
    )
    .bind(id)
    .bind(token)
    .fetch_one(&db.pool)
    .await
    .expect("read the hash")
}

fn not_found(ans: &Answer) {
    assert_eq!(ans.status, StatusCode::NOT_FOUND, "{}", ans.text);
    assert_eq!(ans.body["code"], json!("notFound"), "{}", ans.text);
}

fn keys(v: &Value) -> std::collections::BTreeSet<String> {
    v.as_object().expect("an object").keys().cloned().collect()
}

// ------------------------------------------------------------ c5: webRequestCreate

#[tokio::test(flavor = "multi_thread")]
async fn c5_signed_out_is_401() {
    let db = TestDb::start().await;
    let ans = create(&db, None, usd100(), &key()).await;
    assert_eq!(ans.status, StatusCode::UNAUTHORIZED, "{}", ans.text);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn c5_creates_a_quoted_request_with_the_member_and_a_token_and_no_ticket() {
    let db = TestDb::start().await;
    let ans = create(&db, Some(ALICE), usd100(), &key()).await;
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(
        keys(&ans.body),
        ["expiresAt", "requestId", "requestQrUrl"].iter().map(|s| s.to_string()).collect()
    );
    let id = ans.body["requestId"].as_str().unwrap().to_owned();
    let token = token_of(&ans.body);
    assert_eq!(Uuid::parse_str(&token).map(|u| u.get_version_num()).ok(), Some(4), "a UUID v4 token");
    assert!(token_matches(&db, &id, &token).await, "only the token's SHA-256 is stored");
    let (status, customer, created): (String, Option<String>, DateTime<Utc>) = sqlx::query_as(
        "SELECT status, customer_id, created_at FROM kiosk_requests WHERE id = $1",
    )
    .bind(&id)
    .fetch_one(&db.pool)
    .await
    .expect("row");
    assert_eq!(status, "QUOTED");
    assert_eq!(customer.as_deref(), Some("customer-alice"));
    let expires = DateTime::parse_from_rfc3339(ans.body["expiresAt"].as_str().unwrap()).expect("RFC 3339");
    assert_eq!(expires.with_timezone(&Utc), created + chrono::Duration::minutes(60));
    let tickets: i64 = sqlx::query_scalar("SELECT count(*) FROM queue_tickets")
        .fetch_one(&db.pool)
        .await
        .expect("count");
    assert_eq!(tickets, 0, "no ticket, no number (B3)");
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn c5_a_second_open_request_is_409_already_queued_naming_the_first() {
    let db = TestDb::start().await;
    let (first, _) = alices_request(&db).await;
    let ans = create(&db, Some(ALICE), usd100(), &key()).await;
    assert_eq!(ans.status, StatusCode::CONFLICT, "{}", ans.text);
    assert_eq!(ans.body["code"], json!("alreadyQueued"));
    assert_eq!(ans.body["message"], json!(first));
    // Bob is not blocked by Alice's request
    let bob = create(&db, Some(BOB), usd100(), &key()).await;
    assert_eq!(bob.status, StatusCode::OK, "{}", bob.text);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn c5_a_refused_quote_is_400_quote_refused_and_a_replay_keeps_no_token() {
    let db = TestDb::start().await;
    let bad = create(
        &db,
        Some(ALICE),
        json!({ "branch": B01, "type": "BUY", "currency": "USD", "amount": 0 }),
        &key(),
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);
    assert_eq!(bad.body["code"], json!("quoteRefused"));

    let k = key();
    let first = create(&db, Some(ALICE), usd100(), &k).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let replay = create(&db, Some(ALICE), usd100(), &k).await;
    assert_eq!(replay.status, StatusCode::OK, "{}", replay.text);
    assert_eq!(replay.body["requestId"], first.body["requestId"]);
    assert_eq!(replay.body["requestQrUrl"], Value::Null, "the stored answer keeps no token");
    db.finish().await;
}

// ------------------------------------------------------------ c6: webRequestQrReissue

#[tokio::test(flavor = "multi_thread")]
async fn c6_a_fresh_qr_replaces_the_token_and_only_for_the_owner() {
    let db = TestDb::start().await;
    let (id, old) = alices_request(&db).await;

    let ans = reissue(&db, ALICE, &id).await;
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(ans.body["requestId"], json!(id));
    let new = token_of(&ans.body);
    assert_ne!(new, old);
    assert!(token_matches(&db, &id, &new).await);
    not_found(&scan(&db, &old).await);

    not_found(&reissue(&db, BOB, &id).await);
    not_found(&reissue(&db, ALICE, "no-such-request").await);
    assert!(token_matches(&db, &id, &new).await, "Bob's attempt changed nothing");
    db.finish().await;
}

// ------------------------------------------------------------ c7: webRequestCancel

#[tokio::test(flavor = "multi_thread")]
async fn c7_the_owner_cancels_in_one_press_and_others_get_not_found() {
    let db = TestDb::start().await;
    let (id, _) = alices_request(&db).await;
    not_found(&cancel(&db, BOB, &id).await);

    let ans = cancel(&db, ALICE, &id).await;
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(ans.body, json!({ "requestId": id, "status": "CANCELLED" }));

    let again = cancel(&db, ALICE, &id).await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.text);
    assert_eq!(again.body["code"], json!("notCancellable"));
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn c7_after_the_scan_the_cancel_ends_the_ticket() {
    let db = TestDb::start().await;
    let (id, token) = alices_request(&db).await;
    let scanned = scan(&db, &token).await;
    assert_eq!(scanned.status, StatusCode::OK, "{}", scanned.text);

    let ans = cancel(&db, ALICE, &id).await;
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    let ticket: String = sqlx::query_scalar("SELECT status FROM queue_tickets WHERE kiosk_request_id = $1")
        .bind(&id)
        .fetch_one(&db.pool)
        .await
        .expect("ticket");
    assert_ne!(ticket, "WAITING", "the ticket is ended with the request");
    db.finish().await;
}

// ------------------------------------------------------------ c8: my_web_request

#[tokio::test(flavor = "multi_thread")]
async fn c8_the_member_sees_no_number_before_the_scan_and_the_number_after() {
    let db = TestDb::start().await;
    let (id, token) = alices_request(&db).await;

    assert_eq!(view(&db, None, &id, None).await.status, StatusCode::UNAUTHORIZED);
    not_found(&view(&db, Some(BOB), &id, None).await);

    let before = view(&db, Some(ALICE), &id, None).await;
    assert_eq!(before.status, StatusCode::OK, "{}", before.text);
    assert_eq!(before.body["changed"], json!(true));
    let data = &before.body["data"];
    assert_eq!(
        keys(data),
        [
            "amount", "branch", "currency", "displayNumber", "expiresAt", "requestId", "status",
            "ticketsAhead", "type"
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        "no counter, nothing else (R-7): {data}"
    );
    assert_eq!(data["status"], json!("QUOTED"));
    assert_eq!(data["displayNumber"], Value::Null);
    assert_eq!(data["type"], json!("BUY"));
    assert_eq!(data["currency"], json!("USD"));
    assert_eq!(data["branch"]["code"], json!(B01));
    assert!(data["expiresAt"].is_string());
    let v0 = before.body["version"].as_str().expect("version is a string (W4a)").to_owned();

    let same = view(&db, Some(ALICE), &id, Some(&v0)).await;
    assert_eq!(same.body, json!({ "version": v0, "changed": false }));

    let scanned = scan(&db, &token).await;
    assert_eq!(scanned.status, StatusCode::OK, "{}", scanned.text);
    let after = view(&db, Some(ALICE), &id, Some(&v0)).await;
    assert_eq!(after.body["changed"], json!(true), "{}", after.text);
    assert_eq!(after.body["data"]["status"], json!("WAITING"));
    assert_eq!(after.body["data"]["displayNumber"], scanned.body["displayNumber"]);
    assert!(after.body["data"]["ticketsAhead"].is_number());
    assert_eq!(after.body["data"]["expiresAt"], Value::Null);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn c8_a_cancel_or_a_fresh_qr_moves_the_version_and_a_purge_is_not_found() {
    let db = TestDb::start().await;
    let (id, _) = alices_request(&db).await;
    let v0 = view(&db, Some(ALICE), &id, None).await.body["version"]
        .as_str()
        .unwrap()
        .to_owned();

    assert_eq!(reissue(&db, ALICE, &id).await.status, StatusCode::OK);
    let after_qr = view(&db, Some(ALICE), &id, Some(&v0)).await;
    assert_eq!(after_qr.body["changed"], json!(true), "B11a: a reissue moves the version");
    let v1 = after_qr.body["version"].as_str().unwrap().to_owned();
    assert_ne!(v1, v0);

    assert_eq!(cancel(&db, ALICE, &id).await.status, StatusCode::OK);
    let after_cancel = view(&db, Some(ALICE), &id, Some(&v1)).await;
    assert_eq!(after_cancel.body["changed"], json!(true), "B11a: a cancel moves the version");
    assert_eq!(after_cancel.body["data"]["status"], json!("CANCELLED"));
    let v2 = after_cancel.body["version"].as_str().unwrap().to_owned();

    sqlx::query("DELETE FROM kiosk_requests WHERE id = $1")
        .bind(&id)
        .execute(&db.pool)
        .await
        .expect("purge it");
    not_found(&view(&db, Some(ALICE), &id, Some(&v2)).await);
    db.finish().await;
}

// ------------------------------------------------------------ c9: queue-rates

#[tokio::test(flavor = "multi_thread")]
async fn c9_the_web_page_gets_the_kiosk_rates_and_unknown_is_the_public_404() {
    let db = TestDb::start().await;
    let kiosk = call(
        &db,
        None,
        Method::POST,
        "/api/kiosk",
        Some(json!({ "action": "rates", "payload": { "branchId": B01 } })),
        None,
    )
    .await;
    assert_eq!(kiosk.status, StatusCode::OK, "{}", kiosk.text);

    let web = call(
        &db,
        None,
        Method::GET,
        &format!("/api/public/orgs/XC/branches/{B01}/queue-rates"),
        None,
        None,
    )
    .await;
    assert_eq!(web.status, StatusCode::OK, "{}", web.text);
    assert_eq!(web.body, kiosk.body, "the same rates the kiosk core answers");

    let rates_404 = call(&db, None, Method::GET, "/api/public/orgs/XC/branches/NOPE/rates", None, None).await;
    for uri in ["/api/public/orgs/XC/branches/NOPE/queue-rates", "/api/public/orgs/NOPE/branches/XC-B01/queue-rates"] {
        let ans = call(&db, None, Method::GET, uri, None, None).await;
        assert_eq!((ans.status, ans.text.as_str()), (rates_404.status, rates_404.text.as_str()), "{uri}");
    }
    db.finish().await;
}

