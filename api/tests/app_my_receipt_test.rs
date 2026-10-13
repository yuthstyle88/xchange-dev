//! Ticket #1003 (APP-11, folded in #1012): customer receipt route over HTTP.
//! Design `docs/design/blueprint/customer-app.md` §5.1, §8, §9.1.
//!
//! Written before the implementation by the test-writer gemini/pool-4, on the
//! FE lane controller claude/5edf7bb5's order. Do not edit, delete or weaken.
//!
//! **Pinned by this file** (rulings on #1003 and #1012):
//! - `GET /api/app/receipts/{ref}` streams the reader's own receipt file (status 200, matching content).
//! - Another customer's ref answers 404, not 403, so the response never confirms that the ref exists.
//! - An unknown/non-existent ref answers 404 with identical body shape to another customer's ref.
//! - No session is refused (401 `signIn`).
//! - Staff session or unlinked actor is refused per `CustomerSelf` (400 `roleNotAllowedHere` or 403 `outOfScope`).
//! - A CUSTOMER never needs `FileRead` (the app route streams it directly).
//! - A reversed transaction's receipt is still accessible to the customer (200).
//!
//! **Red on `main` by result:** route does not exist on `main`, returning 404.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

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

const MAIN: &str = "MAIN";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const CX1: &str = "CX-RCPT-1";
const CX2: &str = "CX-RCPT-2";

const REF_CX1_COMPLETED: &str = "RC-CX1-COMPLETED";
const REF_CX1_REVERSED: &str = "RC-CX1-REVERSED";
const REF_CX2_COMPLETED: &str = "RC-CX2-COMPLETED";
const REF_UNKNOWN: &str = "RC-DOES-NOT-EXIST-404";

const HTML_CX1_COMPLETED: &str =
    "<!doctype html><html><body><h1>Receipt CX1 Completed</h1></body></html>";
const HTML_CX1_REVERSED: &str =
    "<!doctype html><html><body><h1>Receipt CX1 Reversed</h1></body></html>";
const HTML_CX2_COMPLETED: &str =
    "<!doctype html><html><body><h1>Receipt CX2 Completed</h1></body></html>";

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
        let schema = unique("my_rcpt");
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

        let files_dir = std::env::temp_dir().join(unique("xc-receipt-test-files"));
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

    async fn write_file(&self, key: &str, content: &[u8], mime: &str) {
        let file_path = self.files_dir.join(key);
        std::fs::write(&file_path, content).expect("write receipt file to disk");

        let sha256_hex = format!("{:x}", Sha256::digest(content));
        self.exec(&format!(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ('{key}', '{MAIN}', 'test', '{mime}', '{sha256_hex}', now()) \
             ON CONFLICT (key) DO NOTHING"
        ))
        .await;
    }

    async fn setup_data(&self) {
        // Branch
        self.exec(
            "INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN') ON CONFLICT (id) DO NOTHING",
        )
        .await;

        // Customers
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) VALUES \
             ('{CX1}', '{ORG}', '{CX1}', 'Customer 1', 'LIVE', now()), \
             ('{CX2}', '{ORG}', '{CX2}', 'Customer 2', 'LIVE', now())"
        ))
        .await;

        // Shift
        self.exec(
            "WITH u AS (INSERT INTO users (organization_id, name, status, created_by) \
                 VALUES ('00000000-0000-0000-0000-000000000001'::uuid, 'Teller MAIN', 'ACTIVE', 'test') \
                 RETURNING id) \
             INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             SELECT 'SH-MAIN', 'MAIN', 'OPEN', now(), 'test', 'req-open-main', u.id FROM u"
        )
        .await;

        // Write files to disk and DB
        let file_key_cx1_completed = format!("receipt-{}", unique("f1"));
        let file_key_cx1_reversed = format!("receipt-{}", unique("f2"));
        let file_key_cx2_completed = format!("receipt-{}", unique("f3"));

        self.write_file(
            &file_key_cx1_completed,
            HTML_CX1_COMPLETED.as_bytes(),
            "text/html; charset=utf-8",
        )
        .await;
        self.write_file(
            &file_key_cx1_reversed,
            HTML_CX1_REVERSED.as_bytes(),
            "text/html; charset=utf-8",
        )
        .await;
        self.write_file(
            &file_key_cx2_completed,
            HTML_CX2_COMPLETED.as_bytes(),
            "text/html; charset=utf-8",
        )
        .await;

        let now = "2026-10-12T10:00:00Z";

        // 1. CX1 - Completed exchange
        let tx1 = uuid::Uuid::new_v4().to_string();
        self.insert_tx(&tx1, CX1, "DOC-CX1-1", now).await;
        self.insert_receipt(
            &tx1,
            REF_CX1_COMPLETED,
            &file_key_cx1_completed,
            1,
            "RECEIPT",
            now,
        )
        .await;

        // 2. CX1 - Reversed exchange
        let tx2 = uuid::Uuid::new_v4().to_string();
        self.insert_tx(&tx2, CX1, "DOC-CX1-2", now).await;
        let rev_time = "2026-10-12T11:00:00Z";
        let receipt_rev_id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
                document_ref, kind, created_at, updated_at) \
             VALUES ('{receipt_rev_id}', '{ORG}', '{tx2}', '{REF_CX1_REVERSED}', 2, '{rev_time}', \
                '{file_key_cx1_reversed}', 'REVERSAL', '{rev_time}', '{rev_time}')"
        ))
        .await;

        let req_id = uuid::Uuid::new_v4().to_string();
        let post_id = uuid::Uuid::new_v4().to_string();
        let rev_id = uuid::Uuid::new_v4().to_string();

        self.exec(&format!(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
                object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
                status, idempotency_key) \
             VALUES ('{req_id}', '{ORG}', 'TRANSACTION_REVERSAL', '{MAIN}', '{rev_id}', 'h', '1', \
                '{{}}', 'teller-1', '{rev_time}', 'APPROVED', 'k-{req_id}')"
        ))
        .await;

        self.exec(&format!(
            "INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, source_key, \
                status, posted_at, posted_by) \
             VALUES ('{post_id}', '{ORG}', '{MAIN}', 'REVERSAL', '{rev_id}', 'reversal', 'POSTED', \
                '{rev_time}', 'teller-1')"
        ))
        .await;

        self.exec(&format!(
            "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, \
                reason, amount_thb, business_date, approval_request_id, posting_id, receipt_id, \
                requested_by, requested_at, posted_by, posted_at, created_at, updated_at) \
             VALUES ('{rev_id}', '{ORG}', '{MAIN}', '{tx2}', 'POSTED', 'Customer request', \
                1000.0, '2026-10-12', '{req_id}', '{post_id}', '{receipt_rev_id}', 'teller-1', \
                '{rev_time}', 'teller-1', '{rev_time}', '{rev_time}', '{rev_time}')"
        ))
        .await;

        // 3. CX2 - Completed exchange
        let tx3 = uuid::Uuid::new_v4().to_string();
        self.insert_tx(&tx3, CX2, "DOC-CX2-1", now).await;
        self.insert_receipt(
            &tx3,
            REF_CX2_COMPLETED,
            &file_key_cx2_completed,
            1,
            "RECEIPT",
            now,
        )
        .await;
    }

    async fn insert_tx(&self, tx_id: &str, customer_id: &str, doc_ref: &str, now: &str) {
        self.exec(&format!(
            "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
                customer_id, customer_name, nationality, type, code, amount, rate, principal, fee, bank_fee, \
                received, paid, method, payment_ref, account, staff_id, booking_no, notes, value_date, paid_at, \
                recorded_at, recorded_by, status, completed_at, organization_id) \
             VALUES ('{tx_id}', '{doc_ref}', '{MAIN}', 'LIVE', 'SH-MAIN', 'sub-{tx_id}', 'hash-{tx_id}', \
                '{customer_id}', 'Customer {customer_id}', 'THA', 'BUY', 'USD', 100, 33.5, 3350, \
                0, 0, 0, 3350, 'Cash', '', 'drawer', 'teller-1', '', '', '2026-10-12', '{now}', \
                '{now}', 'teller-1', 'COMPLETED', '{now}', '{ORG}')"
        ))
        .await;

        self.exec(&format!(
            "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
                quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
             VALUES (gen_random_uuid(), '{ORG}', '{tx_id}', 'BUY', 'USD', 100, 1, 33.5, \
                3350, 0, 3350, 1, '{now}', '{now}')"
        ))
        .await;

        self.exec(&format!(
            "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, method, \
                reference, account, amount_thb, bank_fee_thb, paid_at, created_at, updated_at) \
             VALUES (gen_random_uuid(), '{ORG}', '{tx_id}', 1, 'Cash', '', 'drawer', 3350, 0, \
                '{now}', '{now}', '{now}')"
        ))
        .await;
    }

    async fn insert_receipt(
        &self,
        tx_id: &str,
        number: &str,
        file_key: &str,
        revision: i32,
        kind: &str,
        now: &str,
    ) {
        self.exec(&format!(
            "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
                document_ref, kind, created_at, updated_at) \
             VALUES (gen_random_uuid(), '{ORG}', '{tx_id}', '{number}', {revision}, '{now}', \
                '{file_key}', '{kind}', '{now}', '{now}')"
        ))
        .await;
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
    content_type: String,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> String {
        self.body["error"]["code"].as_str().unwrap_or("").to_owned()
    }
}

async fn get_receipt(room: &Room, who: Option<&Who>, receipt_ref: &str) -> Answer {
    let settings = common::settings(&[("APP_FILES__DIR", room.files_dir.to_str().unwrap())]);
    let state = AppState::new(Arc::new(settings), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let mut req = atest::TestRequest::default()
        .method(Method::GET)
        .uri(&format!("/api/app/receipts/{receipt_ref}"))
        .insert_header((header::HOST, HOST));
    if let Some(w) = who {
        req = req.insert_header((AS, w.0.clone()));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    Answer {
        status,
        content_type,
        text,
        body,
    }
}

// ------------------------------------------------------------ tests

#[actix_web::test]
async fn reader_streams_own_receipt_file() {
    let room = Room::start().await;
    let cx1 = room.customer(CX1).await;

    let ans = get_receipt(&room, Some(&cx1), REF_CX1_COMPLETED).await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "own receipt must return 200: {}",
        ans.text
    );
    assert!(
        ans.content_type.contains("text/html"),
        "content type must be html, got: {}",
        ans.content_type
    );
    assert_eq!(
        ans.text, HTML_CX1_COMPLETED,
        "streamed content must match the stored file"
    );

    room.finish().await;
}

#[actix_web::test]
async fn another_customers_receipt_answers_404_not_403() {
    let room = Room::start().await;
    let cx1 = room.customer(CX1).await;

    // First assert own receipt returns 200 OK
    let own = get_receipt(&room, Some(&cx1), REF_CX1_COMPLETED).await;
    assert_eq!(
        own.status,
        StatusCode::OK,
        "own receipt must be 200: {}",
        own.text
    );

    // CX1 asks for CX2's receipt ref
    let ans = get_receipt(&room, Some(&cx1), REF_CX2_COMPLETED).await;
    assert_eq!(
        ans.status,
        StatusCode::NOT_FOUND,
        "another customer's ref must return 404, not 403: status {} text {}",
        ans.status,
        ans.text
    );
    assert_ne!(
        ans.status,
        StatusCode::FORBIDDEN,
        "another customer's ref must NEVER return 403"
    );

    room.finish().await;
}

#[actix_web::test]
async fn unknown_ref_answers_404_with_identical_body_shape() {
    let room = Room::start().await;
    let cx1 = room.customer(CX1).await;

    // First assert own receipt returns 200 OK
    let own = get_receipt(&room, Some(&cx1), REF_CX1_COMPLETED).await;
    assert_eq!(
        own.status,
        StatusCode::OK,
        "own receipt must be 200: {}",
        own.text
    );

    let other_cx = get_receipt(&room, Some(&cx1), REF_CX2_COMPLETED).await;
    let unknown = get_receipt(&room, Some(&cx1), REF_UNKNOWN).await;

    assert_eq!(
        other_cx.status,
        StatusCode::NOT_FOUND,
        "other customer ref must be 404"
    );
    assert_eq!(
        unknown.status,
        StatusCode::NOT_FOUND,
        "unknown ref must be 404"
    );

    // The wire shape must be identical so no timing/presence oracle exists
    assert_eq!(
        other_cx.text, unknown.text,
        "another customer's ref and unknown ref must return identical body shape"
    );

    room.finish().await;
}

#[actix_web::test]
async fn unauthenticated_session_refused() {
    let room = Room::start().await;

    let ans = get_receipt(&room, None, REF_CX1_COMPLETED).await;
    assert_eq!(
        ans.status,
        StatusCode::UNAUTHORIZED,
        "unauthenticated request must return 401: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "signIn",
        "unauthenticated request must answer signIn"
    );

    room.finish().await;
}

#[actix_web::test]
async fn staff_session_refused_per_customer_self() {
    let room = Room::start().await;

    for role in [Role::Teller, Role::Admin, Role::BranchManager] {
        let staff = room.staff(role).await;
        let ans = get_receipt(&room, Some(&staff), REF_CX1_COMPLETED).await;
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
async fn customer_never_needs_file_read_permission() {
    let room = Room::start().await;
    let cx1 = room.customer(CX1).await;

    // Customer role possesses ONLY CustomerSelf and SignedIn — NO FileRead permission.
    // Yet GET /api/app/receipts/{ref} must succeed (200 OK).
    let ans = get_receipt(&room, Some(&cx1), REF_CX1_COMPLETED).await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "customer streams own receipt directly without FileRead permission: {}",
        ans.text
    );
    assert_eq!(ans.text, HTML_CX1_COMPLETED);

    room.finish().await;
}

#[actix_web::test]
async fn reversed_transaction_receipt_still_accessible() {
    let room = Room::start().await;
    let cx1 = room.customer(CX1).await;

    // Even though transaction was reversed, the customer can still stream its receipt
    let ans = get_receipt(&room, Some(&cx1), REF_CX1_REVERSED).await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "reversed transaction receipt must still be accessible (200): {}",
        ans.text
    );
    assert!(
        ans.content_type.contains("text/html"),
        "content type must be html"
    );
    assert_eq!(
        ans.text, HTML_CX1_REVERSED,
        "streamed content must match the reversed receipt file"
    );

    room.finish().await;
}
