//! BT-6 (design `docs/design/blueprint/kiosk-shared-phone.md` §7.3 T-D7; §7.7 the scan and
//! `webRequestCancel`; §7.8 rows T15, T16, T18): the staff scan of a web request's QR
//! (`issueByToken`) and the member's cancel end, on their own transaction, the member's customer
//! session bound to a tablet of that branch, and clear the tablet. The member's other sessions
//! (their own phone, a tablet of another branch) stay. The scan's answer is unchanged.
//!
//! Written before the implementation by the BE lane's skeleton (BT-0). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! T15 and T16 run on `main` today and fail on the result: nothing ends the session yet. T18
//! needs `webRequestCancel` (`POST /api/app/web-requests/{id}/cancel`), which is in PR 4481
//! (`feat/kwq-c`) and not on `main` at BT-0: it is `#[ignore]`d until then. BT-6 removes that one
//! `#[ignore]` once PR 4481 has merged, and changes nothing else in this file.
//!
//! The harness (database, staff by header, the web request made in SQL) is copied from
//! `kwq_a5_issue_by_token_route_test.rs`. Tablets are written straight into `branch_tablets`
//! (§7.6) with their bound session, so nothing here depends on BT-4 or BT-5.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

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
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B02: &str = "XC-B02";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const TELLER_A_ID: &str = "00000000-0000-0000-0000-000000000010";
const MEMBER_ID: &str = "00000000-0000-0000-0000-0000000000b6";
const CUSTOMER: &str = "cust-1";
const TABLET_B01: &str = "7ab1e700-0000-4000-8000-0000000000b1";
const TABLET_B02: &str = "7ab1e700-0000-4000-8000-0000000000b2";
const KIOSK_IP: &str = "10.20.30.40";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
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
        let schema = unique("btscan");
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
            "INSERT INTO branches (id, name, organization_id) VALUES \
             ($1, 'Branch 1', $3::uuid), ($2, 'Branch 2', $3::uuid)",
        )
        .bind(B01)
        .bind(B02)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert branches");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) VALUES \
             ($1::uuid, $3::uuid, 'Teller A', 'ACTIVE', 'test'), \
             ($2::uuid, $3::uuid, 'Member', 'ACTIVE', 'test')",
        )
        .bind(TELLER_A_ID)
        .bind(MEMBER_ID)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert users");

        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, preferred_language, \
                 status, mode, user_id, created_at) \
             VALUES ($1, $2::uuid, $1, 'Member', 'th', 'ACTIVE', 'LIVE', $3::uuid, now())",
        )
        .bind(CUSTOMER)
        .bind(ORG)
        .bind(MEMBER_ID)
        .execute(&pool)
        .await
        .expect("insert the member's customer");

        let count_a = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO cash_counts (id, organization_id, branch_id, kind, counted_by, counted_at, idempotency_key) \
             VALUES ($1, $2::uuid, $3, 'OPEN', $4::uuid, now(), 'key-a')",
        )
        .bind(count_a)
        .bind(ORG)
        .bind(B01)
        .bind(TELLER_A_ID)
        .execute(&pool)
        .await
        .expect("insert cash count");
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('shift-a', $1, 'OPEN', now(), 'test', 'req-open-a', $2::uuid)",
        )
        .bind(B01)
        .bind(TELLER_A_ID)
        .execute(&pool)
        .await
        .expect("insert shift");
        sqlx::query(
            "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
             VALUES ($1::uuid, $2, $3::uuid, 'ACTIVE') ON CONFLICT (branch_id, teller_id) DO NOTHING",
        )
        .bind(ORG)
        .bind(B01)
        .bind(TELLER_A_ID)
        .execute(&pool)
        .await
        .expect("insert counter");
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

    /// A 12-hour session of the member: its cookie token.
    async fn member_session(&self) -> String {
        let mut conn = self.pool.acquire().await.expect("a connection");
        xc_testdb::sessions::seed_session(
            &mut conn,
            MEMBER_ID,
            None,
            Utc::now() + chrono::Duration::hours(12),
        )
        .await
        .expect("seed a session")
    }

    /// A paired tablet of `branch` holding the member's session `token`.
    async fn tablet_holding(&self, id: &str, branch: &str, token: &str) {
        sqlx::query(
            "INSERT INTO branch_tablets (id, branch_id, name, created_at, created_by, token_hash, paired_at, \
             customer_session_hash, customer_user_id) \
             VALUES ($1::uuid, $2, 'tablet', now(), 'admin', $3, now(), $4, $5::uuid)",
        )
        .bind(id)
        .bind(branch)
        .bind(sha256_hex(&format!("xcbt_{id}")))
        .bind(sha256_hex(token))
        .bind(MEMBER_ID)
        .execute(&self.pool)
        .await
        .expect("a tablet holding the member's session (§7.6)");
    }

    async fn session_exists(&self, token: &str) -> bool {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM user_sessions WHERE hash = $1")
            .bind(sha256_hex(token))
            .fetch_one(&self.pool)
            .await
            .expect("count")
            == 1
    }

    /// `(customer_session_hash, customer_user_id)` of a tablet.
    async fn bound(&self, id: &str) -> (Option<String>, Option<String>) {
        sqlx::query_as(
            "SELECT customer_session_hash, customer_user_id::text FROM branch_tablets WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("read the tablet")
    }
}

/// The member signed in on the B01 tablet, on a B02 tablet and on their own phone: the three
/// session tokens.
struct Sessions {
    on_b01_tablet: String,
    on_b02_tablet: String,
    phone: String,
}

async fn three_sessions(db: &TestDb) -> Sessions {
    let s = Sessions {
        on_b01_tablet: db.member_session().await,
        on_b02_tablet: db.member_session().await,
        phone: db.member_session().await,
    };
    db.tablet_holding(TABLET_B01, B01, &s.on_b01_tablet).await;
    db.tablet_holding(TABLET_B02, B02, &s.on_b02_tablet).await;
    s
}

// ------------------------------------------------------------ callers

struct HeaderUsers;

fn user_of(spec: &str) -> Option<User> {
    let (role_part, target_part) = spec.split_once('@')?;
    let from = DateTime::<Utc>::UNIX_EPOCH;
    match role_part {
        "TELLER_A" => {
            let branch = (target_part != "*").then(|| BranchId::new(target_part));
            Some(User::new(
                TELLER_A_ID,
                ORG,
                vec![ScopedRole::new(Role::Teller, branch, from, None)],
                None,
            ))
        }
        "CUSTOMER" => Some(User::new(
            MEMBER_ID,
            ORG,
            vec![ScopedRole::new(Role::Customer, None, from, None)],
            Some(target_part),
        )),
        other => panic!("unknown role {other}"),
    }
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok().map(|s| s.to_owned()));
        Box::pin(async move {
            let user = spec.as_deref().and_then(user_of);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ HTTP harness

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(db: &TestDb, who: Option<&str>, uri: &str, body: Value, key: Option<&str>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-real-ip", KIOSK_IP))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .set_payload(body.to_string());
    if let Some(who) = who {
        req = req.insert_header((AS, who));
    }
    if let Some(key) = key {
        req = req.insert_header(("idempotency-key", key));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn kiosk(db: &TestDb, who: Option<&str>, action: &str, payload: Value) -> Answer {
    call(
        db,
        who,
        "/api/kiosk",
        json!({ "action": action, "payload": payload }),
        None,
    )
    .await
}

/// A QUOTED web request of `B01` whose request token is `token`, made by `customer` (`None`: a
/// kiosk request with no member). Its id.
async fn web_request(db: &TestDb, token: &str, customer: Option<&str>) -> String {
    let ans = kiosk(
        db,
        None,
        "quote",
        json!({ "type": "BUY", "currency": "USD", "amount": 100, "branchId": B01 }),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "quote: {}", ans.text);
    let id = ans.body["id"].as_str().expect("quote id").to_owned();
    sqlx::query(
        "UPDATE kiosk_requests SET created_at = now() - interval '10s', \
         cancel_token_hash = encode(sha256(convert_to($2, 'UTF8')), 'hex'), customer_id = $3 WHERE id = $1",
    )
    .bind(&id)
    .bind(token)
    .bind(customer)
    .execute(&db.pool)
    .await
    .expect("make it a web request");
    id
}

async fn scan(db: &TestDb, token: &str) -> Answer {
    kiosk(
        db,
        Some(&format!("TELLER_A@{B01}")),
        "issueByToken",
        json!({ "branchId": B01, "token": token }),
    )
    .await
}

// ------------------------------------------------------------ T15, T16

/// T15, T16: the staff scan deletes the member's session bound to the B01 tablet and clears the
/// tablet, in the scan's transaction; the scan answers as before; the member's phone session and
/// the B02 tablet's session stay. A repeat scan answers the same and ends nothing more.
#[tokio::test(flavor = "multi_thread")]
async fn t15_t16_the_scan_ends_only_the_session_bound_to_a_tablet_of_that_branch() {
    let db = TestDb::start().await;
    let s = three_sessions(&db).await;
    web_request(&db, "tok-1", Some(CUSTOMER)).await;

    let ans = scan(&db, "tok-1").await;
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(
        ans.body.as_object().map(|o| o.len()),
        Some(3),
        "the scan's answer is unchanged: {}",
        ans.text
    );
    assert!(
        !db.session_exists(&s.on_b01_tablet).await,
        "T15: the B01 tablet's session is deleted"
    );
    assert_eq!(
        db.bound(TABLET_B01).await,
        (None, None),
        "T15: the tablet is cleared"
    );
    assert!(
        db.session_exists(&s.phone).await,
        "T16: the phone session stays"
    );
    assert!(
        db.session_exists(&s.on_b02_tablet).await,
        "another branch's tablet session stays"
    );
    assert_eq!(
        db.bound(TABLET_B02).await,
        (
            Some(sha256_hex(&s.on_b02_tablet)),
            Some(MEMBER_ID.to_owned())
        )
    );

    let again = scan(&db, "tok-1").await;
    assert_eq!(
        (again.status, &again.body),
        (ans.status, &ans.body),
        "{}",
        again.text
    );
    assert!(db.session_exists(&s.phone).await);
    assert!(db.session_exists(&s.on_b02_tablet).await);
    db.finish().await;
}

/// A scanned request with no member (a kiosk request) ends no session.
#[tokio::test(flavor = "multi_thread")]
async fn a_scan_of_a_request_with_no_member_ends_nothing() {
    let db = TestDb::start().await;
    let s = three_sessions(&db).await;
    web_request(&db, "tok-2", None).await;
    let ans = scan(&db, "tok-2").await;
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    for token in [&s.on_b01_tablet, &s.on_b02_tablet, &s.phone] {
        assert!(db.session_exists(token).await);
    }
    assert_eq!(
        db.bound(TABLET_B01).await,
        (
            Some(sha256_hex(&s.on_b01_tablet)),
            Some(MEMBER_ID.to_owned())
        )
    );
    db.finish().await;
}

// ------------------------------------------------------------ T18

/// T18: the member's cancel (`webRequestCancel`) deletes their session bound to the B01 tablet
/// on the cancel's transaction; the phone session stays.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs PR 4481 (feat/kwq-c): webRequestCancel is not on main at BT-0; BT-6 removes this ignore once it merges"]
async fn t18_the_members_cancel_ends_the_session_bound_to_the_tablet() {
    let db = TestDb::start().await;
    let s = three_sessions(&db).await;
    let id = web_request(&db, "tok-3", Some(CUSTOMER)).await;

    let ans = call(
        &db,
        Some(&format!("CUSTOMER@{CUSTOMER}")),
        &format!("/api/app/web-requests/{id}/cancel"),
        json!({ "branch": B01 }),
        Some(&uuid::Uuid::new_v4().to_string()),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(ans.body["status"], json!("CANCELLED"), "{}", ans.text);
    assert!(
        !db.session_exists(&s.on_b01_tablet).await,
        "the B01 tablet's session is deleted"
    );
    assert_eq!(db.bound(TABLET_B01).await, (None, None));
    assert!(db.session_exists(&s.phone).await);
    assert!(db.session_exists(&s.on_b02_tablet).await);
    db.finish().await;
}
