//! Ticket #616 (OR-2) — migration 2 (`users`, scoped roles) and its backfill from `owners` and `employees`. Written before
//! the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! Design: `docs/design/blueprint/org-roles-branches.md` (#550, merged f0ec3ca) §0 B1 and B3 (the owner's answers),
//! §3.3 (v3 roles → Blueprint roles), §8 (tables), §9 item 2 (the migration: backfill, "an employee role outside
//! §3.3's ten is refused by the migration (`RAISE`), not guessed", `legacy_employee_id = e.id` so that migration 3's
//! precheck passes) and §9 item 3 (the precheck query, quoted below as `PRECHECK`). Follow-up decision 1 (PR 636,
//! `docs/org-roles-or2-followups`): migration 2 counts employees whose `branch_id` has no `branches` row and
//! `RAISE`s if there is any.
//!
//! Each test stages a schema of its own migrated up to, not including, migration 2 (OR-1's migration 1 has run),
//! writes `owners` and `employees` rows as the pre-OR-2 binary writes them (`20261007150000_accounts.sql`), then
//! applies migration 2 alone (`run_to`), so that no later migration can hide what migration 2 did.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test`. Without it these
//! tests FAIL; none skips.
//!
//! Signatures and names these tests assume:
//! - The file is `api/migrations/20261011110000_users_roles.sql` (§9 item 2, [required]); OR-1's
//!   `20261011100000_organizations.sql` creates `organizations` with one row of `code 'XC'` (§9 item 1).
//! - Tables and columns as §8 writes them: `users(id, organization_id, auth_subject, name, status, version,
//!   created_at, created_by, legacy_employee_id)`, `user_invites(user_id, code_hash, expires_at, used_at)`,
//!   `roles(code)`, `user_roles(id, user_id, role_code, organization_id, branch_id, valid_from, valid_to)`.
//!   Ids are read here as `::text`, so their SQL type is the implementer's.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

/// Migration 2 (§9 item 2).
const MIGRATION: &str = "20261011110000_users_roles.sql";
const MIGRATION_VERSION: i64 = 20261011110000;

/// Migration 3's precheck (§9 item 3), word for word. It must be 0 after migration 2.
const PRECHECK: &str = "SELECT count(*) FROM employees e WHERE NOT EXISTS (SELECT 1 FROM users u WHERE u.legacy_employee_id = e.id)";

/// §3.3 (B1): each v3 staff role and the one Blueprint role it becomes.
const MAPPED: [(&str, &str); 9] = [
    ("FRONT", "TELLER"),
    ("BACKEND", "BRANCH_MANAGER"),
    ("MANAGER", "BRANCH_MANAGER"),
    ("RATES", "RATE_MANAGER"),
    ("FINANCE", "FINANCE"),
    ("SYSTEM", "ADMIN"),
    ("MONITOR", "VERIFICATION_REVIEWER"),
    ("DOCUMENTS", "VERIFICATION_REVIEWER"),
    ("DOCUMENT_REVIEW", "VERIFICATION_REVIEWER"),
];

/// §3.1, the "v3 wire" column without PARTNER.
const BLUEPRINT_ROLES: [&str; 9] = [
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

const MAIN: &str = "MAIN";
const B2: &str = "B2";

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

fn created() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 7, 3, 0, 0).unwrap()
}

/// A schema of its own, migrated up to (not including) migration 2.
struct Staged {
    pool: PgPool,
    schema: String,
    migrator: Migrator,
}

impl Staged {
    async fn before_migration_2() -> Staged {
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
        let schema = unique("or2mig");
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
            migrator.version_exists(MIGRATION_VERSION),
            "api/migrations/{MIGRATION} exists"
        );
        migrator
            .run_to(MIGRATION_VERSION - 1, &pool)
            .await
            .expect("migrate up to the one before migration 2");
        Staged {
            pool,
            schema,
            migrator,
        }
    }

    /// Applies migration 2 and nothing after it.
    async fn migration_2(&self) -> Result<(), String> {
        self.migrator
            .run_to(MIGRATION_VERSION, &self.pool)
            .await
            .map_err(|e| e.to_string())
    }

    async fn owner(&self, id: &str, username: &str, name: &str) {
        sqlx::query(
            "INSERT INTO owners (id, username, name, password_hash, active, version, created_at) \
             VALUES ($1, $2, $3, 'argon2-hash', true, 1, $4)",
        )
        .bind(id)
        .bind(username)
        .bind(name)
        .bind(created())
        .execute(&self.pool)
        .await
        .expect("insert the owner as the pre-OR-2 binary does");
    }

    async fn employee(&self, id: &str, role: &str, branch: &str, active: bool) {
        sqlx::query(
            "INSERT INTO employees (id, username, name, role, branch_id, password_hash, active, \
             must_change, version, created_at) VALUES ($1, $2, $3, $4, $5, 'argon2-hash', $6, false, 3, $7)",
        )
        .bind(id)
        .bind(format!("user-{id}"))
        .bind(format!("คุณ {id}"))
        .bind(role)
        .bind(branch)
        .bind(active)
        .bind(created())
        .execute(&self.pool)
        .await
        .expect("insert an employee as the pre-OR-2 binary does");
    }

    async fn branch(&self, id: &str) {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(id)
            .execute(&self.pool)
            .await
            .expect("insert a branch as the pre-OR-1 binary does");
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn organization(&self) -> String {
        sqlx::query_scalar("SELECT id::text FROM organizations WHERE code = 'XC'")
            .fetch_one(&self.pool)
            .await
            .expect("OR-1's one organisation")
    }

    /// (status, auth_subject, name, organization_id) of the user backfilled from employee `id`.
    async fn user_of_employee(&self, id: &str) -> Vec<(String, Option<String>, String, String)> {
        sqlx::query_as(
            "SELECT status, auth_subject, name, organization_id::text FROM users \
             WHERE legacy_employee_id = $1",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .expect("read users")
    }

    /// (role_code, branch_id, organization_id, valid_to is NULL, valid_from <= now()) of the user backfilled
    /// from employee `id`, in role order.
    async fn roles_of_employee(
        &self,
        id: &str,
    ) -> Vec<(String, Option<String>, String, bool, bool)> {
        sqlx::query_as(
            "SELECT ur.role_code::text, ur.branch_id::text, ur.organization_id::text, \
                    ur.valid_to IS NULL, ur.valid_from <= now() \
             FROM user_roles ur JOIN users u ON u.id = ur.user_id \
             WHERE u.legacy_employee_id = $1 ORDER BY 1, 2",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .expect("read user_roles")
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

/// One employee per v3 staff role, spread over two branches, all active.
async fn one_of_each_role(db: &Staged) -> Vec<(String, &'static str, &'static str, &'static str)> {
    db.branch(B2).await;
    let mut made = Vec::new();
    for (i, (v3, blueprint)) in MAPPED.iter().enumerate() {
        let id = format!("e-{}", v3.to_lowercase());
        let branch = if i % 2 == 0 { MAIN } else { B2 };
        db.employee(&id, v3, branch, true).await;
        made.push((id, *v3, *blueprint, branch));
    }
    made
}

// ── the tables ───────────────────────────────────────────────────────────────

/// §8 `roles(code)`: the roles of §3.1, seeded. No v3 name and no PARTNER.
#[tokio::test]
async fn the_roles_table_holds_the_blueprint_roles() {
    let db = Staged::before_migration_2().await;
    db.migration_2()
        .await
        .expect("migration 2 on an empty install");
    let codes: Vec<String> = sqlx::query_scalar("SELECT code::text FROM roles ORDER BY 1")
        .fetch_all(&db.pool)
        .await
        .expect("read roles");
    assert_eq!(codes, BLUEPRINT_ROLES);
    assert_eq!(
        db.count("SELECT count(*) FROM users").await,
        0,
        "nothing to backfill on an install with no accounts"
    );
    db.finish().await;
}

// ── the backfill ─────────────────────────────────────────────────────────────

/// §3.3 (B1): each of the nine v3 staff roles becomes exactly one `user_roles` row of its Blueprint role, at the
/// employee's `branch_id`, in the one organisation; the user is `INVITED` with no `auth_subject`.
#[tokio::test]
async fn each_v3_role_is_mapped_by_duty_at_the_employees_branch() {
    let db = Staged::before_migration_2().await;
    let made = one_of_each_role(&db).await;
    db.migration_2().await.expect("migration 2");
    let org = db.organization().await;

    for (id, v3, blueprint, branch) in &made {
        assert_eq!(
            db.user_of_employee(id).await,
            vec![("INVITED".to_owned(), None, format!("คุณ {id}"), org.clone())],
            "{v3}: one INVITED user, no auth_subject, the employee's name, the one organisation"
        );
        assert_eq!(
            db.roles_of_employee(id).await,
            vec![(
                blueprint.to_string(),
                Some(branch.to_string()),
                org.clone(),
                true,
                true
            )],
            "{v3} → {blueprint} at {branch}, already valid, with no end"
        );
    }
    db.finish().await;
}

/// §3.3 (B3): the owner becomes one user holding EXECUTIVE and ADMIN, both organisation-wide; it is not an
/// employee, so it has no `legacy_employee_id`.
#[tokio::test]
async fn the_owner_becomes_executive_and_admin_organisation_wide() {
    let db = Staged::before_migration_2().await;
    db.owner("owner-1", "boss", "คุณ boss").await;
    db.employee("e-1", "FRONT", MAIN, true).await;
    db.migration_2().await.expect("migration 2");
    let org = db.organization().await;

    let owners: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT id::text, name, auth_subject FROM users WHERE legacy_employee_id IS NULL",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read users");
    assert_eq!(owners.len(), 1, "one user for the one owner: {owners:?}");
    let (owner, name, subject) = &owners[0];
    assert_eq!(name, "คุณ boss");
    assert_eq!(*subject, None, "nobody has claimed it yet");

    let roles: Vec<(String, Option<String>, String, bool)> = sqlx::query_as(
        "SELECT role_code::text, branch_id::text, organization_id::text, valid_to IS NULL \
         FROM user_roles WHERE user_id::text = $1 ORDER BY 1",
    )
    .bind(owner)
    .fetch_all(&db.pool)
    .await
    .expect("read the owner's roles");
    assert_eq!(
        roles,
        vec![
            ("ADMIN".to_owned(), None, org.clone(), true),
            ("EXECUTIVE".to_owned(), None, org.clone(), true),
        ],
        "EXECUTIVE + ADMIN, both with branch_id NULL"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM users").await,
        2,
        "the owner and the one employee"
    );
    db.finish().await;
}

/// §9 item 2: a disabled employee becomes `DISABLED`, and still gets its one role row.
#[tokio::test]
async fn a_disabled_employee_becomes_a_disabled_user() {
    let db = Staged::before_migration_2().await;
    db.employee("e-gone", "BACKEND", MAIN, false).await;
    db.employee("e-here", "BACKEND", MAIN, true).await;
    db.migration_2().await.expect("migration 2");

    let status = |rows: Vec<(String, Option<String>, String, String)>| -> Vec<String> {
        rows.into_iter().map(|r| r.0).collect()
    };
    assert_eq!(status(db.user_of_employee("e-gone").await), ["DISABLED"]);
    assert_eq!(status(db.user_of_employee("e-here").await), ["INVITED"]);
    assert_eq!(db.roles_of_employee("e-gone").await.len(), 1);
    db.finish().await;
}

/// §9 item 2: each backfilled employee gets an invite that has already expired (ADMIN reissues invites), unused.
#[tokio::test]
async fn every_backfilled_employee_gets_an_expired_invite() {
    let db = Staged::before_migration_2().await;
    let made = one_of_each_role(&db).await;
    db.migration_2().await.expect("migration 2");

    for (id, v3, _, _) in &made {
        let invites: Vec<(bool, bool)> = sqlx::query_as(
            "SELECT i.expires_at <= now(), i.used_at IS NULL \
             FROM user_invites i JOIN users u ON u.id = i.user_id WHERE u.legacy_employee_id = $1",
        )
        .bind(id)
        .fetch_all(&db.pool)
        .await
        .expect("read user_invites");
        assert_eq!(
            invites,
            vec![(true, true)],
            "{v3}: one expired, unused invite"
        );
    }
    db.finish().await;
}

/// §9 items 2 and 3: every backfilled employee has `legacy_employee_id = e.id`, so migration 3's precheck query
/// is 0 straight after migration 2, and there is one user per employee.
#[tokio::test]
async fn migration_3s_precheck_is_zero_after_migration_2() {
    let db = Staged::before_migration_2().await;
    let made = one_of_each_role(&db).await;
    db.employee("e-off", "FRONT", B2, false).await;
    db.owner("owner-1", "boss", "คุณ boss").await;
    db.migration_2().await.expect("migration 2");

    assert_eq!(db.count(PRECHECK).await, 0, "migration 3's precheck");
    assert_eq!(
        db.count("SELECT count(*) FROM users WHERE legacy_employee_id IS NOT NULL")
            .await,
        made.len() as i64 + 1,
        "one user per employee, the disabled one included"
    );
    assert_eq!(
        db.count(
            "SELECT count(*) FROM (SELECT legacy_employee_id FROM users \
             WHERE legacy_employee_id IS NOT NULL GROUP BY 1 HAVING count(*) > 1) d"
        )
        .await,
        0,
        "no employee is backfilled twice"
    );
    db.finish().await;
}

// ── a role the mapping does not know ─────────────────────────────────────────

/// §9 item 2: "An employee role outside §3.3's ten is refused by the migration (`RAISE`), not guessed." The
/// migration fails, and its transaction leaves nothing behind.
#[tokio::test]
async fn an_employee_role_outside_the_mapping_stops_the_migration() {
    for odd in ["CASHIER", "PARTNER", "front", ""] {
        let db = Staged::before_migration_2().await;
        db.employee("e-ok", "FRONT", MAIN, true).await;
        db.employee("e-odd", odd, MAIN, true).await;

        let ran = db.migration_2().await;
        assert!(
            ran.is_err(),
            "an employee with role {odd:?} must stop migration 2, not be guessed"
        );
        let users: Option<String> = sqlx::query_scalar("SELECT to_regclass('users')::text")
            .fetch_one(&db.pool)
            .await
            .expect("ask for the users table");
        assert_eq!(
            users, None,
            "role {odd:?}: the failed migration leaves no users table behind"
        );
        let recorded: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _sqlx_migrations WHERE version = $1 AND success",
        )
        .bind(MIGRATION_VERSION)
        .fetch_one(&db.pool)
        .await
        .expect("read _sqlx_migrations");
        assert_eq!(
            recorded, 0,
            "role {odd:?}: migration 2 is not recorded as applied"
        );
        db.finish().await;
    }
}

/// Follow-up decision 1 (PR 636): an employee whose `branch_id` names no `branches` row stops migration 2 with a
/// `RAISE`, and the failed migration leaves nothing behind. `employees.branch_id` has no FK, so such a row can
/// exist before migration 2.
#[tokio::test]
async fn an_employee_at_a_branch_with_no_row_stops_the_migration() {
    let db = Staged::before_migration_2().await;
    db.employee("e-ok", "FRONT", MAIN, true).await;
    db.employee("e-orphan", "FRONT", "NOWHERE", true).await;

    let ran = db.migration_2().await;
    assert!(
        ran.is_err(),
        "an employee at a branch with no row must stop migration 2"
    );
    let users: Option<String> = sqlx::query_scalar("SELECT to_regclass('users')::text")
        .fetch_one(&db.pool)
        .await
        .expect("ask for the users table");
    assert_eq!(
        users, None,
        "the failed migration leaves no users table behind"
    );
    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = $1")
            .bind(MIGRATION_VERSION)
            .fetch_one(&db.pool)
            .await
            .expect("read _sqlx_migrations");
    assert_eq!(recorded, 0, "migration 2 is not recorded at all");

    // Given its branch row, the same data migrates.
    db.branch("NOWHERE").await;
    db.migration_2()
        .await
        .expect("migration 2 once the branch exists");
    assert_eq!(db.count(PRECHECK).await, 0);
    db.finish().await;
}
