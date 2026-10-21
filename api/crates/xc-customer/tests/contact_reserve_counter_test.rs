//! #1594: ContactReserve counter exhaustion test.
//! Verifies that when the customer-LIVE counter is exhausted (value reaches 999,999,999),
//! ContactReserve answers COUNTER_EXHAUSTED ("ตัวนับรหัสไม่ถูกต้อง") through the shared sequence.

use std::sync::Arc;

use chrono::Utc;
use pos_error::AppError;
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_customer::COUNTER_EXHAUSTED;
use xc_customer::application::customer::ContactReserve;
use xc_customer::install_pii_keys;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::pii::PiiKeys;

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn url() -> String {
    std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!("TEST_DATABASE_URL must be set, e.g. postgres://postgres:postgres@127.0.0.1:54343/xchange_test")
    })
}

async fn setup_db() -> (PgPool, String) {
    let db_url = url();
    let schema = unique("cust_test");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .expect("admin connect");

    sqlx::raw_sql(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("create schema");
    admin.close().await;

    let options: sqlx::postgres::PgConnectOptions = db_url.parse().unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .expect("connect schema pool");

    let mut entries: Vec<_> = std::fs::read_dir("../../migrations")
        .expect("read migrations dir")
        .map(|r| r.expect("read entry").path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "sql"))
        .collect();
    entries.sort();

    for path in entries {
        let sql = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        sqlx::raw_sql(AssertSqlSafe(sql))
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("failed to execute {}: {e}", path.display()));
    }

    (pool, schema)
}

async fn cleanup_db(pool: PgPool, schema: String) {
    sqlx::raw_sql(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&pool)
        .await
        .expect("drop schema");
    pool.close().await;
}

fn pii_keys() -> PiiKeys {
    PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").expect("valid keys")
}

#[test]
fn contact_reserve_counter_exhausted_answers_counter_exhausted() {
    sqlx::test_block_on(async {
        let (pool, schema) = setup_db().await;
        let keys = pii_keys();
        let _ = install_pii_keys(Arc::new(keys));

        // Seed counter at maximum value (999,999,999)
        sqlx::raw_sql(AssertSqlSafe(
            "INSERT INTO counters (key, value) VALUES ('customer-LIVE', 999999999) \
             ON CONFLICT (key) DO UPDATE SET value = 999999999",
        ))
        .execute(&pool)
        .await
        .expect("seed exhausted counter");

        let transactor = pos_infra::transactions::Transactor::new(pool.clone());
        let branch_tx = BranchTx::new(transactor);
        let contact_reserve = ContactReserve::new(branch_tx);

        let org_id = Uuid::parse_str(ORG_1).unwrap();
        let now = Utc::now();
        let phone = "+66812345678";
        let name = "Somchai Exhausted";

        let mut tx = pool.begin().await.expect("begin tx");
        let result = contact_reserve
            .reserve_in(&mut tx, org_id, name, phone, now)
            .await;

        match result {
            Err(AppError::Domain(msg)) => {
                assert_eq!(
                    msg, COUNTER_EXHAUSTED,
                    "exhausted counter must return COUNTER_EXHAUSTED constant"
                );
            }
            other => panic!("expected AppError::Domain(COUNTER_EXHAUSTED), got: {other:?}"),
        }

        let _ = tx.rollback().await;

        cleanup_db(pool, schema).await;
    });
}

#[test]
fn contact_reserve_normal_succeeds_and_existing_phone_reuses_customer() {
    sqlx::test_block_on(async {
        let (pool, schema) = setup_db().await;
        let keys = pii_keys();
        let _ = install_pii_keys(Arc::new(keys));

        let transactor = pos_infra::transactions::Transactor::new(pool.clone());
        let branch_tx = BranchTx::new(transactor);
        let contact_reserve = ContactReserve::new(branch_tx);

        let org_id = Uuid::parse_str(ORG_1).unwrap();
        let now = Utc::now();
        let phone = "+66898765432";
        let name = "Somchai Active";

        // First reservation: creates new customer
        let mut tx1 = pool.begin().await.expect("begin tx1");
        let res1 = contact_reserve
            .reserve_in(&mut tx1, org_id, name, phone, now)
            .await
            .expect("first reserve succeeds");
        tx1.commit().await.expect("commit tx1");

        assert!(!res1.existing, "first reservation creates new customer");
        assert!(
            res1.customer_id.starts_with("CX-"),
            "customer_id starts with CX-"
        );

        // Second reservation: reuses existing customer
        let mut tx2 = pool.begin().await.expect("begin tx2");
        let res2 = contact_reserve
            .reserve_in(&mut tx2, org_id, "Somchai Caller 2", phone, now)
            .await
            .expect("second reserve succeeds");
        tx2.commit().await.expect("commit tx2");

        assert!(res2.existing, "second reservation reuses existing customer");
        assert_eq!(res1.customer_id, res2.customer_id, "customer_id matches");

        cleanup_db(pool, schema).await;
    });
}
