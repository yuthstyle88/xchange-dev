//! Ticket #4985 (P8): cash transfer note (create and reconcile) and branch profile code and address
//! take bounded lengths.
//!
//! Behaviour rows (#4985 §1, §3; one test per row: max accepted, max+1 refused with the field):
//! 1. Transfer note: 0..200, field `note` (`POST /api/cash-transfers`)
//! 2. Reconcile note: 0..200, field `note` (`POST /api/cash-transfers/{id}/reconcile`)
//! 3. Branch code: 1..40, field `code` (`PUT /api/branches/profile`)
//! 4. Branch address: 0..200, field `address` (`PUT /api/branches/profile`)

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_approval::domain::request::Outcome;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::posting::CashPlace;
use xc_kernel::ids::BranchId;
use xc_testdb::people::{RoleRow, seed_person};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const ORG: &str = "00000000-0000-0000-0000-000000000001";

static FUND: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn accepted(a: &Answer, what: &str) {
    assert_eq!(
        a.status,
        StatusCode::OK,
        "{what} must be accepted: {}",
        a.text
    );
}

fn refused_on(a: &Answer, field: &str, what: &str) {
    assert!(
        a.status == StatusCode::BAD_REQUEST
            && a.body["error"]["code"] == "validationFailed"
            && a.body["error"]["field"] == field,
        "{what}: expected 400 validationFailed on {field:?}; got {} {}",
        a.status,
        a.text
    );
}

fn chars(n: usize) -> String {
    "a".repeat(n)
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
        let schema = unique("cashbrbounds");
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
        for branch in [B01, B09] {
            sqlx::query(
                "INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)",
            )
            .bind(branch)
            .bind(ORG)
            .execute(&pool)
            .await
            .expect("insert a branch");
            sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
                .bind(branch)
                .execute(&pool)
                .await
                .expect("the branch's ops_state");
            let safe = || CashPlace::Safe(BranchId::new(branch));
            lg4_fund(&pool, CashPlace::Bank, safe(), "USD", "500.00").await;
            lg4_fund(&pool, CashPlace::Bank, safe(), "THB", "1000000.00").await;
        }
        Self { pool, schema }
    }

    async fn person(&self, role: Role, branch: Option<&str>) -> Who {
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let id = seed_person(
            &mut self.pool.acquire().await.unwrap(),
            ORG,
            &unique(role.as_str()),
            &[RoleRow {
                role_code: role.as_str(),
                branch_id: branch,
                valid_from: from,
                valid_to: None,
            }],
        )
        .await
        .expect("insert a user");
        Who {
            spec: format!("{id}|{}|{}", role.as_str(), branch.unwrap_or("*")),
            id,
        }
    }

    async fn denomination(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the {currency} {face} face value: {e}"))
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

async fn lg4_fund(pool: &PgPool, from: CashPlace, to: CashPlace, currency: &str, amount: &str) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPostings, PostingRequest};
    let mut tx = pool.begin().await.expect("begin");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from,
                to,
                currency: xc_kernel::money::CurrencyCode::parse(currency).expect("a currency"),
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: format!(
                        "lg4-fund:{currency}:{}",
                        FUND.fetch_add(1, Ordering::Relaxed)
                    ),
                    source_key: "fund".to_string(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("lg4 fund");
    tx.commit().await.expect("commit");
}

struct Who {
    id: String,
    spec: String,
}

fn user_of(spec: &str) -> Option<User> {
    if spec.contains('|') {
        let mut parts = spec.splitn(4, '|');
        let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
        let org = parts.next().unwrap_or(ORG);
        let role = Role::parse(role)?;
        let branch = (branch != "*").then(|| BranchId::new(branch));
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Some(User::new(
            id,
            org,
            vec![ScopedRole::new(role, branch, from, None)],
            None,
        ))
    } else {
        let (role, branch) = spec.split_once('@')?;
        let role = Role::parse(role)?;
        let branch = (branch != "*").then(|| BranchId::new(branch));
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Some(User::new(
            &format!("user-{}", spec.replace(['@', '*'], "-")),
            ORG,
            vec![ScopedRole::new(role, branch, from, None)],
            None,
        ))
    }
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move { test_callers::callers(None, user_of(&spec)) })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn send(db: &TestDb, who: &str, method: Method, uri: &str, body: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who))
        .insert_header(("x-real-ip", "203.0.113.30"))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn id_of(a: &Answer, what: &str) -> String {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    a.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{what} answers its record's id: {}", a.text))
        .to_owned()
}

async fn received_transfer(db: &TestDb) -> String {
    let creator = db.person(Role::BranchManager, Some(B01)).await;
    let approver = db.person(Role::BranchManager, Some(B01)).await;
    let receiver = db.person(Role::BranchManager, Some(B09)).await;
    let executive = db.person(Role::Executive, None).await;
    let usd50 = db.denomination("USD", "50").await;
    sqlx::query(
        "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by) \
         VALUES ('v1', $1::uuid, jsonb_build_object('USD', '300.00'), $2::uuid)",
    )
    .bind(ORG)
    .bind(&executive.id)
    .execute(&db.pool)
    .await
    .expect("a transfer policy version");
    let a = send(
        db,
        &creator.spec,
        Method::POST,
        "/api/cash-transfers",
        json!({
            "branch": B01,
            "kind": "BRANCH",
            "toBranchId": B09,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "2" }],
            "note": "to B09"
        }),
    )
    .await;
    let id = id_of(&a, "transferCreate");
    send(
        db,
        &creator.spec,
        Method::POST,
        &format!("/api/cash-transfers/{id}/submit"),
        json!({ "branch": B01 }),
    )
    .await;
    let approver_user = user_of(&approver.spec).expect("the approver");
    TransferFlow::pg(db.pool.clone())
        .decide(&approver_user, &id, Outcome::Approved, "ok", Utc::now())
        .await
        .expect("a branch manager of the source approves within the limit");
    let a = send(
        db,
        &creator.spec,
        Method::POST,
        &format!("/api/cash-transfers/{id}/dispatch"),
        json!({ "branch": B01 }),
    )
    .await;
    assert_eq!(a.body["status"], "DISPATCHED", "{}", a.text);
    let a = send(
        db,
        &receiver.spec,
        Method::POST,
        &format!("/api/cash-transfers/{id}/receive"),
        json!({
            "branch": B09,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qtyReceived": "1" }]
        }),
    )
    .await;
    assert_eq!(a.body["status"], "RECEIVED", "{}", a.text);
    id
}

/// Row 1: Transfer note: 0..200, field `note` (`POST /api/cash-transfers`)
#[actix_web::test]
async fn test_cash_transfer_note_bound() {
    let db = TestDb::start().await;
    let manager = db.person(Role::BranchManager, Some(B01)).await;
    let usd50 = db.denomination("USD", "50").await;

    // max+1 refused (201 chars)
    let res = send(
        &db,
        &manager.spec,
        Method::POST,
        "/api/cash-transfers",
        json!({
            "branch": B01,
            "kind": "BRANCH",
            "toBranchId": B09,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "1" }],
            "note": chars(201)
        }),
    )
    .await;
    refused_on(&res, "note", "transfer note at 201 chars");

    // max accepted (200 chars)
    let res = send(
        &db,
        &manager.spec,
        Method::POST,
        "/api/cash-transfers",
        json!({
            "branch": B01,
            "kind": "BRANCH",
            "toBranchId": B09,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "1" }],
            "note": chars(200)
        }),
    )
    .await;
    accepted(&res, "transfer note at 200 chars");

    db.finish().await;
}

/// Row 2: Reconcile note: 0..200, field `note` (`POST /api/cash-transfers/{id}/reconcile`)
#[actix_web::test]
async fn test_cash_transfer_reconcile_note_bound() {
    let db = TestDb::start().await;
    let id = received_transfer(&db).await;
    let finance = db.person(Role::Finance, Some(B01)).await;

    // max+1 refused (201 chars)
    let res = send(
        &db,
        &finance.spec,
        Method::POST,
        &format!("/api/cash-transfers/{id}/reconcile"),
        json!({
            "branch": B01,
            "note": chars(201)
        }),
    )
    .await;
    refused_on(&res, "note", "reconcile note at 201 chars");

    // max accepted (200 chars)
    let res = send(
        &db,
        &finance.spec,
        Method::POST,
        &format!("/api/cash-transfers/{id}/reconcile"),
        json!({
            "branch": B01,
            "note": chars(200)
        }),
    )
    .await;
    accepted(&res, "reconcile note at 200 chars");

    db.finish().await;
}

/// Row 3: Branch code: 1..40, field `code` (`PUT /api/branches/profile`)
#[actix_web::test]
async fn test_branch_code_bound() {
    let db = TestDb::start().await;
    let admin = db.person(Role::Admin, None).await;

    // max+1 refused (41 chars)
    let res = send(
        &db,
        &admin.spec,
        Method::PUT,
        "/api/branches/profile",
        json!({
            "branch": B01,
            "code": chars(41),
            "address": "1 Silom Rd",
            "timezone": "Asia/Bangkok",
            "openingHours": { "mon": [{ "open": "09:00", "close": "17:00" }] },
            "bookingEnabled": true
        }),
    )
    .await;
    refused_on(&res, "code", "branch code at 41 chars");

    // max accepted (40 chars)
    let res = send(
        &db,
        &admin.spec,
        Method::PUT,
        "/api/branches/profile",
        json!({
            "branch": B01,
            "code": chars(40),
            "address": "1 Silom Rd",
            "timezone": "Asia/Bangkok",
            "openingHours": { "mon": [{ "open": "09:00", "close": "17:00" }] },
            "bookingEnabled": true
        }),
    )
    .await;
    accepted(&res, "branch code at 40 chars");

    db.finish().await;
}

/// Row 4: Branch address: 0..200, field `address` (`PUT /api/branches/profile`)
#[actix_web::test]
async fn test_branch_address_bound() {
    let db = TestDb::start().await;
    let admin = db.person(Role::Admin, None).await;

    // max+1 refused (201 chars)
    let res = send(
        &db,
        &admin.spec,
        Method::PUT,
        "/api/branches/profile",
        json!({
            "branch": B01,
            "code": "B01",
            "address": chars(201),
            "timezone": "Asia/Bangkok",
            "openingHours": { "mon": [{ "open": "09:00", "close": "17:00" }] },
            "bookingEnabled": true
        }),
    )
    .await;
    refused_on(&res, "address", "branch address at 201 chars");

    // max accepted (200 chars)
    let res = send(
        &db,
        &admin.spec,
        Method::PUT,
        "/api/branches/profile",
        json!({
            "branch": B01,
            "code": "B01",
            "address": chars(200),
            "timezone": "Asia/Bangkok",
            "openingHours": { "mon": [{ "open": "09:00", "close": "17:00" }] },
            "bookingEnabled": true
        }),
    )
    .await;
    accepted(&res, "branch address at 200 chars");

    db.finish().await;
}
