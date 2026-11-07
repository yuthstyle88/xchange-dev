//! XC Counter, the migration `20261227000000_xc_counter.sql` (`docs/design/xc-counter-app.md` §5, §8.1), read
//! straight from the database. Written by the controller with the skeleton that adds the migration. Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! | #  | Expected |
//! |----|----------|
//! | M1 | `device_agents_one_active_per_branch_idx` is gone |
//! | M2 | `device_agents.counter_name text NOT NULL DEFAULT ''`; `cash_drawer`, `scanner`, `kiosk_lock` `boolean NULL` with no default |
//! | M3 | `device_jobs.target_agent_id uuid NULL` and `user_sessions.counter_agent_id uuid NULL`, both referencing `device_agents (id)` |
//! | M4 | no backfill: a counter that existed before the migration has every setting NULL and an empty name |

use std::path::{Path, PathBuf};
use std::str::FromStr;

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

const VERSION: i64 = 20261227000000;

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

async fn schema() -> (PgPool, String, Migrator) {
    let url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL names a database whose name ends in `_test`");
    let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
    assert!(options.get_database().unwrap_or("").ends_with("_test"));
    let schema = xc_testdb::unique_schema_name("xccm");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .expect("connect");
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("create the schema");
    admin.close().await;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .expect("connect to the schema");
    let migrator = Migrator::new(migrations_dir().as_path())
        .await
        .expect("read api/migrations");
    (pool, schema, migrator)
}

async fn drop_schema(pool: PgPool, schema: &str) {
    pool.close().await;
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("reconnect");
    sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .expect("drop the schema");
    admin.close().await;
}

/// `(data type, nullable, default)` of a column.
async fn column(pool: &PgPool, table: &str, name: &str) -> (String, bool, Option<String>) {
    let (kind, nullable, default): (String, String, Option<String>) = sqlx::query_as(
        "SELECT data_type::text, is_nullable::text, column_default::text FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = $1 AND column_name = $2",
    )
    .bind(table)
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("{table}.{name}: {e}"));
    (kind, nullable == "YES", default)
}

/// The table and column a foreign key of `table.name` references.
async fn references(pool: &PgPool, table: &str, name: &str) -> (String, String) {
    sqlx::query_as(
        "SELECT ccu.table_name::text, ccu.column_name::text \
         FROM information_schema.key_column_usage kcu \
         JOIN information_schema.referential_constraints rc \
           ON rc.constraint_name = kcu.constraint_name AND rc.constraint_schema = kcu.constraint_schema \
         JOIN information_schema.constraint_column_usage ccu \
           ON ccu.constraint_name = rc.unique_constraint_name AND ccu.constraint_schema = rc.unique_constraint_schema \
         WHERE kcu.table_schema = current_schema() AND kcu.table_name = $1 AND kcu.column_name = $2",
    )
    .bind(table)
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("the foreign key of {table}.{name}: {e}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_migration_adds_the_counter_columns_and_drops_the_one_per_branch_index() {
    let (pool, schema, migrator) = schema().await;
    migrator
        .run_to(VERSION - 1, &pool)
        .await
        .expect("migrate up to the one before");
    sqlx::query(
        "INSERT INTO device_agents (id, branch_id, name, created_at, created_by) \
         VALUES ('c0000000-0000-4000-8000-00000000000a'::uuid, 'MAIN', 'Mac', now(), 'owner')",
    )
    .execute(&pool)
    .await
    .expect("a counter from before");
    migrator.run(&pool).await.expect("apply the rest");

    let index: Option<String> = sqlx::query_scalar(
        "SELECT indexname::text FROM pg_indexes WHERE schemaname = current_schema() \
         AND indexname = 'device_agents_one_active_per_branch_idx'",
    )
    .fetch_optional(&pool)
    .await
    .expect("read pg_indexes");
    assert_eq!(index, None, "M1");

    assert_eq!(
        column(&pool, "device_agents", "counter_name").await,
        ("text".to_owned(), false, Some("''::text".to_owned())),
        "M2"
    );
    for name in ["cash_drawer", "scanner", "kiosk_lock"] {
        assert_eq!(
            column(&pool, "device_agents", name).await,
            ("boolean".to_owned(), true, None),
            "M2 {name}"
        );
    }
    for (table, name) in [
        ("device_jobs", "target_agent_id"),
        ("user_sessions", "counter_agent_id"),
    ] {
        assert_eq!(
            column(&pool, table, name).await,
            ("uuid".to_owned(), true, None),
            "M3 {table}.{name}"
        );
        assert_eq!(
            references(&pool, table, name).await,
            ("device_agents".to_owned(), "id".to_owned()),
            "M3 {table}.{name}"
        );
    }

    let before: (String, Option<bool>, Option<bool>, Option<bool>) = sqlx::query_as(
        "SELECT counter_name, cash_drawer, scanner, kiosk_lock FROM device_agents \
         WHERE id = 'c0000000-0000-4000-8000-00000000000a'::uuid",
    )
    .fetch_one(&pool)
    .await
    .expect("read the counter");
    assert_eq!(before, (String::new(), None, None, None), "M4: no backfill");

    sqlx::query(
        "INSERT INTO device_agents (id, branch_id, name, created_at, created_by) \
         VALUES ('c0000000-0000-4000-8000-00000000000b'::uuid, 'MAIN', 'Mac 2', now(), 'owner')",
    )
    .execute(&pool)
    .await
    .expect("M1: a second active counter in MAIN");
    drop_schema(pool, &schema).await;
}
