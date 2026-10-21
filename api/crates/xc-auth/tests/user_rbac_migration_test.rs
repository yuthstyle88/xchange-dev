//! Up-front tests for the RBAC migration M (design `docs/design/blueprint/user-management-rbac.md` §4, PR 3083).
//! Written before the implementation by the ticket author (claude/f03345fe) for the BE controller claude/c81c8986.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Each test stages a schema of its own migrated up to, not including, M. It writes rows as today's binary writes
//! them, then applies M alone (`run_to(M_VERSION)`), so no later migration can hide what M did. The checks after M
//! read the schema, not the file text.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test`. Without it these
//! tests FAIL; none skips.
//!
//! Names these tests assume ([required] in the ticket):
//! - the file `api/migrations/20261218100000_user_rbac.sql`;
//! - `users.employee_code|department|position|tell_code text NULL`, unique per `(organization_id, lower(employee_code))`;
//! - `users.status` accepting `SUSPENDED`;
//! - `user_sessions.must_change_password boolean NOT NULL DEFAULT false`;
//! - `user_audit.before jsonb NULL`, `user_audit.after jsonb NULL`, `user_audit.ip inet NULL`;
//! - `login_history(id, organization_id, user_id, identifier_hash, ip, result, at)` with the result CHECK of §4.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

const M_FILE: &str = "20261218100000_user_rbac.sql";
const M_VERSION: i64 = 20261218100000;

/// §4: the seven results `login_history.result` accepts.
const RESULTS: [&str; 7] = [
    "OK",
    "BAD_PASSWORD",
    "UNKNOWN_USER",
    "NOT_ACTIVE",
    "TEMP_EXPIRED",
    "RATE_LIMITED",
    "UNAVAILABLE",
];

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
}

impl Staged {
    /// A schema of its own, migrated up to (not including) M.
    async fn before_m() -> Staged {
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
        let schema = unique("rbacm");
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
            migrator.version_exists(M_VERSION),
            "api/migrations/{M_FILE} must exist"
        );
        migrator
            .run_to(M_VERSION - 1, &pool)
            .await
            .expect("migrate up to the one before M");
        Staged {
            pool,
            schema,
            migrator,
        }
    }

    /// Applies M and nothing after it.
    async fn m(&self) {
        self.migrator
            .run_to(M_VERSION, &self.pool)
            .await
            .expect("M applies");
    }

    async fn exec(&self, sql: &str) -> Result<(), String> {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
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

    async fn text(&self, sql: &str) -> Option<String> {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}\n→ {e}"))
    }

    async fn org(&self) -> String {
        self.text("SELECT id::text FROM organizations WHERE code = 'XC'")
            .await
            .expect("the default organisation")
    }

    async fn second_org(&self) -> String {
        let id = "00000000-0000-0000-0000-0000000000b2";
        self.ok(&format!(
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ('{id}', 'ORG2', 'Asia/Bangkok', now()) ON CONFLICT (id) DO NOTHING"
        ))
        .await;
        id.to_owned()
    }

    /// A user as today's binary writes it; its id.
    async fn user(&self, org: &str, status: &str) -> String {
        self.text(&format!(
            "INSERT INTO users (organization_id, auth_subject, name, status, version, created_by) \
             VALUES ('{org}', '{}', 'คุณ ทดสอบ', '{status}', 1, 'test') RETURNING id::text",
            unique("sub")
        ))
        .await
        .expect("a user id")
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

/// M1 (§4 item 1): rows written before M keep working. The new user columns are NULL, an existing session reads
/// `must_change_password = false`, and an existing audit row has NULL `before`, `after` and `ip`.
#[tokio::test]
async fn m1_rows_written_before_m_read_as_not_recorded() {
    let db = Staged::before_m().await;
    let org = db.org().await;
    let user = db.user(&org, "ACTIVE").await;
    db.ok(&format!(
        "INSERT INTO user_sessions (hash, user_id, version, expires, organization_id) \
         VALUES ('h-before-m', '{user}', 1, now() + interval '1 hour', '{org}')"
    ))
    .await;
    db.ok(&format!(
        "INSERT INTO user_audit (actor_id, subject_id, action, organization_id) \
         VALUES ('owner', '{user}', 'grant', '{org}')"
    ))
    .await;

    db.m().await;

    assert_eq!(
        db.text(&format!(
            "SELECT concat_ws('|', coalesce(employee_code, 'NULL'), coalesce(department, 'NULL'), \
                    coalesce(position, 'NULL'), coalesce(tell_code, 'NULL'), status) \
             FROM users WHERE id = '{user}'"
        ))
        .await
        .as_deref(),
        Some("NULL|NULL|NULL|NULL|ACTIVE")
    );
    assert_eq!(
        db.text("SELECT must_change_password::text FROM user_sessions WHERE hash = 'h-before-m'")
            .await
            .as_deref(),
        Some("false")
    );
    assert_eq!(
        db.text(&format!(
            "SELECT concat_ws('|', (before IS NULL)::text, (after IS NULL)::text, (ip IS NULL)::text) \
             FROM user_audit WHERE subject_id = '{user}'"
        ))
        .await
        .as_deref(),
        Some("true|true|true")
    );
    db.finish().await;
}

/// M2 (§4 table, Q3 A): `users.status` accepts SUSPENDED, still refuses anything outside the four.
#[tokio::test]
async fn m2_status_accepts_suspended_and_still_refuses_unknown() {
    let db = Staged::before_m().await;
    let org = db.org().await;
    db.refused(&format!(
        "INSERT INTO users (organization_id, name, status, version, created_by) \
         VALUES ('{org}', 'x', 'SUSPENDED', 1, 'test')"
    ))
    .await;
    db.m().await;
    for status in ["PENDING", "ACTIVE", "SUSPENDED", "DISABLED"] {
        db.user(&org, status).await;
    }
    db.refused(&format!(
        "INSERT INTO users (organization_id, name, status, version, created_by) \
         VALUES ('{org}', 'x', 'INVITED', 1, 'test')"
    ))
    .await;
    db.refused(&format!(
        "INSERT INTO users (organization_id, name, status, version, created_by) \
         VALUES ('{org}', 'x', 'suspended', 1, 'test')"
    ))
    .await;
    db.finish().await;
}

/// M3 (§4 table): `employee_code` is unique per organisation, ignoring case. Many NULLs are allowed, and the same
/// code in two organisations is allowed.
#[tokio::test]
async fn m3_employee_code_is_unique_per_organisation_ignoring_case() {
    let db = Staged::before_m().await;
    db.m().await;
    let org = db.org().await;
    let org2 = db.second_org().await;
    let a = db.user(&org, "ACTIVE").await;
    let b = db.user(&org, "ACTIVE").await;
    let c = db.user(&org2, "ACTIVE").await;
    let d = db.user(&org, "ACTIVE").await;
    db.ok(&format!(
        "UPDATE users SET employee_code = 'TK-010326-MN002' WHERE id = '{a}'"
    ))
    .await;
    db.refused(&format!(
        "UPDATE users SET employee_code = 'tk-010326-mn002' WHERE id = '{b}'"
    ))
    .await;
    db.ok(&format!(
        "UPDATE users SET employee_code = 'tk-010326-mn002' WHERE id = '{c}'"
    ))
    .await;
    db.ok(&format!(
        "UPDATE users SET department = 'Front', position = 'Teller', tell_code = 'T7' WHERE id IN ('{b}', '{d}')"
    ))
    .await;
    db.finish().await;
}

/// M4 (§4 table, Q6 A): `login_history` takes each of the seven results, refuses any other, allows a row with no
/// organisation and no user (an unknown identifier), and requires `identifier_hash` and `result`.
#[tokio::test]
async fn m4_login_history_takes_the_seven_results_and_nothing_else() {
    let db = Staged::before_m().await;
    db.m().await;
    let org = db.org().await;
    let user = db.user(&org, "ACTIVE").await;
    for r in RESULTS {
        db.ok(&format!(
            "INSERT INTO login_history (organization_id, user_id, identifier_hash, ip, result) \
             VALUES ('{org}', '{user}', 'h', '203.0.113.7', '{r}')"
        ))
        .await;
    }
    db.ok(
        "INSERT INTO login_history (identifier_hash, result) VALUES ('h-unknown', 'UNKNOWN_USER')",
    )
    .await;
    db.refused("INSERT INTO login_history (identifier_hash, result) VALUES ('h', 'LOCKED')")
        .await;
    db.refused("INSERT INTO login_history (identifier_hash, result) VALUES ('h', 'ok')")
        .await;
    db.refused("INSERT INTO login_history (result) VALUES ('OK')")
        .await;
    db.refused("INSERT INTO login_history (identifier_hash) VALUES ('h')")
        .await;
    assert_eq!(
        db.text(
            "SELECT (at IS NOT NULL AND id IS NOT NULL)::text FROM login_history WHERE identifier_hash = 'h-unknown'"
        )
        .await
        .as_deref(),
        Some("true"),
        "`id` and `at` have defaults"
    );
    db.finish().await;
}

/// M5 (§4 table): `login_history` has an index on `(organization_id, at DESC)`, the order the login-history view
/// reads.
#[tokio::test]
async fn m5_login_history_is_indexed_by_organisation_and_newest_first() {
    let db = Staged::before_m().await;
    db.m().await;
    let defs = db
        .text(
            "SELECT string_agg(indexdef, ' ; ') FROM pg_indexes \
             WHERE schemaname = current_schema() AND tablename = 'login_history'",
        )
        .await
        .unwrap_or_default();
    assert!(
        defs.contains("(organization_id, at DESC)"),
        "no index on (organization_id, at DESC): {defs}"
    );
    db.finish().await;
}

/// M6 (§4 table): a new session gets `must_change_password = false` unless the writer says otherwise, and the
/// column refuses NULL.
#[tokio::test]
async fn m6_session_flag_defaults_to_false_and_is_not_null() {
    let db = Staged::before_m().await;
    db.m().await;
    let org = db.org().await;
    let user = db.user(&org, "ACTIVE").await;
    db.ok(&format!(
        "INSERT INTO user_sessions (hash, user_id, version, expires, organization_id) \
         VALUES ('h-default', '{user}', 1, now() + interval '1 hour', '{org}')"
    ))
    .await;
    db.ok(&format!(
        "INSERT INTO user_sessions (hash, user_id, version, expires, organization_id, must_change_password) \
         VALUES ('h-forced', '{user}', 1, now() + interval '1 hour', '{org}', true)"
    ))
    .await;
    assert_eq!(
        db.text(
            "SELECT string_agg(hash || '=' || must_change_password::text, ',' ORDER BY hash) FROM user_sessions"
        )
        .await
        .as_deref(),
        Some("h-default=false,h-forced=true")
    );
    db.refused(&format!(
        "INSERT INTO user_sessions (hash, user_id, version, expires, organization_id, must_change_password) \
         VALUES ('h-null', '{user}', 1, now() + interval '1 hour', '{org}', NULL)"
    ))
    .await;
    db.finish().await;
}

/// M7 (§4 table, §3.4): an audit row can carry `before` / `after` as JSON and the caller's `ip`.
#[tokio::test]
async fn m7_audit_rows_carry_before_after_and_ip() {
    let db = Staged::before_m().await;
    db.m().await;
    let org = db.org().await;
    let user = db.user(&org, "ACTIVE").await;
    db.ok(&format!(
        "INSERT INTO user_audit (actor_id, subject_id, action, organization_id, before, after, ip) \
         VALUES ('owner', '{user}', 'suspend', '{org}', '{{\"status\":\"ACTIVE\"}}', \
                 '{{\"status\":\"SUSPENDED\"}}', '203.0.113.7')"
    ))
    .await;
    assert_eq!(
        db.text(&format!(
            "SELECT concat_ws('|', before->>'status', after->>'status', host(ip)) FROM user_audit \
             WHERE subject_id = '{user}'"
        ))
        .await
        .as_deref(),
        Some("ACTIVE|SUSPENDED|203.0.113.7")
    );
    db.refused(&format!(
        "INSERT INTO user_audit (actor_id, subject_id, action, organization_id, ip) \
         VALUES ('owner', '{user}', 'x', '{org}', 'not-an-ip')"
    ))
    .await;
    db.finish().await;
}
