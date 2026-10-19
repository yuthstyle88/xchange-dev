//! Up-front tests for RBAC migration M2 (design `docs/design/blueprint/user-management-rbac.md` §11.5).
//! Written before the implementation by the sub-controller claude/65357afb on `feat/rbac-groups`.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Each test stages a schema of its own migrated up to, not including, M2, writes rows as today's binary writes
//! them, then applies M2 alone, so no later migration can hide what M2 did.
//!
//! M2's version is renumbered at `pr_open` (`scripts/migration-slot.sh`), so these tests find it by its name: the one
//! file in `api/migrations/` whose name ends `_role_groups.sql` (sqlx description `role groups`).
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test`. Without it these
//! tests FAIL; none skips.
//!
//! Names these tests assume ([required] in the ticket):
//! - `roles.name text NOT NULL`, `roles.base_role text NOT NULL REFERENCES roles(code)`,
//!   `roles.built_in boolean NOT NULL DEFAULT false`;
//! - `role_permission_grants (role_code text REFERENCES roles(code) ON DELETE CASCADE, permission text,
//!   PRIMARY KEY (role_code, permission))`;
//! - `role_grant_version (version bigint NOT NULL)` holding exactly one row, `version = 0` after M2;
//! - a `-- down:` block at the end of the file, every line of it starting `-- `, that refuses while any
//!   `user_roles` row names a group that is not built in, and otherwise drops what M2 added.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

const SUFFIX: &str = "_role_groups.sql";

/// The nine `roles` rows before M2 (`20261011110000_users_roles.sql`).
const BUILT_IN: [&str; 9] = [
    "ADMIN",
    "BRANCH_MANAGER",
    "CUSTOMER",
    "EXECUTIVE",
    "FINANCE",
    "MARKETING",
    "RATE_MANAGER",
    "TELLER",
    "VERIFICATION_REVIEWER",
];

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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

/// M2's file: exactly one in `api/migrations/` ends with [`SUFFIX`].
fn m2_file() -> (i64, String) {
    let mut found: Vec<(i64, String)> = std::fs::read_dir(migrations_dir())
        .expect("read api/migrations")
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            let version = name.strip_suffix(SUFFIX)?.parse().ok()?;
            Some((version, name))
        })
        .collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one api/migrations/<version>{SUFFIX} must exist, found {found:?}"
    );
    found.pop().expect("one")
}

/// The `-- down:` block of M2, uncommented.
fn m2_down() -> String {
    let (_, name) = m2_file();
    let text = std::fs::read_to_string(migrations_dir().join(&name)).expect("read M2");
    let (_, down) = text
        .split_once("\n-- down:\n")
        .unwrap_or_else(|| panic!("{name} must end with a `-- down:` block"));
    down.lines()
        .map(|l| {
            l.strip_prefix("-- ")
                .or_else(|| l.strip_prefix("--"))
                .unwrap_or_else(|| panic!("every line of the down block starts `-- `: {l:?}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct Staged {
    pool: PgPool,
    schema: String,
    migrator: Migrator,
    version: i64,
}

impl Staged {
    async fn before_m2() -> Staged {
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
        let (version, _) = m2_file();
        let schema = unique("rbacm2");
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
        migrator
            .run_to(version - 1, &pool)
            .await
            .expect("migrate up to the one before M2");
        Staged {
            pool,
            schema,
            migrator,
            version,
        }
    }

    async fn m2(&self) {
        self.migrator
            .run_to(self.version, &self.pool)
            .await
            .expect("M2 applies");
    }

    async fn exec(&self, sql: &str) -> Result<(), String> {
        sqlx::raw_sql(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn ok(&self, sql: &str) {
        self.exec(sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}\n→ {e}"));
    }

    async fn refused(&self, sql: &str) {
        assert!(
            self.exec(sql).await.is_err(),
            "the database must refuse:\n{sql}"
        );
    }

    async fn int(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}\n→ {e}"))
    }

    async fn org(&self) -> String {
        sqlx::query_scalar("SELECT id::text FROM organizations WHERE code = 'XC'")
            .fetch_one(&self.pool)
            .await
            .expect("the default organisation")
    }

    /// A user holding `role_code` organisation-wide, as today's binary writes it.
    async fn user_holding(&self, role_code: &str) -> String {
        let org = self.org().await;
        let user: String = sqlx::query_scalar(AssertSqlSafe(format!(
            "INSERT INTO users (organization_id, auth_subject, name, status, version, created_by) \
             VALUES ('{org}', '{}', 'คุณ ทดสอบ', 'ACTIVE', 1, 'test') RETURNING id::text",
            unique("sub")
        )))
        .fetch_one(&self.pool)
        .await
        .expect("a user");
        self.ok(&format!(
            "INSERT INTO user_roles (id, user_id, role_code, organization_id, branch_id, valid_from, valid_to) \
             VALUES (gen_random_uuid(), '{user}', '{role_code}', '{org}', NULL, now(), NULL)"
        ))
        .await;
        user
    }

    async fn finish(self) {
        let Self { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

/// §11.5 item 1: the nine rows before M2 become built-in groups whose name and base are their code.
#[tokio::test]
async fn m2_backfills_the_nine_roles_as_built_in_groups_of_themselves() {
    let db = Staged::before_m2().await;
    assert_eq!(
        db.int("SELECT count(*) FROM roles").await,
        9,
        "nine roles before M2"
    );
    db.m2().await;
    assert_eq!(
        db.int("SELECT count(*) FROM roles").await,
        9,
        "M2 adds no role"
    );
    for code in BUILT_IN {
        let n = db
            .int(&format!(
                "SELECT count(*) FROM roles WHERE code = '{code}' AND name = '{code}' \
                 AND base_role = '{code}' AND built_in"
            ))
            .await;
        assert_eq!(n, 1, "{code} is a built-in group named and based on itself");
    }
    db.finish().await;
}

/// §11.5 item 1: `user_roles` is unchanged, and an assignment made before M2 still names its group.
#[tokio::test]
async fn m2_leaves_user_roles_as_they_were() {
    let db = Staged::before_m2().await;
    db.user_holding("TELLER").await;
    db.user_holding("EXECUTIVE").await;
    let before = db.int("SELECT count(*) FROM user_roles").await;
    db.m2().await;
    assert_eq!(db.int("SELECT count(*) FROM user_roles").await, before);
    assert_eq!(
        db.int("SELECT count(*) FROM user_roles ur JOIN roles r ON r.code = ur.role_code WHERE r.built_in")
            .await,
        before,
        "every assignment names a built-in group"
    );
    db.finish().await;
}

/// §11.5: the new columns are NOT NULL, `built_in` defaults to false, and `base_role` must name a group.
#[tokio::test]
async fn m2_roles_columns_hold_their_constraints() {
    let db = Staged::before_m2().await;
    db.m2().await;
    db.ok("INSERT INTO roles (code, name, base_role) VALUES ('NIGHT_TELLER', 'Night teller', 'TELLER')")
        .await;
    assert_eq!(
        db.int("SELECT count(*) FROM roles WHERE code = 'NIGHT_TELLER' AND NOT built_in")
            .await,
        1,
        "built_in defaults to false"
    );
    db.refused("INSERT INTO roles (code, base_role) VALUES ('NO_NAME', 'TELLER')")
        .await;
    db.refused("INSERT INTO roles (code, name) VALUES ('NO_BASE', 'No base')")
        .await;
    db.refused(
        "INSERT INTO roles (code, name, base_role) VALUES ('BAD_BASE', 'Bad', 'NOT_A_GROUP')",
    )
    .await;
    db.finish().await;
}

/// §11.5: `role_permission_grants` is keyed by (group, permission), names a group, and goes with it.
#[tokio::test]
async fn m2_grants_table_is_keyed_and_cascades_with_its_group() {
    let db = Staged::before_m2().await;
    db.m2().await;
    assert_eq!(
        db.int("SELECT count(*) FROM role_permission_grants").await,
        0,
        "the permission seed is not SQL (§11.5 item 2)"
    );
    db.ok("INSERT INTO roles (code, name, base_role) VALUES ('NIGHT_TELLER', 'Night teller', 'TELLER')")
        .await;
    db.ok("INSERT INTO role_permission_grants (role_code, permission) VALUES ('NIGHT_TELLER', 'ExchangeTransact')")
        .await;
    db.refused(
        "INSERT INTO role_permission_grants (role_code, permission) VALUES ('NIGHT_TELLER', 'ExchangeTransact')",
    )
    .await;
    db.refused("INSERT INTO role_permission_grants (role_code, permission) VALUES ('NOT_A_GROUP', 'SignedIn')")
        .await;
    db.ok("DELETE FROM roles WHERE code = 'NIGHT_TELLER'").await;
    assert_eq!(
        db.int("SELECT count(*) FROM role_permission_grants WHERE role_code = 'NIGHT_TELLER'")
            .await,
        0,
        "a group's grants go with it"
    );
    db.finish().await;
}

/// §11.5: `role_grant_version` is one row, at 0 after M2.
#[tokio::test]
async fn m2_grant_version_is_one_row_at_zero() {
    let db = Staged::before_m2().await;
    db.m2().await;
    assert_eq!(db.int("SELECT count(*) FROM role_grant_version").await, 1);
    assert_eq!(db.int("SELECT version FROM role_grant_version").await, 0);
    db.finish().await;
}

/// §11.5 item 4: the reversal refuses while a user holds a custom group.
#[tokio::test]
async fn m2_down_refuses_while_a_custom_group_is_assigned() {
    let db = Staged::before_m2().await;
    db.m2().await;
    db.ok("INSERT INTO roles (code, name, base_role) VALUES ('NIGHT_TELLER', 'Night teller', 'TELLER')")
        .await;
    db.user_holding("NIGHT_TELLER").await;
    db.refused(&m2_down()).await;
    assert_eq!(
        db.int("SELECT count(*) FROM roles WHERE code = 'NIGHT_TELLER'")
            .await,
        1,
        "a refused reversal changes nothing"
    );
    db.finish().await;
}

/// §11.5 item 4: with no custom group assigned, the reversal drops what M2 added and keeps the nine roles and every
/// assignment.
#[tokio::test]
async fn m2_down_drops_what_m2_added_when_no_custom_group_is_assigned() {
    let db = Staged::before_m2().await;
    db.user_holding("TELLER").await;
    db.m2().await;
    db.ok("INSERT INTO roles (code, name, base_role) VALUES ('UNUSED_GROUP', 'Unused', 'TELLER')")
        .await;
    db.ok(&m2_down()).await;
    assert_eq!(
        db.int("SELECT count(*) FROM roles").await,
        9,
        "custom groups are lost, the nine stay"
    );
    assert_eq!(db.int("SELECT count(*) FROM user_roles").await, 1);
    assert_eq!(
        db.int(
            "SELECT count(*) FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = 'roles' \
             AND column_name IN ('name', 'base_role', 'built_in')"
        )
        .await,
        0,
        "the new columns are gone"
    );
    assert_eq!(
        db.int(
            "SELECT count(*) FROM information_schema.tables \
             WHERE table_schema = current_schema() \
             AND table_name IN ('role_permission_grants', 'role_grant_version')"
        )
        .await,
        0,
        "the new tables are gone"
    );
    db.finish().await;
}
