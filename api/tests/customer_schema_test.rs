//! CI-4 (#651) Part 1: migration M1 `api/migrations/20261019100000_customers_org.sql` (design
//! `docs/design/blueprint/customer-identity-consent.md` §4.1, §8 M1, §9.1 row "CI-4 Part 1").
//!
//! Written before the implementation by the ticket author (claude/89458e53). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

const M1: i64 = 20261019100000;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

struct Db {
    pool: PgPool,
    migrator: Migrator,
}

impl Db {
    async fn new() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip. \
                 Point it at an existing database whose name ends in `_test`."
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("ci4m1");
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
            .max_connections(2)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrator = Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations");
        assert!(
            migrator.version_exists(M1),
            "api/migrations/20261019100000_customers_org.sql exists"
        );
        Db { pool, migrator }
    }

    async fn fresh() -> Db {
        let db = Db::new().await;
        db.migrator
            .run(&db.pool)
            .await
            .expect("every migration applies in version order on a fresh database");
        db
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn exec(&self, sql: &str) -> Result<(), String> {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn columns(&self, table: &str) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT column_name::text FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = $1 ORDER BY 1",
        )
        .bind(table)
        .fetch_all(&self.pool)
        .await
        .expect("read the columns")
    }

    async fn customer(&self, id: &str) {
        self.exec(
            "INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN') ON CONFLICT (id) DO NOTHING",
        )
        .await
        .expect("the branch MAIN");
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT '{id}', organization_id, '{id}', 'Somchai', 'LIVE', now() FROM branches WHERE id = 'MAIN'"
        ))
        .await
        .expect("insert a customer of the new shape");
    }

    async fn identity(&self, customer: &str, hash: &str, status: &str) -> Result<(), String> {
        self.exec(&format!(
            "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, \
             lookup_hash, nationality, status) \
             SELECT gen_random_uuid(), organization_id, id, 'PASSPORT', '\\x00'::bytea, 'k1', \
             decode('{hash}', 'hex'), 'THA', '{status}' FROM customers WHERE id = '{customer}'"
        ))
        .await
    }
}

#[tokio::test]
async fn a_fresh_database_applies_every_migration_in_version_order() {
    let db = Db::fresh().await;
    for table in [
        "customer_identities",
        "consent_texts",
        "consents",
        "retention_policies",
    ] {
        assert_eq!(
            db.count(&format!(
                "SELECT count(*) FROM information_schema.tables \
                 WHERE table_schema = current_schema() AND table_name = '{table}'"
            ))
            .await,
            1,
            "{table} exists"
        );
    }
}

#[tokio::test]
async fn m1_refuses_while_customers_has_rows_saying_recreate_the_database() {
    let db = Db::new().await;
    db.migrator
        .run_to(M1 - 1, &db.pool)
        .await
        .expect("migrate up to the one before M1");
    db.exec("INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN') ON CONFLICT (id) DO NOTHING")
        .await
        .expect("the branch MAIN");
    db.exec(
        "INSERT INTO customers (id, branch_id, name, passport_no, nationality, mode, created_at) \
         VALUES ('CX-00000001', 'MAIN', 'Somchai', 'AA1234567', 'THA', 'LIVE', now())",
    )
    .await
    .expect("a customer as the pre-CI-4 binary writes it");
    let err = db
        .migrator
        .run_to(M1, &db.pool)
        .await
        .expect_err("M1 refuses: U4, the test database is recreated, not converted")
        .to_string();
    assert!(
        err.contains("recreate the test database first (U4)"),
        "the U4 sentence of §8 M1 item 1: {err}"
    );
}

#[tokio::test]
async fn customers_lose_every_document_column_and_gain_the_blueprint_ones() {
    let db = Db::fresh().await;
    let cols = db.columns("customers").await;
    for gone in [
        "branch_id",
        "passport_no",
        "nationality",
        "birth_date",
        "passport_expiry",
        "name",
    ] {
        assert!(
            !cols.contains(&gone.to_owned()),
            "customers.{gone} is removed: {cols:?}"
        );
    }
    for kept in [
        "id",
        "organization_id",
        "customer_no",
        "display_name",
        "preferred_language",
        "status",
        "mode",
        "user_id",
        "created_at",
        "updated_at",
        "last_document_ref",
    ] {
        assert!(
            cols.contains(&kept.to_owned()),
            "customers.{kept}: {cols:?}"
        );
    }
}

#[tokio::test]
async fn a_customer_number_is_unique_within_its_organisation() {
    let db = Db::fresh().await;
    db.customer("CX-00000001").await;
    let again = db
        .exec(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT 'CX-OTHER', organization_id, 'CX-00000001', 'Other', 'LIVE', now() FROM branches WHERE id = 'MAIN'",
        )
        .await;
    assert!(again.is_err(), "UNIQUE (organization_id, customer_no)");
}

#[tokio::test]
async fn the_preferred_language_is_th_en_or_zh() {
    let db = Db::fresh().await;
    db.customer("CX-00000001").await;
    for (lang, ok) in [("th", true), ("en", true), ("zh", true), ("fr", false)] {
        let res = db
            .exec(&format!(
                "UPDATE customers SET preferred_language = '{lang}' WHERE id = 'CX-00000001'"
            ))
            .await;
        assert_eq!(res.is_ok(), ok, "{lang}");
    }
}

#[tokio::test]
async fn one_active_identity_per_organisation_type_and_hash_and_a_revoked_one_frees_it() {
    let db = Db::fresh().await;
    db.customer("CX-00000001").await;
    db.customer("CX-00000002").await;
    let hash = "aa".repeat(32);
    db.identity("CX-00000001", &hash, "ACTIVE")
        .await
        .expect("the first ACTIVE identity");
    assert!(
        db.identity("CX-00000002", &hash, "ACTIVE").await.is_err(),
        "a second ACTIVE identity with the same (organisation, type, hash) is refused"
    );
    db.exec("UPDATE customer_identities SET status = 'REVOKED' WHERE customer_id = 'CX-00000001'")
        .await
        .expect("revoke the first");
    db.identity("CX-00000002", &hash, "ACTIVE")
        .await
        .expect("allowed once the first is REVOKED");
}

#[tokio::test]
async fn consents_are_append_only() {
    let db = Db::fresh().await;
    db.customer("CX-00000001").await;
    db.exec(
        "INSERT INTO consents (id, organization_id, customer_id, purpose, channel, policy_version, lang, granted, \
         effective_at, evidence_ref) SELECT gen_random_uuid(), organization_id, id, 'TERMS', NULL, 1, 'th', true, \
         now(), 'req-1' FROM customers WHERE id = 'CX-00000001'",
    )
    .await
    .expect("a consent row");
    assert!(
        db.exec("UPDATE consents SET granted = false")
            .await
            .is_err(),
        "no UPDATE path"
    );
    assert!(
        db.exec("DELETE FROM consents").await.is_err(),
        "no DELETE path"
    );
    assert_eq!(db.count("SELECT count(*) FROM consents").await, 1);
}
