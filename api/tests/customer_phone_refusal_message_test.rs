//! #4900: Desk phone verification: a mistyped phone answers a worded message, not "validationFailed".
//!
//! Ticket behaviour (§3):
//! - P1: start with phone `12ab` => 400, code `validationFailed`, field `phone`, message != `validationFailed` and non-empty.
//! - P2: confirm with a malformed phone => same as P1.
//! - P3: start with a valid phone => as today.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_kernel::pii::PiiKeys;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const TEST_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

fn test_keys() -> Arc<PiiKeys> {
    Arc::new(PiiKeys::parse(TEST_KEYS).expect("the test PII key parses"))
}

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn xc_org() -> Uuid {
    Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("org 1")
}

const TELLER_USER_ID: &str = "00000000-0000-4000-8000-000000000010";

const HOST: &str = "localhost:8080";
const SAME_ORIGIN: &str = "http://localhost:8080";
const AS: &str = "x-test-as";

struct HeaderUsers;

fn user_uuid(spec: &str) -> Uuid {
    match spec {
        "TELLER@MAIN" | "TELLER" => Uuid::parse_str(TELLER_USER_ID).unwrap(),
        other => {
            if let Ok(u) = Uuid::parse_str(other) {
                u
            } else {
                Uuid::parse_str("00000000-0000-4000-8000-000000000099").unwrap()
            }
        }
    }
}

fn user_id(spec: &str) -> String {
    user_uuid(spec).to_string()
}

fn user_of(spec: &str) -> Option<User> {
    let (uid, role_str, branch_str) = if spec == TELLER_USER_ID {
        (TELLER_USER_ID.to_string(), "TELLER", "MAIN")
    } else if let Some((id_part, rest)) = spec.split_once('|') {
        let (r, b) = rest.split_once('@')?;
        (id_part.to_string(), r, b)
    } else {
        let (r, b) = spec.split_once('@')?;
        (user_id(spec), r, b)
    };

    let role = match role_str {
        "TELLER" => Role::Teller,
        _ => return None,
    };
    let branch = (branch_str != "*").then(|| BranchId::new(branch_str));
    Some(User::new(
        &uid,
        &xc_org().to_string(),
        vec![ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        Box::pin(async move {
            let u = user_of(&spec);
            Callers {
                user: u.clone(),
                staff_user: u,
            }
        })
    }
}

struct Db {
    pool: PgPool,
    #[allow(dead_code)]
    schema: String,
}

impl Db {
    async fn start() -> Self {
        let raw_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL must be set and name a database ending in `_test`.\n\
                 Never touch port 6381."
            )
        });
        assert!(
            !raw_url.contains(":6381"),
            "Safety: never touch port 6381 (live/shared service)"
        );
        let options = PgConnectOptions::from_str(&raw_url).expect("valid connection URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("phone_refusal");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(16)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");

        Migrator::new(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("migrations")
                .as_path(),
        )
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("apply api/migrations");

        let _ = xc_customer::install_pii_keys(test_keys());
        for b in ["MAIN", "B2", "CHINATOWN"] {
            let name = match b {
                "CHINATOWN" => "Chinatown Branch",
                "B2" => "Siam Paragon",
                _ => "Main Branch",
            };
            sqlx::query(
                "INSERT INTO branches (id, name) VALUES ($1, $2) ON CONFLICT (id) DO UPDATE SET name = $2",
            )
            .bind(b)
            .bind(name)
            .execute(&pool)
            .await
            .expect("insert a branch");
        }

        let teller_user_id = Uuid::parse_str(TELLER_USER_ID).unwrap();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2, 'Route Teller', 'ACTIVE', 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(teller_user_id)
        .bind(xc_org())
        .execute(&pool)
        .await
        .expect("seed route users in xc_org");

        Db { pool, schema }
    }

    async fn customer(&self, org: Uuid, lang: &str) -> String {
        let id = unique("CX");
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, preferred_language, created_at) \
             VALUES ($1, $2, $1, 'Test Customer', 'LIVE', $3, now())",
        )
        .bind(&id)
        .bind(org)
        .bind(lang)
        .execute(&self.pool)
        .await
        .expect("insert a customer");
        id
    }
}

#[actix_web::test]
async fn p1_start_with_phone_12ab_answers_validation_failed_with_phone_and_worded_message() {
    let db = Db::start().await;
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);

    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let org = xc_org();
    let cust_id = db.customer(org, "th").await;
    let teller_hdr = format!("{TELLER_USER_ID}|TELLER@MAIN");

    let start_uri = format!("/api/customers/{cust_id}/phone-verifications");
    let req = atest::TestRequest::post()
        .uri(&start_uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("idempotency-key", unique("k")))
        .insert_header((AS, teller_hdr.as_str()))
        .set_json(json!({
            "branch": "MAIN",
            "phone": "12ab"
        }));
    let resp = atest::call_service(&app, req.to_request()).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let body: Value = atest::read_body_json(resp).await;
    assert_eq!(body["error"]["code"], "validationFailed");
    assert_eq!(body["error"]["field"], "phone");
    let msg = body["error"]["message"]
        .as_str()
        .expect("message must be a string");
    assert!(!msg.is_empty(), "message must not be empty");
    assert_ne!(
        msg, "validationFailed",
        "message must not equal validationFailed"
    );
    assert_eq!(msg, "invalid phone number");
}

#[actix_web::test]
async fn p2_confirm_with_malformed_phone_answers_validation_failed_with_phone_and_worded_message() {
    let db = Db::start().await;
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);

    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let org = xc_org();
    let cust_id = db.customer(org, "th").await;
    let teller_hdr = format!("{TELLER_USER_ID}|TELLER@MAIN");
    let teller_user_id = Uuid::parse_str(TELLER_USER_ID).unwrap();

    let challenge_id = Uuid::new_v4();
    // Non-pending status triggers PhoneVerificationError::ValidationFailed("challenge is not pending")
    sqlx::query(
        "INSERT INTO phone_verifications ( \
            id, organization_id, customer_id, branch_id, phone_encrypted, \
            key_id, phone_lookup_hash, code_hash, expires_at, attempts, \
            status, requested_by, created_at \
         ) VALUES ( \
            $1, $2, $3, 'MAIN', decode('', 'hex'), \
            'test-1', decode('', 'hex'), decode('', 'hex'), now() + interval '10 minutes', 0, \
            'VERIFIED', $4, now() \
         )",
    )
    .bind(challenge_id)
    .bind(org)
    .bind(&cust_id)
    .bind(teller_user_id)
    .execute(&db.pool)
    .await
    .expect("seed non-pending phone verification");

    let confirm_uri =
        format!("/api/customers/{cust_id}/phone-verifications/{challenge_id}/confirm");
    let req = atest::TestRequest::post()
        .uri(&confirm_uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("idempotency-key", unique("k")))
        .insert_header((AS, teller_hdr.as_str()))
        .set_json(json!({
            "branch": "MAIN",
            "code": "123456",
            "phone": "12ab"
        }));
    let resp = atest::call_service(&app, req.to_request()).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let body: Value = atest::read_body_json(resp).await;
    assert_eq!(body["error"]["code"], "validationFailed");
    assert_eq!(body["error"]["field"], "phone");
    let msg = body["error"]["message"]
        .as_str()
        .expect("message must be a string");
    assert!(!msg.is_empty(), "message must not be empty");
    assert_ne!(
        msg, "validationFailed",
        "message must not equal validationFailed"
    );
    assert_eq!(msg, "challenge is not pending");
}

#[actix_web::test]
async fn p3_start_with_valid_phone_behaves_as_today() {
    let db = Db::start().await;
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);

    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let org = xc_org();
    let cust_id = db.customer(org, "th").await;
    let teller_hdr = format!("{TELLER_USER_ID}|TELLER@MAIN");

    let start_uri = format!("/api/customers/{cust_id}/phone-verifications");
    let req = atest::TestRequest::post()
        .uri(&start_uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("idempotency-key", unique("k")))
        .insert_header((AS, teller_hdr.as_str()))
        .set_json(json!({
            "branch": "MAIN",
            "phone": "0812345678"
        }));
    let resp = atest::call_service(&app, req.to_request()).await;

    // As today: reaching the service with Notify unset gives 503 smsUnavailable,
    // proving valid phone passes validation and reaches the SMS notification stage.
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = atest::read_body_json(resp).await;
    assert_eq!(body["error"]["code"], "smsUnavailable");
}
