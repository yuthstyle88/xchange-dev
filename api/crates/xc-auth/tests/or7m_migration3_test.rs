//! OR-7.m (#1027) provided tests for migration 3, `api/migrations/<slot>_drop_password_login.sql`
//! (design `docs/design/blueprint/or7m-password-login-removal.md` §5). Written by the sub-controller
//! before the migration; the implementer must not edit, delete or weaken them.
//!
//! Each test migrates a schema of its own up to (not including) migration 3, seeds the old tables
//! as the pre-OR-7.m binary does, then applies migration 3 alone (`run_to`).
//!
//! Needs a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in `_test`). Without it
//! the tests FAIL; none skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

const NAME: &str = "drop_password_login";
const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

struct Staged {
    pool: PgPool,
    schema: String,
    migrator: Migrator,
    version: i64,
}

impl Staged {
    /// A schema of its own, migrated up to (not including) migration 3.
    async fn before_migration_3() -> Staged {
        let url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| panic!("TEST_DATABASE_URL is not set; these tests do not skip"));
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("or7m_m3");
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
        let version = migrator
            .iter()
            .find(|m| m.description == NAME.replace('_', " ") || m.description == NAME)
            .map(|m| m.version)
            .unwrap_or_else(|| {
                panic!("api/migrations has no `<version>_{NAME}.sql` (migration 3)")
            });
        migrator
            .run_to(version - 1, &pool)
            .await
            .expect("migrate up to migration 3");
        Staged {
            pool,
            schema,
            migrator,
            version,
        }
    }

    async fn migration_3(&self) -> Result<(), String> {
        self.migrator
            .run_to(self.version, &self.pool)
            .await
            .map_err(|e| e.to_string())
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn table_exists(&self, table: &str) -> bool {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = $1)",
        )
        .bind(table)
        .fetch_one(&self.pool)
        .await
        .expect("read information_schema.tables")
    }

    async fn column_exists(&self, table: &str, column: &str) -> bool {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = $1 AND column_name = $2)",
        )
        .bind(table)
        .bind(column)
        .fetch_one(&self.pool)
        .await
        .expect("read information_schema.columns")
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn finish(self) {
        let Staged { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }

    /// One owner, one employee mirrored into `users` (as migration 2 and `create_employee` did),
    /// their sessions, an audit row and an attempt window.
    async fn seed_mirrored(&self) {
        self.exec("INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN') ON CONFLICT DO NOTHING")
            .await;
        self.exec(
            "INSERT INTO owners (id, username, name, password_hash, active, version, created_at) \
             VALUES ('o1', 'boss', 'Boss', 'h', true, 1, now())",
        )
        .await;
        self.exec("INSERT INTO owner_sessions (hash, owner_id, version, expires) VALUES ('os1', 'o1', 1, now())")
            .await;
        self.exec(
            "INSERT INTO employees (id, username, name, role, branch_id, password_hash, active, must_change, version, created_at) \
             VALUES ('e1', 'anna', 'Anna', 'FRONT', 'MAIN', 'h', true, false, 1, now())",
        )
        .await;
        self.exec("INSERT INTO employee_sessions (hash, employee_id, version, expires) VALUES ('es1', 'e1', 1, now())")
            .await;
        self.exec(&format!(
            "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
             VALUES ('{ORG}'::uuid, 'Anna', 'ACTIVE', 'test', 'e1')"
        ))
        .await;
        self.exec("INSERT INTO employee_audit (actor, subject, action, at) VALUES ('o1', 'e1', 'create', now())")
            .await;
        self.exec(
            "INSERT INTO auth_attempts (key, window_start, count) VALUES ('AGENT:x', now(), 2)",
        )
        .await;
    }
}

const DROPPED: [&str; 4] = ["owner_sessions", "employee_sessions", "employees", "owners"];

#[tokio::test]
async fn migration_3_drops_password_login_and_keeps_the_rest() {
    let db = Staged::before_migration_3().await;
    db.seed_mirrored().await;

    db.migration_3().await.expect("migration 3 applies");

    for table in DROPPED {
        assert!(!db.table_exists(table).await, "{table} is dropped");
    }
    assert!(
        !db.column_exists("users", "legacy_employee_id").await,
        "users.legacy_employee_id is dropped"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM users WHERE name = 'Anna'")
            .await,
        1,
        "the user stays"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM employee_audit").await,
        1,
        "employee_audit stays, read-only history"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM auth_attempts").await,
        1,
        "auth_attempts stays (design §3 ruling 1)"
    );
    db.finish().await;
}

#[tokio::test]
async fn an_employee_with_no_user_stops_migration_3_and_drops_nothing() {
    let db = Staged::before_migration_3().await;
    db.seed_mirrored().await;
    db.exec(
        "INSERT INTO employees (id, username, name, role, branch_id, password_hash, active, must_change, version, created_at) \
         VALUES ('e2', 'bo', 'Bo', 'RATES', 'MAIN', 'h', true, false, 1, now())",
    )
    .await;

    let refused = db.migration_3().await;
    assert!(
        refused.is_err(),
        "an unmirrored employee must stop migration 3 (§5 item 2)"
    );

    for table in DROPPED {
        assert!(
            db.table_exists(table).await,
            "{table} is untouched after the refusal"
        );
    }
    assert!(
        db.column_exists("users", "legacy_employee_id").await,
        "users.legacy_employee_id is untouched after the refusal"
    );
    db.finish().await;
}

#[tokio::test]
async fn migration_3_applies_on_an_empty_database() {
    let db = Staged::before_migration_3().await;
    db.migration_3()
        .await
        .expect("migration 3 applies with no rows");
    for table in DROPPED {
        assert!(!db.table_exists(table).await, "{table} is dropped");
    }
    db.finish().await;
}
