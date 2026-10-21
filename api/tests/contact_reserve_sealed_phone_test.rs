//! #2701: A walk-in customer created from a desk booking has their phone sealed
//! in normalised form (+66812345678), not as typed (081-234-5678).

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_infra::transactions::Transactor;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_customer::application::customer::ContactReserve;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, PiiKeys, Sealed};

const TEST_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const TYPED_PHONE: &str = "081-234-5678";
const E164_PHONE: &str = "+66812345678";

fn test_keys() -> Arc<PiiKeys> {
    Arc::new(PiiKeys::parse(TEST_KEYS).expect("the test PII key parses"))
}

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn xc_org() -> Uuid {
    Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("org 1")
}

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
struct IdentityRow {
    id: Uuid,
    organization_id: Uuid,
    r#type: String,
    encrypted_value: Vec<u8>,
    key_id: String,
    lookup_hash: Vec<u8>,
    verified_at: Option<DateTime<Utc>>,
    status: String,
}

struct Db {
    pool: PgPool,
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
        assert!(!raw_url.contains(":6381"), "never touch port 6381");
        let options = PgConnectOptions::from_str(&raw_url).expect("valid connection URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("cr_phone");
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
        Self { pool, schema }
    }

    async fn finish(&self) {
        let raw_url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let options = PgConnectOptions::from_str(&raw_url).expect("valid connection URL");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("reconnect admin");
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .expect("drop the test schema");
        admin.close().await;
    }

    fn contact_reserve(&self) -> ContactReserve {
        ContactReserve::new(BranchTx::new(Transactor::new(self.pool.clone())))
    }

    async fn phone_identities(&self, customer_id: &str) -> Vec<IdentityRow> {
        sqlx::query_as(
            "SELECT id, organization_id, type, encrypted_value, key_id, lookup_hash, \
             verified_at, status FROM customer_identities \
             WHERE customer_id = $1 AND type = 'PHONE' ORDER BY created_at ASC",
        )
        .bind(customer_id)
        .fetch_all(&self.pool)
        .await
        .expect("read customer_identities")
    }
}

// ── Behaviour table rows §8 ─────────────────────────────────────────────────

#[tokio::test]
async fn row_n1_typed_phone_new_customer_sealed_identity_decrypts_to_normalized_e164() {
    let db = Db::start().await;
    let contact_reserve = db.contact_reserve();
    let org_id = xc_org();
    let now = Utc::now();

    let mut tx = db.pool.begin().await.expect("begin tx");
    let reservation = contact_reserve
        .reserve_in(&mut tx, org_id, "Walk-in Customer", TYPED_PHONE, now)
        .await
        .expect("reserve customer");
    tx.commit().await.expect("commit tx");

    assert!(!reservation.existing, "creates new customer");
    let rows = db.phone_identities(&reservation.customer_id).await;
    assert_eq!(rows.len(), 1, "exactly one PHONE identity row");
    let row = &rows[0];

    let cipher = PiiCipher::new(test_keys());
    let aad = format!("{}PHONE", reservation.customer_id);
    let opened = cipher
        .decrypt(
            aad.as_bytes(),
            &Sealed {
                key_id: row.key_id.clone(),
                bytes: row.encrypted_value.clone(),
            },
        )
        .expect("identity opens with AAD {customer_id}PHONE");

    assert_eq!(
        std::str::from_utf8(&opened).expect("valid utf8"),
        E164_PHONE,
        "sealed value must be normalized to E.164, not raw typed"
    );

    db.finish().await;
}

#[tokio::test]
async fn row_n2_e164_phone_new_customer_decrypts_to_e164() {
    let db = Db::start().await;
    let contact_reserve = db.contact_reserve();
    let org_id = xc_org();
    let now = Utc::now();

    let mut tx = db.pool.begin().await.expect("begin tx");
    let reservation = contact_reserve
        .reserve_in(&mut tx, org_id, "E164 Customer", E164_PHONE, now)
        .await
        .expect("reserve customer");
    tx.commit().await.expect("commit tx");

    assert!(!reservation.existing, "creates new customer");
    let rows = db.phone_identities(&reservation.customer_id).await;
    assert_eq!(rows.len(), 1, "exactly one PHONE identity row");
    let row = &rows[0];

    let cipher = PiiCipher::new(test_keys());
    let aad = format!("{}PHONE", reservation.customer_id);
    let opened = cipher
        .decrypt(
            aad.as_bytes(),
            &Sealed {
                key_id: row.key_id.clone(),
                bytes: row.encrypted_value.clone(),
            },
        )
        .expect("identity opens with AAD {customer_id}PHONE");

    assert_eq!(
        std::str::from_utf8(&opened).expect("valid utf8"),
        E164_PHONE,
        "sealed value is E.164"
    );

    db.finish().await;
}

#[tokio::test]
async fn row_n3_already_active_phone_returns_existing_customer_and_writes_no_identity() {
    let db = Db::start().await;
    let contact_reserve = db.contact_reserve();
    let org_id = xc_org();
    let now = Utc::now();

    let mut tx1 = db.pool.begin().await.expect("begin tx1");
    let res1 = contact_reserve
        .reserve_in(&mut tx1, org_id, "Existing Customer", E164_PHONE, now)
        .await
        .expect("first reserve");
    tx1.commit().await.expect("commit tx1");
    assert!(!res1.existing, "first call creates customer");

    let count_before = db.phone_identities(&res1.customer_id).await.len();
    assert_eq!(count_before, 1, "one identity initially");

    let mut tx2 = db.pool.begin().await.expect("begin tx2");
    let res2 = contact_reserve
        .reserve_in(&mut tx2, org_id, "Duplicate Caller", TYPED_PHONE, now)
        .await
        .expect("second reserve with typed phone");
    tx2.commit().await.expect("commit tx2");

    assert!(res2.existing, "second call finds existing customer");
    assert_eq!(
        res2.customer_id, res1.customer_id,
        "returns the existing customer id"
    );

    let count_after = db.phone_identities(&res1.customer_id).await.len();
    assert_eq!(count_after, 1, "writes no new identity row");

    db.finish().await;
}

#[tokio::test]
async fn row_n4_any_phone_lookup_hash_equals_normalized_hash() {
    let db = Db::start().await;
    let contact_reserve = db.contact_reserve();
    let org_id = xc_org();
    let now = Utc::now();

    let expected_hash = LookupHasher::new(&test_keys())
        .hash(IdentityType::Phone, E164_PHONE)
        .to_vec();

    // Test with typed phone
    let mut tx1 = db.pool.begin().await.expect("begin tx1");
    let res1 = contact_reserve
        .reserve_in(&mut tx1, org_id, "Customer 1", TYPED_PHONE, now)
        .await
        .expect("reserve typed");
    tx1.commit().await.expect("commit tx1");

    let rows1 = db.phone_identities(&res1.customer_id).await;
    assert_eq!(rows1.len(), 1);
    assert_eq!(
        rows1[0].lookup_hash, expected_hash,
        "lookup_hash for typed phone equals hash(Phone, '+66812345678')"
    );

    // Test with another distinct phone to verify general property
    let other_typed = "089-999-9999";
    let other_e164 = "+66899999999";
    let other_expected = LookupHasher::new(&test_keys())
        .hash(IdentityType::Phone, other_e164)
        .to_vec();

    let mut tx2 = db.pool.begin().await.expect("begin tx2");
    let res2 = contact_reserve
        .reserve_in(&mut tx2, org_id, "Customer 2", other_typed, now)
        .await
        .expect("reserve other");
    tx2.commit().await.expect("commit tx2");

    let rows2 = db.phone_identities(&res2.customer_id).await;
    assert_eq!(rows2.len(), 1);
    assert_eq!(
        rows2[0].lookup_hash, other_expected,
        "lookup_hash equals hash(Phone, normalized)"
    );

    db.finish().await;
}
