//! BK-10 (#1412) rework: Verifying an unverified ACTIVE PHONE identity must set verified_at
//! and write one value-free audit row ({identity_id} only, no phone value) in the same transaction.
//! Verifying again keeps the first verified_at and writes no additional audit row.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use pos_infra::transactions::Transactor;
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_customer::application::customer::contact_verify::ContactVerify;
use xc_customer::install_pii_keys;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, PiiKeys};

const ORG_ID_STR: &str = "00000000-0000-0000-0000-000000000001";
const TEST_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn url() -> String {
    std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!("TEST_DATABASE_URL must be set, e.g. postgres://postgres:postgres@127.0.0.1:55435/xchange_test")
    })
}

fn test_keys() -> Arc<PiiKeys> {
    Arc::new(PiiKeys::parse(TEST_KEYS).expect("valid keys"))
}

async fn setup_db() -> (PgPool, String) {
    let db_url = url();
    let schema = unique("audit_test");
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

    let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let mut entries: Vec<_> = std::fs::read_dir(&migrations_dir)
        .unwrap_or_else(|e| panic!("read migrations dir {}: {e}", migrations_dir.display()))
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
    let db_url = url();
    pool.close().await;

    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .expect("admin reconnect");

    sqlx::raw_sql(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .expect("drop schema");
    admin.close().await;
}

fn teller_user(org_id: &Uuid, branch: &str) -> Actor {
    Actor::new(
        &Uuid::new_v4().to_string(),
        &org_id.to_string(),
        vec![ScopedRole::new(
            Role::Teller,
            Some(BranchId::new(branch)),
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

/// Whole seconds, so a round trip through `timestamptz` is exact on every platform.
fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_760_371_200 + secs, 0).expect("valid timestamp")
}

#[test]
fn unverified_active_phone_verification_writes_audit_and_reverify_is_idempotent() {
    sqlx::test_block_on(async {
        let (pool, schema) = setup_db().await;
        let keys = test_keys();
        let _ = install_pii_keys(keys.clone());

        let org_id = Uuid::parse_str(ORG_ID_STR).unwrap();
        let customer_id = unique("CX");

        // 1. Seed customer
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($1, $2, $1, $3, 'LIVE', $4)",
        )
        .bind(&customer_id)
        .bind(org_id)
        .bind("Somchai Unverified")
        .bind(at(-3600))
        .execute(&pool)
        .await
        .expect("insert customer");

        // 2. Seed unverified ACTIVE PHONE identity
        let raw_phone = "081-234-5678";
        let e164_phone = "+66812345678";
        let sealed = PiiCipher::new(keys.clone()).encrypt(
            format!("{customer_id}PHONE").as_bytes(),
            e164_phone.as_bytes(),
        );
        let lookup_hash = LookupHasher::new(&keys).hash(IdentityType::Phone, e164_phone);
        let existing_identity_id = Uuid::new_v4();

        sqlx::query(
            "INSERT INTO customer_identities ( \
                 id, customer_id, organization_id, type, encrypted_value, key_id, \
                 lookup_hash, verified_at, status, created_at \
             ) VALUES ($1, $2, $3, 'PHONE', $4, $5, $6, NULL, 'ACTIVE', $7)",
        )
        .bind(existing_identity_id)
        .bind(&customer_id)
        .bind(org_id)
        .bind(&sealed.bytes)
        .bind(&sealed.key_id)
        .bind(&lookup_hash[..])
        .bind(at(-1800))
        .execute(&pool)
        .await
        .expect("seed unverified active phone");

        // Verify initial state: 0 audit events, verified_at is NULL
        let initial_audit_count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_events")
            .fetch_one(&pool)
            .await
            .expect("initial audit count");
        assert_eq!(initial_audit_count, 0, "must start with 0 audit events");

        let initial_verified_at: Option<DateTime<Utc>> =
            sqlx::query_scalar("SELECT verified_at FROM customer_identities WHERE id = $1")
                .bind(existing_identity_id)
                .fetch_one(&pool)
                .await
                .expect("initial verified_at check");
        assert!(initial_verified_at.is_none(), "starts unverified");

        let transactor = Transactor::new(pool.clone());
        let branch_tx = BranchTx::new(transactor);
        let service = ContactVerify::new(branch_tx);
        let who = teller_user(&org_id, "MAIN");
        let branch = BranchId::new("MAIN");

        // 3. First verify: unverified ACTIVE phone -> should set verified_at and write exactly 1 audit row
        let verify_time_1 = at(0);
        let mut tx1 = pool.begin().await.expect("begin tx1");
        let summary1 = service
            .verify_in(
                &mut tx1,
                &who,
                &branch,
                &customer_id,
                raw_phone,
                verify_time_1,
            )
            .await
            .expect("first verify succeeds");
        tx1.commit().await.expect("commit tx1");

        assert_eq!(summary1.customer_id, customer_id);
        assert_eq!(summary1.identity_id, existing_identity_id);
        assert_eq!(summary1.verified_at, verify_time_1);
        assert!(summary1.phone_masked.ends_with("5678"));

        // DB check: verified_at is now set
        let db_verified_at: Option<DateTime<Utc>> =
            sqlx::query_scalar("SELECT verified_at FROM customer_identities WHERE id = $1")
                .bind(existing_identity_id)
                .fetch_one(&pool)
                .await
                .expect("db verified_at check");
        assert_eq!(
            db_verified_at,
            Some(verify_time_1),
            "verified_at must be updated"
        );

        // Audit check: exactly 1 audit row written
        let audit_count_after_first: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_events")
            .fetch_one(&pool)
            .await
            .expect("audit count after first verify");
        assert_eq!(
            audit_count_after_first, 1,
            "verifying unverified active phone must write exactly 1 audit row"
        );

        // Verify audit row contents: value-free, after_redacted has identity_id, no phone PII
        let (action, object_ref, after_redacted, audit_row_text): (
            String,
            String,
            serde_json::Value,
            String,
        ) = sqlx::query_as(
            "SELECT action, object_ref, after_redacted, \
                    concat_ws(' | ', actor_id, actor_role, scope, branch_id, action, object_ref, \
                              before_redacted::text, after_redacted::text) \
             FROM audit_events ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("fetch audit row");

        assert_eq!(action, "customerContactVerify");
        assert_eq!(object_ref, customer_id);
        assert_eq!(
            after_redacted,
            serde_json::json!({ "identity_id": existing_identity_id }),
            "after_redacted must only contain identity_id"
        );

        // Ensure no phone representation exists in audit row text
        for forbidden in [
            "081-234-5678",
            "+66812345678",
            "0812345678",
            "812345678",
            "5678",
        ] {
            assert!(
                !audit_row_text.contains(forbidden),
                "audit row text must not contain phone token '{forbidden}': {audit_row_text}"
            );
        }

        // 4. Second verify (re-verify / idempotent call): already verified -> leaves still 1 audit row
        let verify_time_2 = at(600);
        let mut tx2 = pool.begin().await.expect("begin tx2");
        let summary2 = service
            .verify_in(
                &mut tx2,
                &who,
                &branch,
                &customer_id,
                raw_phone,
                verify_time_2,
            )
            .await
            .expect("second verify succeeds");
        tx2.commit().await.expect("commit tx2");

        assert_eq!(summary2.customer_id, customer_id);
        assert_eq!(summary2.identity_id, existing_identity_id);
        assert_eq!(
            summary2.verified_at, verify_time_1,
            "must keep the first verified_at timestamp"
        );

        // DB check: verified_at is still verify_time_1
        let db_verified_at_after_second: Option<DateTime<Utc>> =
            sqlx::query_scalar("SELECT verified_at FROM customer_identities WHERE id = $1")
                .bind(existing_identity_id)
                .fetch_one(&pool)
                .await
                .expect("db verified_at check after second verify");
        assert_eq!(
            db_verified_at_after_second,
            Some(verify_time_1),
            "verified_at must remain unchanged after reverification"
        );

        // Audit check: STILL exactly 1 audit row (already verified writes nothing)
        let audit_count_after_second: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_events")
            .fetch_one(&pool)
            .await
            .expect("audit count after second verify");
        assert_eq!(
            audit_count_after_second, 1,
            "re-verifying already verified phone must NOT write another audit row"
        );

        cleanup_db(pool, schema).await;
    });
}
