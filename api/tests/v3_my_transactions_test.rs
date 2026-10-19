//! Ticket #1003 (APP-11): view `my_transactions` over HTTP.
//! Design `docs/design/blueprint/customer-app.md` §3 (C04), §4.4, §9.1.
//!
//! Written before the implementation by the test-writer gemini/pool-4, on the
//! FE lane controller claude/5edf7bb5's order. Do not edit, delete or weaken.
//!
//! **Pinned by this file** (ruling on #1003):
//! - `GET /api/v3/views/my_transactions`
//! - A CUSTOMER session sees only its own completed exchanges, with branch and receipt ref.
//! - Its reversed exchange shows as reversed.
//! - Another customer's rows never appear.
//! - The view takes no customer id from the request (`?customerId=` is ignored or refused, never honoured).
//! - A staff session (e.g. Teller, Admin) or no session is refused per the `CustomerSelf` row (400 `roleNotAllowedHere` or 401 `signIn`).
//!
//! **Red on `main` by result:** the route does not exist on `main`, answering 404.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::Value;
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
const VIEW_PATH: &str = "/api/v3/views/my_transactions";

const MAIN: &str = "MAIN";
const B01: &str = "B01";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const CX1: &str = "CX-APP11-1";
const CX2: &str = "CX-APP11-2";

const TX_CX1_1: &str = "00000000-0000-0000-0000-000000000011";
const TX_CX1_2: &str = "00000000-0000-0000-0000-000000000012";
const TX_CX2_1: &str = "00000000-0000-0000-0000-000000000021";

const REF_CX1_1: &str = "RC-CX1-001";
const REF_CX1_2: &str = "RC-CX1-002";
const REF_CX2_1: &str = "RC-CX2-001";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

// ------------------------------------------------------------ database

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
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
        let schema = unique("my_tx");
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

        let room = Self { pool, schema };
        room.setup_data().await;
        room
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn setup_data(&self) {
        // Branches
        self.exec(
            "INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN') ON CONFLICT (id) DO NOTHING",
        )
        .await;
        self.exec("INSERT INTO branches (id, name) VALUES ('B01', 'Branch 01') ON CONFLICT (id) DO NOTHING")
            .await;

        // Customers
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) VALUES \
             ('{CX1}', '{ORG}', '{CX1}', 'Somchai', 'LIVE', now()), \
             ('{CX2}', '{ORG}', '{CX2}', 'Somsri', 'LIVE', now())"
        ))
        .await;

        // Shifts for LIVE transactions
        self.exec(
            "WITH u AS (INSERT INTO users (organization_id, name, status, created_by) \
                 VALUES ('00000000-0000-0000-0000-000000000001'::uuid, 'Teller MAIN', 'ACTIVE', 'test') \
                 RETURNING id) \
             INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             SELECT 'SH-MAIN', 'MAIN', 'OPEN', now(), 'test', 'req-open-main', u.id FROM u"
        )
        .await;

        self.exec(
            "WITH u AS (INSERT INTO users (organization_id, name, status, created_by) \
                 VALUES ('00000000-0000-0000-0000-000000000001'::uuid, 'Teller B01', 'ACTIVE', 'test') \
                 RETURNING id) \
             INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             SELECT 'SH-B01', 'B01', 'OPEN', now(), 'test', 'req-open-b01', u.id FROM u"
        )
        .await;

        // Files for receipts
        self.exec(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) VALUES \
             ('receipt-cx1-1', 'MAIN', 'test', 'text/html', 'sha-r1', now()), \
             ('receipt-cx1-2', 'B01', 'test', 'text/html', 'sha-r2', now()), \
             ('receipt-cx1-2-rev', 'B01', 'test', 'text/html', 'sha-r2-rev', now()), \
             ('receipt-cx2-1', 'MAIN', 'test', 'text/html', 'sha-r3', now())",
        )
        .await;

        // 1. CX1 - Completed exchange at MAIN (not reversed)
        self.insert_exchange(
            TX_CX1_1,
            MAIN,
            CX1,
            "SH-MAIN",
            "DOC-CX1-1",
            REF_CX1_1,
            "receipt-cx1-1",
            "USD",
            100.0,
            3350.0,
            false,
        )
        .await;

        // 2. CX1 - Completed exchange at B01, reversed (POSTED reversal)
        self.insert_exchange(
            TX_CX1_2,
            B01,
            CX1,
            "SH-B01",
            "DOC-CX1-2",
            REF_CX1_2,
            "receipt-cx1-2",
            "EUR",
            50.0,
            1850.0,
            true,
        )
        .await;

        // 3. CX2 - Completed exchange at MAIN
        self.insert_exchange(
            TX_CX2_1,
            MAIN,
            CX2,
            "SH-MAIN",
            "DOC-CX2-1",
            REF_CX2_1,
            "receipt-cx2-1",
            "JPY",
            10000.0,
            2400.0,
            false,
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_exchange(
        &self,
        tx_id: &str,
        branch: &str,
        customer_id: &str,
        shift_id: &str,
        doc_ref: &str,
        receipt_ref: &str,
        file_key: &str,
        currency: &str,
        amount: f64,
        settle_thb: f64,
        reversed: bool,
    ) {
        let now = "2026-10-12T08:00:00Z";
        self.exec(&format!(
            "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, customer_id, \
                customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
                recorded_by, status, completed_at, organization_id) \
             VALUES ('{tx_id}', '{doc_ref}', '{branch}', 'LIVE', '{shift_id}', 'sub-{tx_id}', 'hash-{tx_id}', \
                '{customer_id}', 'Customer {customer_id}', 'THA', 'teller-1', '', '', '2026-10-12', \
                '{now}', '{now}', 'teller-1', 'COMPLETED', '{now}', '{ORG}')"
        ))
        .await;

        self.exec(&format!(
            "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
                quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
             VALUES (gen_random_uuid(), '{ORG}', '{tx_id}', 'BUY', '{currency}', {amount}, 1, 33.5, \
                {settle_thb}, 0, {settle_thb}, 1, '{now}', '{now}')"
        ))
        .await;

        self.exec(&format!(
            "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, method, \
                reference, account, amount_thb, bank_fee_thb, paid_at, created_at, updated_at) \
             VALUES (gen_random_uuid(), '{ORG}', '{tx_id}', 1, 'Cash', '', 'drawer', {settle_thb}, 0, \
                '{now}', '{now}', '{now}')"
        ))
        .await;

        self.exec(&format!(
            "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
                document_ref, kind, created_at, updated_at) \
             VALUES (gen_random_uuid(), '{ORG}', '{tx_id}', '{receipt_ref}', 1, '{now}', \
                '{file_key}', 'RECEIPT', '{now}', '{now}')"
        ))
        .await;

        if reversed {
            let rev_id = uuid::Uuid::new_v4().to_string();
            let req_id = uuid::Uuid::new_v4().to_string();
            let post_id = uuid::Uuid::new_v4().to_string();
            let rec_rev_id = uuid::Uuid::new_v4().to_string();
            let rev_time = "2026-10-12T09:00:00Z";

            self.exec(&format!(
                "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
                    object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
                    status, idempotency_key) \
                 VALUES ('{req_id}', '{ORG}', 'TRANSACTION_REVERSAL', '{branch}', '{rev_id}', 'h', '1', \
                    '{{}}', 'teller-1', '{rev_time}', 'APPROVED', 'k-{req_id}')"
            ))
            .await;

            self.exec(&format!(
                "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
                    document_ref, kind, created_at, updated_at) \
                 VALUES ('{rec_rev_id}', '{ORG}', '{tx_id}', '{receipt_ref}-REV', 2, '{rev_time}', \
                    '{file_key}-rev', 'REVERSAL', '{rev_time}', '{rev_time}')"
            ))
            .await;

            self.exec(&format!(
                "INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, source_key, \
                    status, posted_at, posted_by) \
                 VALUES ('{post_id}', '{ORG}', '{branch}', 'REVERSAL', '{rev_id}', 'reversal', 'POSTED', \
                    '{rev_time}', 'teller-1')"
            ))
            .await;

            self.exec(&format!(
                "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, \
                    reason, amount_thb, business_date, approval_request_id, posting_id, receipt_id, \
                    requested_by, requested_at, posted_by, posted_at, created_at, updated_at) \
                 VALUES ('{rev_id}', '{ORG}', '{branch}', '{tx_id}', 'POSTED', 'Customer request', \
                    {settle_thb}, '2026-10-12', '{req_id}', '{post_id}', '{rec_rev_id}', 'teller-1', \
                    '{rev_time}', 'teller-1', '{rev_time}', '{rev_time}', '{rev_time}')"
            ))
            .await;
        }
    }

    async fn user(&self) -> String {
        sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique("u"))
        .fetch_one(&self.pool)
        .await
        .expect("insert a user")
    }

    async fn customer(&self, customer_id: &str) -> Who {
        let id = self.user().await;
        Who(format!("{id}|CUSTOMER|{customer_id}"))
    }

    async fn staff(&self, role: Role) -> Who {
        let id = self.user().await;
        Who(format!("{id}|{}|", role.as_str()))
    }

    async fn finish(self) {
        let Room { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ who is signed in

struct Who(String);

fn user_of(spec: &str) -> Option<User> {
    if spec.is_empty() {
        return None;
    }
    let mut parts = spec.splitn(3, '|');
    let (id, role, customer) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        ORG,
        vec![ScopedRole::new(role, None::<BranchId>, from, None)],
        (!customer.is_empty()).then_some(customer),
    ))
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ request helper

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

async fn call(room: &Room, who: Option<&Who>, method: Method, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
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
    if let Some(w) = who {
        req = req.insert_header((AS, w.0.clone()));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    Answer {
        status,
        text: String::from_utf8_lossy(&bytes).into_owned(),
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

fn get_transactions(val: &Value) -> Vec<Value> {
    if let Some(arr) = val.get("data").and_then(|d| d.as_array()) {
        return arr.clone();
    }
    if let Some(arr) = val.pointer("/data/transactions").and_then(|d| d.as_array()) {
        return arr.clone();
    }
    if let Some(arr) = val.pointer("/data/items").and_then(|d| d.as_array()) {
        return arr.clone();
    }
    if let Some(arr) = val.pointer("/data/history").and_then(|d| d.as_array()) {
        return arr.clone();
    }
    panic!("cannot find transactions array in: {val}");
}

fn get_branch(item: &Value) -> Option<&str> {
    item.get("branch")
        .or_else(|| item.get("branchId"))
        .or_else(|| item.get("branch_id"))
        .and_then(|v| v.as_str())
}

fn get_receipt_ref(item: &Value) -> Option<&str> {
    item.get("receiptRef")
        .or_else(|| item.get("receipt_ref"))
        .or_else(|| item.get("receiptNumber"))
        .or_else(|| item.get("receipt_number"))
        .or_else(|| item.get("documentRef"))
        .or_else(|| item.get("document_ref"))
        .or_else(|| item.get("ref"))
        .and_then(|v| v.as_str())
}

fn is_status_reversed(item: &Value) -> bool {
    let s = item["status"].as_str().unwrap_or("").to_uppercase();
    s == "REVERSED"
        || item["isReversed"].as_bool().unwrap_or(false)
        || item["reversed"].as_bool().unwrap_or(false)
}

fn is_status_completed(item: &Value) -> bool {
    let s = item["status"].as_str().unwrap_or("").to_uppercase();
    s == "COMPLETED"
}

// ------------------------------------------------------------ tests

#[actix_web::test]
async fn customer_sees_only_own_completed_exchanges_with_branch_and_receipt_ref() {
    let room = Room::start().await;
    let cx1 = room.customer(CX1).await;

    let ans = call(&room, Some(&cx1), Method::GET, VIEW_PATH).await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "customer view must return 200: {}",
        ans.text
    );

    let items = get_transactions(&ans.body);
    assert_eq!(
        items.len(),
        2,
        "CX1 should see exactly 2 transactions (1 normal, 1 reversed)"
    );

    // Verify both branch and receipt ref are present
    let has_tx1 = items.iter().any(|item| {
        let b = get_branch(item);
        let r = get_receipt_ref(item);
        b == Some(MAIN) && r == Some(REF_CX1_1)
    });
    assert!(
        has_tx1,
        "CX1 should see TX 1 with branch MAIN and receipt ref {REF_CX1_1}: {items:?}"
    );

    let has_tx2 = items.iter().any(|item| {
        let b = get_branch(item);
        let r = get_receipt_ref(item);
        b == Some(B01) && r.is_some_and(|ref_str| ref_str.starts_with(REF_CX1_2))
    });
    assert!(
        has_tx2,
        "CX1 should see TX 2 with branch B01 and receipt ref {REF_CX1_2}: {items:?}"
    );

    // Verify CX2's transaction is not present
    let has_tx3 = items.iter().any(|item| {
        get_receipt_ref(item) == Some(REF_CX2_1)
            || item.get("id").and_then(|v| v.as_str()) == Some(TX_CX2_1)
    });
    assert!(!has_tx3, "CX1 must never see CX2's transaction: {items:?}");

    room.finish().await;
}

#[actix_web::test]
async fn reversed_exchange_shows_as_reversed() {
    let room = Room::start().await;
    let cx1 = room.customer(CX1).await;

    let ans = call(&room, Some(&cx1), Method::GET, VIEW_PATH).await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "view must return 200: {}",
        ans.text
    );

    let items = get_transactions(&ans.body);

    let tx1 = items
        .iter()
        .find(|item| get_receipt_ref(item) == Some(REF_CX1_1))
        .expect("TX 1 exists");
    assert!(
        is_status_completed(tx1),
        "non-reversed TX 1 should show as COMPLETED: {tx1}"
    );

    let tx2 = items
        .iter()
        .find(|item| {
            get_receipt_ref(item).is_some_and(|r| r.starts_with(REF_CX1_2))
                || get_branch(item) == Some(B01)
        })
        .expect("TX 2 exists");
    assert!(
        is_status_reversed(tx2),
        "reversed TX 2 must show status as REVERSED: {tx2}"
    );

    room.finish().await;
}

#[actix_web::test]
async fn another_customers_rows_never_appear() {
    let room = Room::start().await;
    let cx2 = room.customer(CX2).await;

    let ans = call(&room, Some(&cx2), Method::GET, VIEW_PATH).await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "view must return 200: {}",
        ans.text
    );

    let items = get_transactions(&ans.body);
    assert_eq!(
        items.len(),
        1,
        "CX2 should see exactly 1 transaction: {items:?}"
    );

    let tx3 = &items[0];
    assert_eq!(
        get_branch(tx3),
        Some(MAIN),
        "CX2's transaction was at MAIN: {tx3}"
    );
    assert_eq!(
        get_receipt_ref(tx3),
        Some(REF_CX2_1),
        "CX2's receipt ref matches: {tx3}"
    );

    // Verify neither of CX1's transactions appear
    let has_cx1_tx = items.iter().any(|item| {
        let r = get_receipt_ref(item);
        r == Some(REF_CX1_1) || r.is_some_and(|s| s.starts_with(REF_CX1_2))
    });
    assert!(!has_cx1_tx, "CX2 must not see any rows belonging to CX1");

    room.finish().await;
}

#[actix_web::test]
async fn view_takes_no_customer_id_from_request() {
    let room = Room::start().await;
    let cx1 = room.customer(CX1).await;

    // Normal call must return 200 OK
    let own = call(&room, Some(&cx1), Method::GET, VIEW_PATH).await;
    assert_eq!(
        own.status,
        StatusCode::OK,
        "view must return 200: {}",
        own.text
    );

    // Call with ?customerId= naming another customer
    let ans_query = call(
        &room,
        Some(&cx1),
        Method::GET,
        &format!("{VIEW_PATH}?customerId={CX2}"),
    )
    .await;

    // Either the view ignores the query parameter and answers CX1's own transactions (200),
    // or it rejects the invalid parameter (400/403). It must NEVER return CX2's rows!
    if ans_query.status == StatusCode::OK {
        let items = get_transactions(&ans_query.body);
        let sees_cx2 = items.iter().any(|item| {
            get_receipt_ref(item) == Some(REF_CX2_1)
                || item.get("id").and_then(|v| v.as_str()) == Some(TX_CX2_1)
        });
        assert!(
            !sees_cx2,
            "view must never honor customerId query param to return another customer's data: {items:?}"
        );
        assert_eq!(
            items.len(),
            2,
            "must still return CX1's own rows: {items:?}"
        );
    } else {
        assert!(
            ans_query.status.is_client_error(),
            "if refused, must be 4xx client error: {}",
            ans_query.text
        );
    }

    room.finish().await;
}

#[actix_web::test]
async fn staff_session_refused_per_customer_self() {
    let room = Room::start().await;

    for role in [Role::Teller, Role::Admin, Role::BranchManager] {
        let staff = room.staff(role).await;
        let ans = call(&room, Some(&staff), Method::GET, VIEW_PATH).await;
        assert_eq!(
            ans.status,
            StatusCode::BAD_REQUEST,
            "{role:?} must be refused 400: {}",
            ans.text
        );
        assert_eq!(
            ans.code(),
            "roleNotAllowedHere",
            "{role:?} must be refused with roleNotAllowedHere"
        );
    }

    room.finish().await;
}

#[actix_web::test]
async fn unauthenticated_session_refused_per_customer_self() {
    let room = Room::start().await;

    let ans = call(&room, None, Method::GET, VIEW_PATH).await;
    assert_eq!(
        ans.status,
        StatusCode::UNAUTHORIZED,
        "unauthenticated call must return 401: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "signIn",
        "unauthenticated call must answer signIn"
    );

    room.finish().await;
}
