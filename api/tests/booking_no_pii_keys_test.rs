//! Regression test for #1850: when NO PII keys are installed in the process,
//! a v3 booking command must answer 500 `serverError` with no panic and no fallback key.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use std::path::Path;
use std::str::FromStr;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::json;
use sqlx::AssertSqlSafe;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use uuid::Uuid;

use bk6_common::*;

async fn start_db_without_pii_keys() -> TestDb {
    let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!("TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip")
    });
    let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
    let database = options.get_database().unwrap_or("").to_owned();
    assert!(
        database.ends_with("_test"),
        "'{database}' does not end in `_test`"
    );
    let schema = unique("bk6_no_pii_keys");
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

    // Deliberately do NOT call xc_customer::install_pii_keys here.
    TestDb { pool, schema }
}

#[actix_web::test]
async fn booking_command_without_pii_keys_answers_500_server_error() {
    assert!(
        xc_customer::get_pii_keys().is_err(),
        "no PII keys may be installed in this test binary"
    );

    let db = start_db_without_pii_keys().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::minutes(10),
        now + Duration::minutes(40),
        5,
        0,
        "OPEN",
    )
    .await;

    let create_payload = json!({
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "BUY",
        "currency": "USD",
        "quantity": 100.0,
    });

    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        "/api/bookings",
        Some(create_payload),
        Some("k-1850-no-keys"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "booking command without PII keys installed must answer 500 Internal Server Error, got: {} {}",
        res.status,
        res.text
    );
    assert_eq!(
        res.body["error"]["code"], "serverError",
        "error code must be serverError when no PII keys are installed, got: {}",
        res.text
    );

    db.finish().await;

    assert!(
        xc_customer::get_pii_keys().is_err(),
        "no PII keys may be installed as a side effect of handling a booking command without keys"
    );
}
