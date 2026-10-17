//! Pinned tests for #1003 (APP-11 rework round 2).
//!
//! Pinned by this file:
//! - Test A: Two organisations each have a receipt with the SAME number; each owner
//!   GETs /api/app/receipts/{ref} and gets their own; a customer of the other org
//!   gets 404 with a body identical to an unknown ref's.
//! - Test B: A transaction whose rate has 4 decimals (e.g. 35.1234) comes back
//!   from my_transactions with all 4 decimals preserved.

mod common;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";

const ORG1: &str = "00000000-0000-0000-0000-000000000001";
const ORG2: &str = "00000000-0000-0000-0000-000000000002";

const BRANCH_ORG1: &str = "MAIN";
const BRANCH_ORG2: &str = "B_ORG2";

const CX_ORG1: &str = "CX-PIN-ORG1";
const CX_ORG2: &str = "CX-PIN-ORG2";
const CX_OTHER_ORG2: &str = "CX-PIN-OTHER-ORG2";
const CX_RATE_4DEC: &str = "CX-PIN-RATE-4DEC";

const RC_SAME_NUMBER: &str = "RC-SAME-NUMBER-001";
const RC_ORG1_ONLY: &str = "RC-ORG1-ONLY-999";
const RC_UNKNOWN: &str = "RC-DOES-NOT-EXIST-404";

const HTML_ORG1: &str = "<!doctype html><html><body><h1>Receipt Org 1</h1></body></html>";
const HTML_ORG2: &str = "<!doctype html><html><body><h1>Receipt Org 2</h1></body></html>";
const HTML_ORG1_ONLY: &str = "<!doctype html><html><body><h1>Receipt Org 1 Only</h1></body></html>";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

// ------------------------------------------------------------ database & files

struct Room {
    pool: PgPool,
    schema: String,
    files_dir: PathBuf,
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
        let schema = unique("app_hist_pins");
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

        let files_dir = std::env::temp_dir().join(unique("xc-pins-test-files"));
        std::fs::create_dir_all(&files_dir).expect("create test files directory");

        let room = Self {
            pool,
            schema,
            files_dir,
        };
        room.setup_data().await;
        room
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn write_file(&self, key: &str, content: &[u8], mime: &str, branch: &str, org: &str) {
        let file_path = self.files_dir.join(key);
        std::fs::write(&file_path, content).expect("write receipt file to disk");

        let sha256_hex = format!("{:x}", Sha256::digest(content));
        self.exec(&format!(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at, organization_id) \
             VALUES ('{key}', '{branch}', 'test', '{mime}', '{sha256_hex}', now(), '{org}'::uuid) \
             ON CONFLICT (key) DO NOTHING"
        ))
        .await;
    }

    async fn setup_data(&self) {
        // 1. Organizations
        self.exec(&format!(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) VALUES \
             ('{ORG2}'::uuid, 'ORG2', 'Organization Two', 'Asia/Bangkok', now()) \
             ON CONFLICT (id) DO NOTHING"
        ))
        .await;

        // 2. Branches
        self.exec(&format!(
            "INSERT INTO branches (id, name, organization_id) VALUES \
             ('{BRANCH_ORG1}', 'Main Org 1', '{ORG1}'::uuid), \
             ('{BRANCH_ORG2}', 'Branch Org 2', '{ORG2}'::uuid) \
             ON CONFLICT (id) DO NOTHING"
        ))
        .await;

        // 3. Customers
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) VALUES \
             ('{CX_ORG1}', '{ORG1}'::uuid, '{CX_ORG1}', 'Customer Org 1', 'LIVE', now()), \
             ('{CX_ORG2}', '{ORG2}'::uuid, '{CX_ORG2}', 'Customer Org 2 Owner', 'LIVE', now()), \
             ('{CX_OTHER_ORG2}', '{ORG2}'::uuid, '{CX_OTHER_ORG2}', 'Customer Org 2 Other', 'LIVE', now()), \
             ('{CX_RATE_4DEC}', '{ORG1}'::uuid, '{CX_RATE_4DEC}', 'Customer Rate 4Dec', 'LIVE', now())"
        ))
        .await;

        // 4. Shifts
        let u1 = self.user(ORG1).await;
        let u2 = self.user(ORG2).await;
        self.exec(&format!(
            "INSERT INTO shifts (id, organization_id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) VALUES \
             ('SH-ORG1', '{ORG1}'::uuid, '{BRANCH_ORG1}', 'OPEN', now(), 'test', 'req-open-org1', '{u1}'::uuid), \
             ('SH-ORG2', '{ORG2}'::uuid, '{BRANCH_ORG2}', 'OPEN', now(), 'test', 'req-open-org2', '{u2}'::uuid)"
        ))
        .await;

        let now = "2026-10-12T10:00:00Z";

        // 5. Files
        let f_org1 = format!("file-{}", unique("org1"));
        let f_org2 = format!("file-{}", unique("org2"));
        let f_org1_only = format!("file-{}", unique("org1_only"));

        self.write_file(
            &f_org1,
            HTML_ORG1.as_bytes(),
            "text/html; charset=utf-8",
            BRANCH_ORG1,
            ORG1,
        )
        .await;
        self.write_file(
            &f_org2,
            HTML_ORG2.as_bytes(),
            "text/html; charset=utf-8",
            BRANCH_ORG2,
            ORG2,
        )
        .await;
        self.write_file(
            &f_org1_only,
            HTML_ORG1_ONLY.as_bytes(),
            "text/html; charset=utf-8",
            BRANCH_ORG1,
            ORG1,
        )
        .await;

        // 6. Test A transactions and receipts
        // Tx1 in ORG1 for CX_ORG1
        let tx1 = uuid::Uuid::new_v4().to_string();
        self.insert_tx(
            &tx1,
            ORG1,
            BRANCH_ORG1,
            CX_ORG1,
            "SH-ORG1",
            "DOC-PINS-1",
            33.5,
            now,
        )
        .await;
        self.insert_receipt(&tx1, ORG1, RC_SAME_NUMBER, &f_org1, 1, "RECEIPT", now)
            .await;

        // Tx in ORG1 for CX_ORG1 (solo receipt)
        let tx_solo = uuid::Uuid::new_v4().to_string();
        self.insert_tx(
            &tx_solo,
            ORG1,
            BRANCH_ORG1,
            CX_ORG1,
            "SH-ORG1",
            "DOC-PINS-SOLO",
            33.5,
            now,
        )
        .await;
        self.insert_receipt(
            &tx_solo,
            ORG1,
            RC_ORG1_ONLY,
            &f_org1_only,
            1,
            "RECEIPT",
            now,
        )
        .await;

        // Tx2 in ORG2 for CX_ORG2 with the EXACT SAME receipt number RC_SAME_NUMBER
        let tx2 = uuid::Uuid::new_v4().to_string();
        self.insert_tx(
            &tx2,
            ORG2,
            BRANCH_ORG2,
            CX_ORG2,
            "SH-ORG2",
            "DOC-PINS-2",
            33.5,
            now,
        )
        .await;
        self.insert_receipt(&tx2, ORG2, RC_SAME_NUMBER, &f_org2, 1, "RECEIPT", now)
            .await;

        // 7. Test B transaction with 4-decimal rate (35.1234)
        let tx_b = uuid::Uuid::new_v4().to_string();
        let f_b = format!("file-{}", unique("b"));
        self.write_file(&f_b, b"test b receipt", "text/plain", BRANCH_ORG1, ORG1)
            .await;
        self.insert_tx(
            &tx_b,
            ORG1,
            BRANCH_ORG1,
            CX_RATE_4DEC,
            "SH-ORG1",
            "DOC-PINS-B",
            35.1234,
            now,
        )
        .await;
        self.insert_receipt(&tx_b, ORG1, "RC-RATE-4DEC-001", &f_b, 1, "RECEIPT", now)
            .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_tx(
        &self,
        tx_id: &str,
        org: &str,
        branch: &str,
        customer_id: &str,
        shift_id: &str,
        doc_ref: &str,
        rate: f64,
        now: &str,
    ) {
        let amount = 100.0;
        let settle_thb = amount * rate;
        self.exec(&format!(
            "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, customer_id, \
                customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
                recorded_by, status, completed_at, organization_id) \
             VALUES ('{tx_id}', '{doc_ref}', '{branch}', 'LIVE', '{shift_id}', 'sub-{tx_id}', 'hash-{tx_id}', \
                '{customer_id}', 'Customer {customer_id}', 'THA', 'teller-1', '', '', '2026-10-12', \
                '{now}', '{now}', 'teller-1', 'COMPLETED', '{now}', '{org}'::uuid)"
        ))
        .await;

        self.exec(&format!(
            "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
                quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
             VALUES (gen_random_uuid(), '{org}'::uuid, '{tx_id}', 'BUY', 'USD', {amount}, 1, {rate}, \
                {settle_thb}, 0, {settle_thb}, 1, '{now}', '{now}')"
        ))
        .await;

        self.exec(&format!(
            "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, method, \
                reference, account, amount_thb, bank_fee_thb, paid_at, created_at, updated_at) \
             VALUES (gen_random_uuid(), '{org}'::uuid, '{tx_id}', 1, 'Cash', '', 'drawer', {settle_thb}, 0, \
                '{now}', '{now}', '{now}')"
        ))
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_receipt(
        &self,
        tx_id: &str,
        org: &str,
        number: &str,
        file_key: &str,
        revision: i32,
        kind: &str,
        now: &str,
    ) {
        self.exec(&format!(
            "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
                document_ref, kind, created_at, updated_at) \
             VALUES (gen_random_uuid(), '{org}'::uuid, '{tx_id}', '{number}', {revision}, '{now}', \
                '{file_key}', '{kind}', '{now}', '{now}')"
        ))
        .await;
    }

    async fn user(&self, org_id: &str) -> String {
        sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(org_id)
        .bind(unique("u"))
        .fetch_one(&self.pool)
        .await
        .expect("insert a user")
    }

    async fn customer(&self, customer_id: &str, org_id: &str) -> Who {
        let id = self.user(org_id).await;
        Who(format!("{id}|CUSTOMER|{customer_id}|{org_id}"))
    }

    async fn finish(self) {
        let _ = std::fs::remove_dir_all(&self.files_dir);
        let Room { pool, schema, .. } = self;
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
    let parts: Vec<&str> = spec.split('|').collect();
    if parts.len() < 4 {
        return None;
    }
    let id = parts[0];
    let role = Role::parse(parts[1])?;
    let customer = parts[2];
    let org = parts[3];
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        org,
        vec![ScopedRole::new(role, None::<BranchId>, from, None)],
        (!customer.is_empty()).then_some(customer),
    ))
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                owner: None,
                staff: None,
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

async fn get_receipt(room: &Room, who: &Who, receipt_ref: &str) -> Answer {
    let settings = common::settings(&[("APP_FILES__DIR", room.files_dir.to_str().unwrap())]);
    let state = AppState::new(Arc::new(settings), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let req = atest::TestRequest::default()
        .method(Method::GET)
        .uri(&format!("/api/app/receipts/{receipt_ref}"))
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who.0.clone()));

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    Answer { status, text, body }
}

async fn get_my_transactions(room: &Room, who: &Who) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let req = atest::TestRequest::default()
        .method(Method::GET)
        .uri("/api/v3/views/my_transactions")
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who.0.clone()));

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    Answer { status, text, body }
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
    panic!("cannot find transactions array in: {val}");
}

// ------------------------------------------------------------ tests

/// Test A: Two organisations each have a receipt with the SAME number.
/// Each owner GETs /api/app/receipts/{ref} and gets their own.
/// A customer of the other org gets 404 with a body identical to an unknown ref's.
#[actix_web::test]
async fn test_a_two_organisations_same_receipt_number_isolated() {
    let room = Room::start().await;

    let cx_org1 = room.customer(CX_ORG1, ORG1).await;
    let cx_org2 = room.customer(CX_ORG2, ORG2).await;
    let cx_other_org2 = room.customer(CX_OTHER_ORG2, ORG2).await;

    // 1. Owner in Org 1 gets their own receipt file (200 OK)
    let ans1 = get_receipt(&room, &cx_org1, RC_SAME_NUMBER).await;
    assert_eq!(
        ans1.status,
        StatusCode::OK,
        "Org 1 customer must receive their own receipt with 200 OK: {}",
        ans1.text
    );
    assert_eq!(
        ans1.text, HTML_ORG1,
        "Org 1 customer must stream Org 1 receipt content"
    );

    // 2. Owner in Org 2 gets their own receipt file (200 OK) with the identical number
    let ans2 = get_receipt(&room, &cx_org2, RC_SAME_NUMBER).await;
    assert_eq!(
        ans2.status,
        StatusCode::OK,
        "Org 2 customer must receive their own receipt with 200 OK: {}",
        ans2.text
    );
    assert_eq!(
        ans2.text, HTML_ORG2,
        "Org 2 customer must stream Org 2 receipt content"
    );

    // 3. Customer of Org 2 who does not own the receipt gets 404
    let ans_other = get_receipt(&room, &cx_other_org2, RC_SAME_NUMBER).await;
    assert_eq!(
        ans_other.status,
        StatusCode::NOT_FOUND,
        "customer not owning the receipt must get 404: {}",
        ans_other.text
    );

    // 4. Unknown ref also gets 404
    let ans_unknown = get_receipt(&room, &cx_other_org2, RC_UNKNOWN).await;
    assert_eq!(
        ans_unknown.status,
        StatusCode::NOT_FOUND,
        "unknown ref must get 404: {}",
        ans_unknown.text
    );

    // 5. Body must be identical between non-owner request and unknown ref request
    assert_eq!(
        ans_other.text, ans_unknown.text,
        "404 body for unowned ref and unknown ref must be byte-identical"
    );

    // 6. Org 2 customer requesting an Org 1-only receipt gets 404 identical to unknown ref
    let ans_cross_org = get_receipt(&room, &cx_org2, RC_ORG1_ONLY).await;
    assert_eq!(
        ans_cross_org.status,
        StatusCode::NOT_FOUND,
        "cross-org ref lookup must answer 404"
    );
    let ans_cross_unknown = get_receipt(&room, &cx_org2, RC_UNKNOWN).await;
    assert_eq!(
        ans_cross_org.text, ans_cross_unknown.text,
        "cross-org 404 body must be byte-identical to unknown ref"
    );

    room.finish().await;
}

/// Test B: A transaction whose rate has 4 decimals (e.g. 35.1234) comes back
/// from my_transactions with all 4 decimals.
#[actix_web::test]
async fn test_b_transaction_rate_4_decimals_preserved() {
    let room = Room::start().await;

    let cx_b = room.customer(CX_RATE_4DEC, ORG1).await;

    let ans = get_my_transactions(&room, &cx_b).await;
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
        "CX_RATE_4DEC should have exactly 1 transaction: {items:?}"
    );

    let tx = &items[0];
    let rate_str = tx["rate"]
        .as_str()
        .expect("rate must be present and a string");

    assert!(
        rate_str.starts_with("35.1234"),
        "rate with 4 decimals must preserve all 4 decimals, got: {rate_str}"
    );
    assert_eq!(
        rate_str.trim_end_matches('0'),
        "35.1234",
        "rate must preserve 4 decimal digits, got: {rate_str}"
    );
    assert_ne!(
        rate_str, "35.12",
        "rate must not be truncated to 2 decimals"
    );

    room.finish().await;
}
