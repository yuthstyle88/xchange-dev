//! Common test harness for BK-6 (#1388) specification tests.
//!
//! Provides throwaway database isolation via per-test PostgreSQL schemas,
//! session readers for Customer, Teller, Manager, Admin, and Owner actors,
//! and seeding helpers for branches, slot rules, slots, and bookings.

#![allow(dead_code)]

#[path = "../common/mod.rs"]
pub mod test_common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

pub const MUST_FINISH: Duration = Duration::from_secs(30);
pub const AS: &str = "x-test-as";
pub const HOST: &str = "xc.test";
pub const SAME_ORIGIN: &str = "http://xc.test";
pub const B01: &str = "XC-B01";
pub const ORG: &str = "00000000-0000-0000-0000-000000000001";
pub const TELLER_ID: &str = "00000000-0000-0000-0000-000000000010";
pub const MANAGER_ID: &str = "00000000-0000-0000-0000-000000000020";
pub const ADMIN_ID: &str = "00000000-0000-0000-0000-000000000025";
pub const FINANCE_ID: &str = "00000000-0000-0000-0000-000000000030";
pub const CUSTOMER_ALICE_ID: &str = "00000000-0000-0000-0000-000000000040";
pub const CUSTOMER_ALICE_CUST_ID: &str = "CUST-ALICE-1";
pub const CUSTOMER_BOB_ID: &str = "00000000-0000-0000-0000-000000000050";
pub const CUSTOMER_BOB_CUST_ID: &str = "CUST-BOB-1";

static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

pub fn bangkok_today() -> chrono::NaiveDate {
    Utc::now()
        .with_timezone(&chrono_tz::Asia::Bangkok)
        .date_naive()
}

// ------------------------------------------------------------ database

pub struct TestDb {
    pub pool: PgPool,
    pub schema: String,
}

impl TestDb {
    pub async fn start() -> Self {
        let keys =
            xc_kernel::pii::PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
                .unwrap();
        let _ = xc_customer::install_pii_keys(std::sync::Arc::new(keys));

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
        let schema = unique("bk6test");
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
            "INSERT INTO branches (id, name, organization_id, booking_enabled) VALUES ($1, 'Branch 1', $2::uuid, true)",
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
               ($5::uuid, $6::uuid, 'Customer Bob', 'ACTIVE', 'test'), \
               ('00000000-0000-0000-0000-000000000025'::uuid, $6::uuid, 'Admin 1', 'ACTIVE', 'test')",
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

        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES \
               ($1, $3::uuid, 'CN-001', 'Alice', 'LIVE', now()), \
               ($2, $3::uuid, 'CN-002', 'Bob', 'LIVE', now()) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(CUSTOMER_ALICE_CUST_ID)
        .bind(CUSTOMER_BOB_CUST_ID)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert customers");

        seed_slot_rule(&pool, B01, 30, 10, 14).await;

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

        // Ensure branch_service_status exists and is open
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

        Self { pool, schema }
    }

    pub async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ callers

pub struct HeaderUsers;

pub fn user_id_for_role(role_name: &str) -> String {
    match role_name {
        "TELLER" => TELLER_ID.to_string(),
        "BRANCH_MANAGER" => MANAGER_ID.to_string(),
        "ADMIN" => ADMIN_ID.to_string(),
        "FINANCE" => FINANCE_ID.to_string(),
        "CUSTOMER" => CUSTOMER_ALICE_ID.to_string(),
        _ => Uuid::new_v4().to_string(),
    }
}

pub fn user_of(spec: &str) -> Option<User> {
    let (role_part, target_part) = spec.split_once('@')?;
    let role = match role_part {
        "OWNER" | "EXECUTIVE" => Role::Executive,
        "ADMIN" => Role::Admin,
        "FINANCE" => Role::Finance,
        "BRANCH_MANAGER" => Role::BranchManager,
        "TELLER" => Role::Teller,
        "CUSTOMER" => Role::Customer,
        other => panic!("unknown role {other}"),
    };

    if role == Role::Customer {
        let (uid, cid) = if target_part == "customer-alice" || target_part == CUSTOMER_ALICE_CUST_ID
        {
            (CUSTOMER_ALICE_ID.to_string(), CUSTOMER_ALICE_CUST_ID)
        } else if target_part == "customer-bob" || target_part == CUSTOMER_BOB_CUST_ID {
            (CUSTOMER_BOB_ID.to_string(), CUSTOMER_BOB_CUST_ID)
        } else {
            (Uuid::new_v4().to_string(), target_part)
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
            Some(cid),
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

pub fn spec_of(req: &HttpRequest) -> Option<String> {
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

#[derive(Debug)]
pub struct Answer {
    pub status: StatusCode,
    pub text: String,
    pub body: Value,
}

pub async fn call(
    db: &TestDb,
    who: Option<&str>,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: Option<&str>,
) -> Answer {
    let state = AppState::new(Arc::new(test_common::settings(&[])), db.pool.clone());
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

/// A policy refusal: the role code and a client-error status (`PolicyError::status` = 400).
pub fn policy_refused(a: &Answer, what: &str) {
    assert!(
        a.status == StatusCode::BAD_REQUEST
            && (a.body["error"]["code"] == "roleNotAllowed"
                || a.body["error"]["code"] == "roleNotAllowedHere"),
        "{what}: expected 400 roleNotAllowed refusal; got {} {}",
        a.status,
        a.text
    );
}

/// A domain refusal with `code` and a client-error status.
pub fn refused_with(a: &Answer, code: &str, what: &str) {
    assert!(
        a.status.is_client_error() && a.body["error"]["code"] == code,
        "{what}: expected client-error {code} refusal; got {} {}",
        a.status,
        a.text
    );
}

// ------------------------------------------------------------ seed helpers

pub async fn seed_slot_rule(
    pool: &PgPool,
    branch_id: &str,
    slot_minutes: i32,
    capacity: i32,
    horizon_days: i32,
) {
    sqlx::query(
        "INSERT INTO booking_slot_rules (organization_id, branch_id, slot_minutes, capacity, horizon_days, updated_by) \
         VALUES ($1::uuid, $2, $3, $4, $5, $6::uuid) \
         ON CONFLICT (branch_id) DO UPDATE SET slot_minutes = $3, capacity = $4, horizon_days = $5",
    )
    .bind(ORG)
    .bind(branch_id)
    .bind(slot_minutes)
    .bind(capacity)
    .bind(horizon_days)
    .bind(ADMIN_ID)
    .execute(pool)
    .await
    .expect("seed slot rule");
}

#[allow(clippy::too_many_arguments)]
pub async fn seed_slot(
    pool: &PgPool,
    slot_id: Uuid,
    branch_id: &str,
    service_date: chrono::NaiveDate,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    capacity: i32,
    reserved: i32,
    status: &str,
) {
    sqlx::query(
        "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(slot_id)
    .bind(ORG)
    .bind(branch_id)
    .bind(service_date)
    .bind(start_at)
    .bind(end_at)
    .bind(capacity)
    .bind(reserved)
    .bind(status)
    .execute(pool)
    .await
    .expect("seed slot");
}

#[allow(clippy::too_many_arguments)]
pub async fn seed_booking(
    pool: &PgPool,
    booking_id: &str,
    booking_no: &str,
    customer_id: &str,
    branch_id: &str,
    slot_id: Uuid,
    source_channel: &str,
    status: &str,
    qr_hash: &[u8],
) {
    let confirm_expires_at = (status == "PENDING_CUSTOMER_CONFIRMATION")
        .then(|| Utc::now() + chrono::Duration::hours(1));
    let partner_attribution_id = (source_channel == "PARTNER").then(Uuid::new_v4);
    sqlx::query(
        "INSERT INTO bookings ( \
             id, organization_id, branch_id, booking_no, customer_id, slot_id, \
             shop_side, requested_currency, requested_quantity, source_channel, \
             status, qr_token_hash, qr_version, confirm_expires_at, partner_attribution_id, created_by \
         ) VALUES ( \
             $1, $2::uuid, $3, $4, $5, $6, \
             'BUY', 'USD', 100.0, $7, \
             $8, $9, 1, $10, $11, 'test' \
         )",
    )
    .bind(booking_id)
    .bind(ORG)
    .bind(branch_id)
    .bind(booking_no)
    .bind(customer_id)
    .bind(slot_id)
    .bind(source_channel)
    .bind(status)
    .bind(qr_hash)
    .bind(confirm_expires_at)
    .bind(partner_attribution_id)
    .execute(pool)
    .await
    .expect("seed booking");
}

pub async fn seed_customer_phone(pool: &PgPool, customer_id: &str, phone: &str) {
    let keys =
        xc_kernel::pii::PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
            .expect("the test PII key parses");
    let normalized = xc_kernel::pii::normalize(xc_kernel::pii::IdentityType::Phone, phone, None)
        .expect("valid phone");
    let lookup_hash = xc_kernel::pii::LookupHasher::new(&keys)
        .hash(xc_kernel::pii::IdentityType::Phone, &normalized);
    let cipher = xc_kernel::pii::PiiCipher::new(std::sync::Arc::new(keys));
    let sealed = cipher.encrypt(format!("{customer_id}PHONE").as_bytes(), phone.as_bytes());

    sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, \
         lookup_hash, status) SELECT gen_random_uuid(), organization_id, id, 'PHONE', $2, $3, $4, 'ACTIVE' \
         FROM customers WHERE id = $1",
    )
    .bind(customer_id)
    .bind(&sealed.bytes)
    .bind(&sealed.key_id)
    .bind(lookup_hash.as_slice())
    .execute(pool)
    .await
    .expect("insert the phone identity");
}
