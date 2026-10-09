//! Ticket #615 (OR-1) — migration 1 (`api/migrations/20261011100000_organizations.sql`) against a real PostgreSQL, and the
//! branch profile the OR-1 binary reads and writes through it.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/org-roles-branches.md` (#550, merged f0ec3ca) D-1 (every entity carries
//! `organization_id`; tables with `branch_id` get the composite FK `(organization_id, branch_id)` →
//! `branches(organization_id, id)`, the others an FK to `organizations(id)`; `branch_versions` and `employees` are
//! exempt from the composite FK by the designer's ruling on OR-1, see `NO_COMPOSITE_FK`; the column's DEFAULT is the
//! organisation the migration creates), §6 (the `Branch` profile), §8 (tables), §9 migration 1 (one organisation
//! `XC` in `Asia/Bangkok` with `legal_name` NULL; `code` backfilled from `id` and still nullable; `timezone text NOT
//! NULL DEFAULT 'Asia/Bangkok'`; `opening_hours jsonb NULL`; `booking_enabled bool NOT NULL DEFAULT false`; an
//! insert naming only `(id, name, …)` still works; the OR-1 binary writes `code`, the id) and §10 OR-1 (a row
//! naming another organisation's branch is refused by the composite FK).
//!
//! Signatures these tests assume ([proposed] where the design does not state them):
//! - The migration's version is `20261011100000` (§9).
//! - Constraint names are not pinned (§8 names none): the foreign keys are found by their columns, and the
//!   unique rules by the INSERT they refuse.
//! - `organizations(id uuid, code text, legal_name text NULL, timezone text, created_at)` (§8). The tests insert a
//!   second organisation naming all five columns.
//! - `xc_rate::domain::branch::entities::branch::Branch` gains the public fields `code: Option<String>`,
//!   `address: Option<String>`, `timezone: String`, `opening_hours: Option<serde_json::Value>` and
//!   `booking_enabled: bool` (§6, §7), and `Branch::hours(&self) -> pos_error::AppResult<BranchHours>` [proposed]
//!   built from its `timezone` and `opening_hours` (`branch_hours_test.rs`). `organization_id` is not pinned on the
//!   Rust `Branch`: no OR-1 reader needs it and `xc-rate` has no uuid type.
//! - `BranchService::{create, require, list}` keep their signatures; `create` writes `code` = the new id (§9).
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing database whose name ends in
//! `_test`. When it is absent every test FAILS; none skips. Each test runs in a schema of its own, so a test can
//! stop the migrations just before migration 1, write rows, and then apply the rest.

use std::borrow::Cow;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use pos_infra::transactions::Transactor;
use serde_json::json;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;
use xc_rate::domain::branch::branch_hours::HoursWindow;
use xc_rate::domain::branch::commands::create_branch::CreateBranch;

/// §9: migration 1's version.
const MIGRATION_1: i64 = 20261011100000;
const MUST_FINISH: Duration = Duration::from_secs(20);

static NEXT: AtomicU64 = AtomicU64::new(0);
static STARTING: Semaphore = Semaphore::const_new(4);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn url() -> String {
    std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "TEST_DATABASE_URL is not set.\n\
             These tests need a real PostgreSQL and do not skip. Point it at an existing database whose \
             name ends in `_test`, e.g. TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
        )
    })
}

struct Db {
    pool: PgPool,
    schema: String,
}

impl Db {
    /// An empty schema of its own; no migration applied yet.
    async fn empty() -> Self {
        let options =
            PgConnectOptions::from_str(&url()).expect("TEST_DATABASE_URL is a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("or1");
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
        Self { pool, schema }
    }

    /// Apply `api/migrations/` up to and including `last` (every one when `None`).
    async fn migrate_through(&self, last: Option<i64>) {
        let _slot = STARTING.acquire().await.expect("never closed");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let mut migrator = Migrator::new(dir.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()));
        assert!(
            migrator.migrations.iter().any(|m| m.version == MIGRATION_1),
            "api/migrations has no migration {MIGRATION_1} (§9 migration 1)"
        );
        if let Some(last) = last {
            let kept: Vec<_> = migrator
                .migrations
                .iter()
                .filter(|m| m.version <= last)
                .cloned()
                .collect();
            migrator.migrations = Cow::Owned(kept);
        }
        migrator
            .run(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("apply api/migrations through {last:?}: {e}"));
    }

    /// Every migration applied.
    async fn start() -> Self {
        let db = Self::empty().await;
        db.migrate_through(None).await;
        db
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// The SQLSTATE `sql` fails with, or `"ok"` when it succeeds.
    async fn outcome(&self, sql: &str) -> String {
        match sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
        {
            Ok(_) => "ok".to_owned(),
            Err(e) => match e.as_database_error().and_then(|d| d.code()) {
                Some(code) => code.into_owned(),
                None => panic!("{sql}: not a database error: {e}"),
            },
        }
    }

    async fn text(&self, sql: &str) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// The id of the organisation migration 1 created.
    async fn install_org(&self) -> String {
        self.text("SELECT id::text FROM organizations WHERE code = 'XC'")
            .await
            .expect("the XC organisation has an id")
    }

    /// A second organisation, as a later install or ADMIN would add one. Returns its id.
    async fn other_org(&self, code: &str) -> String {
        self.text(&format!(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES (gen_random_uuid(), '{code}', NULL, 'Asia/Bangkok', now()) RETURNING id::text"
        ))
        .await
        .expect("an id")
    }

    /// Base tables of this schema, migration bookkeeping and `organizations` left out.
    async fn tables(&self) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT table_name::text FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_type = 'BASE TABLE' \
               AND table_name NOT IN ('_sqlx_migrations', 'organizations') ORDER BY 1",
        )
        .fetch_all(&self.pool)
        .await
        .expect("list tables")
    }

    /// `(data_type, is_nullable, column_default)` of one column, `None` when it does not exist.
    async fn column(&self, table: &str, column: &str) -> Option<(String, String, Option<String>)> {
        sqlx::query_as(
            "SELECT data_type::text, is_nullable::text, column_default::text \
             FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = $1 AND column_name = $2",
        )
        .bind(table)
        .bind(column)
        .fetch_optional(&self.pool)
        .await
        .expect("read information_schema.columns")
    }

    /// What a column's DEFAULT evaluates to, as text.
    async fn default_value(&self, table: &str, column: &str) -> Option<String> {
        let (_, _, default) = self
            .column(table, column)
            .await
            .unwrap_or_else(|| panic!("{table}.{column} does not exist"));
        let default = default?;
        self.text(&format!("SELECT ({default})::text")).await
    }

    /// Every foreign key of this schema: (table, referenced table, sorted "column->referenced column" pairs).
    async fn foreign_keys(&self) -> Vec<(String, String, Vec<String>)> {
        sqlx::query_as(
            "SELECT cl.relname::text, rf.relname::text, \
                    (SELECT array_agg(sa.attname::text || '->' || da.attname::text \
                                      ORDER BY sa.attname::text) \
                       FROM unnest(c.conkey, c.confkey) AS k(s, d) \
                       JOIN pg_attribute sa ON sa.attrelid = c.conrelid AND sa.attnum = k.s \
                       JOIN pg_attribute da ON da.attrelid = c.confrelid AND da.attnum = k.d) \
             FROM pg_constraint c \
             JOIN pg_class cl ON cl.oid = c.conrelid \
             JOIN pg_namespace n ON n.oid = cl.relnamespace \
             JOIN pg_class rf ON rf.oid = c.confrelid \
             WHERE c.contype = 'f' AND n.nspname = current_schema()",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read pg_constraint")
    }

    fn branches(&self) -> BranchService {
        BranchService::new(BranchTx::new(Transactor::new(self.pool.clone())))
    }

    /// Drop the schema. Reached only by a test that passed.
    async fn finish(self) {
        self.pool.close().await;
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url())
            .await
            .expect("reconnect to drop the test schema");
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .expect("drop the test schema");
        admin.close().await;
    }
}

fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("{text:?}: {e}"))
        .with_timezone(&Utc)
}

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap_or_else(|e| panic!("{text:?}: {e}"))
}

const FOREIGN_KEY_VIOLATION: &str = "23503";
const UNIQUE_VIOLATION: &str = "23505";

// ── the organisation ────────────────────────────────────────────────────────

#[tokio::test]
async fn migration_1_creates_one_organisation_xc_in_bangkok_with_no_legal_name() {
    let db = Db::start().await;
    assert_eq!(db.count("SELECT count(*) FROM organizations").await, 1);
    let (code, timezone, legal_name): (String, String, Option<String>) =
        sqlx::query_as("SELECT code, timezone, legal_name FROM organizations")
            .fetch_one(&db.pool)
            .await
            .expect("the one organisation");
    assert_eq!(code, "XC");
    assert_eq!(timezone, "Asia/Bangkok");
    assert_eq!(legal_name, None, "legal_name is NULL until ADMIN sets it");

    let (id_type, _, _) = db
        .column("organizations", "id")
        .await
        .expect("organizations.id");
    assert_eq!(id_type, "uuid");
    let (_, legal_nullable, _) = db
        .column("organizations", "legal_name")
        .await
        .expect("organizations.legal_name");
    assert_eq!(legal_nullable, "YES");
    db.finish().await;
}

#[tokio::test]
async fn an_organisation_code_is_unique() {
    let db = Db::start().await;
    let again = db
        .outcome(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES (gen_random_uuid(), 'XC', NULL, 'Asia/Bangkok', now())",
        )
        .await;
    assert_eq!(again, UNIQUE_VIOLATION, "a second XC");
    db.other_org("OTHER").await;
    assert_eq!(db.count("SELECT count(*) FROM organizations").await, 2);
    db.finish().await;
}

// ── the branch columns ──────────────────────────────────────────────────────

#[tokio::test]
async fn the_branch_columns_are_the_ones_section_9_adds() {
    let db = Db::start().await;
    let org = db.install_org().await;

    let col = |c: &'static str| {
        let db = &db;
        async move {
            db.column("branches", c)
                .await
                .unwrap_or_else(|| panic!("branches.{c} does not exist"))
        }
    };

    let (t, nullable, _) = col("code").await;
    assert_eq!(
        (t.as_str(), nullable.as_str()),
        ("text", "YES"),
        "code stays nullable in this migration"
    );
    let (t, nullable, _) = col("address").await;
    assert_eq!((t.as_str(), nullable.as_str()), ("text", "YES"), "address");
    let (t, nullable, _) = col("timezone").await;
    assert_eq!((t.as_str(), nullable.as_str()), ("text", "NO"), "timezone");
    assert_eq!(
        db.default_value("branches", "timezone").await.as_deref(),
        Some("Asia/Bangkok")
    );
    let (t, nullable, default) = col("opening_hours").await;
    assert_eq!(
        (t.as_str(), nullable.as_str()),
        ("jsonb", "YES"),
        "opening_hours"
    );
    assert_eq!(
        default, None,
        "opening_hours has no default: NULL is \"not set\" (D-6, B4)"
    );
    let (t, nullable, _) = col("booking_enabled").await;
    assert_eq!(
        (t.as_str(), nullable.as_str()),
        ("boolean", "NO"),
        "booking_enabled"
    );
    assert_eq!(
        db.default_value("branches", "booking_enabled")
            .await
            .as_deref(),
        Some("false")
    );
    let (t, nullable, _) = col("organization_id").await;
    assert_eq!(
        (t.as_str(), nullable.as_str()),
        ("uuid", "NO"),
        "organization_id"
    );
    assert_eq!(
        db.default_value("branches", "organization_id").await,
        Some(org)
    );
    db.finish().await;
}

#[tokio::test]
async fn rows_written_before_migration_1_belong_to_the_organisation_and_get_their_id_as_code() {
    let db = Db::empty().await;
    db.migrate_through(Some(MIGRATION_1 - 1)).await;
    // What the pre-OR-1 binary wrote.
    db.exec(
        "INSERT INTO branches (id, name, created_at, created_by, request_id) \
         VALUES ('Bold-0000000000000001', 'Old Town', now(), 'owner', 'old-0000000000000001')",
    )
    .await;
    db.exec(
        "INSERT INTO abandoned_requests (branch_id, request_id, at, actor) \
         VALUES ('Bold-0000000000000001', 'gone-000000000000001', now(), 'owner')",
    )
    .await;

    db.migrate_through(None).await;
    let org = db.install_org().await;

    let rows: Vec<(
        String,
        Option<String>,
        String,
        String,
        Option<serde_json::Value>,
        bool,
    )> = sqlx::query_as(
        "SELECT id, code, organization_id::text, timezone, opening_hours, booking_enabled \
             FROM branches ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read branches");
    let ids: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(ids, ["Bold-0000000000000001", "MAIN"]);
    for (id, code, organization, timezone, opening_hours, booking_enabled) in rows {
        assert_eq!(
            code.as_deref(),
            Some(id.as_str()),
            "{id}: code is backfilled from id"
        );
        assert_eq!(organization, org, "{id}: belongs to the one organisation");
        assert_eq!(timezone, "Asia/Bangkok", "{id}");
        assert_eq!(opening_hours, None, "{id}: hours not set (B4)");
        assert!(!booking_enabled, "{id}");
    }

    let abandoned_org = db
        .text("SELECT organization_id::text FROM abandoned_requests")
        .await;
    assert_eq!(abandoned_org, Some(org), "a branch-scoped row too");
    db.finish().await;
}

#[tokio::test]
async fn an_insert_naming_only_the_old_columns_still_works() {
    let db = Db::start().await;
    let org = db.install_org().await;
    // `tests/common` seeds this way; the pre-OR-1 repository inserts the second way.
    db.exec("INSERT INTO branches (id, name) VALUES ('Bseed-000000000000001', 'Seeded')")
        .await;
    db.exec(
        "INSERT INTO branches (id, name, created_at, created_by, request_id) \
         VALUES ('Bold-0000000000000002', 'Riverside', now(), 'owner', 'old-0000000000000002')",
    )
    .await;
    db.exec(
        "INSERT INTO abandoned_requests (branch_id, request_id, at, actor) \
         VALUES ('Bold-0000000000000002', 'gone-000000000000002', now(), 'owner')",
    )
    .await;

    let rows: Vec<(
        Option<String>,
        String,
        String,
        Option<serde_json::Value>,
        bool,
    )> = sqlx::query_as(
        "SELECT code, organization_id::text, timezone, opening_hours, booking_enabled \
             FROM branches WHERE id IN ('Bseed-000000000000001', 'Bold-0000000000000002')",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read branches");
    assert_eq!(rows.len(), 2);
    for (code, organization, timezone, opening_hours, booking_enabled) in rows {
        assert_eq!(
            code, None,
            "a writer that leaves code out is accepted: code stays nullable (§9)"
        );
        assert_eq!(organization, org);
        assert_eq!(timezone, "Asia/Bangkok");
        assert_eq!(opening_hours, None);
        assert!(!booking_enabled);
    }
    assert_eq!(
        db.text("SELECT organization_id::text FROM abandoned_requests")
            .await,
        Some(org)
    );
    db.finish().await;
}

#[tokio::test]
async fn a_branch_code_is_unique_within_an_organisation_not_across_them() {
    let db = Db::start().await;
    let other = db.other_org("OTHER").await;
    db.exec("INSERT INTO branches (id, name, code) VALUES ('Bcode-000000000000001', 'One', 'DUP')")
        .await;
    assert_eq!(
        db.outcome(
            "INSERT INTO branches (id, name, code) VALUES ('Bcode-000000000000002', 'Two', 'DUP')"
        )
        .await,
        UNIQUE_VIOLATION,
        "the same code twice in one organisation"
    );
    assert_eq!(
        db.outcome(&format!(
            "INSERT INTO branches (id, name, code, organization_id) \
             VALUES ('Bcode-000000000000003', 'Three', 'DUP', '{other}')"
        ))
        .await,
        "ok",
        "the same code in another organisation"
    );
    db.finish().await;
}

// ── organization_id on every table ──────────────────────────────────────────

#[tokio::test]
async fn every_table_carries_the_organisation_with_its_default() {
    let db = Db::empty().await;
    // Every table that exists when migration 1 runs; later migrations answer for their own tables.
    db.migrate_through(Some(MIGRATION_1)).await;
    let org = db.install_org().await;
    let tables = db.tables().await;
    assert!(
        tables.len() >= 30,
        "expected the v3 schema, found {tables:?}"
    );

    let mut wrong = Vec::new();
    for table in &tables {
        match db.column(table, "organization_id").await {
            None => wrong.push(format!("{table}: no organization_id")),
            Some((t, nullable, _)) => {
                if (t.as_str(), nullable.as_str()) != ("uuid", "NO") {
                    wrong.push(format!(
                        "{table}: organization_id is {t}, nullable {nullable}"
                    ));
                }
                let default = db.default_value(table, "organization_id").await;
                if default.as_deref() != Some(org.as_str()) {
                    wrong.push(format!(
                        "{table}: organization_id defaults to {default:?}, not the install organisation"
                    ));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    db.finish().await;
}

/// Tables with a `branch_id` that get no composite FK, by the designer's ruling on OR-1 (claude/e5ab6c,
/// 2026-10-09). Each still carries `organization_id` with its FK to `organizations(id)`.
const NO_COMPOSITE_FK: [(&str, &str); 2] = [
    (
        "branch_versions",
        "the kernel's lock row for any branch id, by contract (v3_view_unknown_branch_test.rs:131-142)",
    ),
    (
        "employees",
        "the old model OR-7 retires; its branch check moves to the writer (OR-2, PR 636)",
    ),
];

#[tokio::test]
async fn every_table_is_tied_to_organisations_and_branch_tables_by_the_composite_key() {
    let db = Db::empty().await;
    db.migrate_through(Some(MIGRATION_1)).await;
    let tables = db.tables().await;
    let fks = db.foreign_keys().await;
    let has = |table: &str, target: &str, pairs: &[&str]| {
        fks.iter().any(|(t, r, p)| {
            t == table && r == target && p.iter().map(String::as_str).eq(pairs.iter().copied())
        })
    };
    let composite = ["branch_id->id", "organization_id->organization_id"];
    let to_organisations = ["organization_id->id"];

    let mut wrong = Vec::new();
    let mut with_branch = 0;
    for table in &tables {
        let has_branch = table != "branches" && db.column(table, "branch_id").await.is_some();
        let exempt = NO_COMPOSITE_FK.iter().find(|(t, _)| t == table);
        match (has_branch, exempt) {
            (true, None) => {
                with_branch += 1;
                if !has(table, "branches", &composite) {
                    wrong.push(format!(
                        "{table}: no FOREIGN KEY (organization_id, branch_id) REFERENCES branches (organization_id, id)"
                    ));
                }
            }
            (true, Some((_, why))) => {
                if has(table, "branches", &composite) {
                    wrong.push(format!(
                        "{table}: has the composite FK to branches, but is exempt: {why}"
                    ));
                }
                if !has(table, "organizations", &to_organisations) {
                    wrong.push(format!(
                        "{table}: no FOREIGN KEY (organization_id) REFERENCES organizations (id)"
                    ));
                }
            }
            (false, Some(_)) => wrong.push(format!(
                "{table}: listed as exempt from the composite FK, but has no branch_id"
            )),
            (false, None) => {
                if !has(table, "organizations", &to_organisations) {
                    wrong.push(format!(
                        "{table}: no FOREIGN KEY (organization_id) REFERENCES organizations (id)"
                    ));
                }
            }
        }
    }
    for (table, _) in NO_COMPOSITE_FK {
        if !tables.iter().any(|t| t == table) {
            wrong.push(format!("{table}: listed as exempt, but does not exist"));
        }
    }
    assert!(
        with_branch >= 20,
        "expected the v3 branch-scoped tables, found {with_branch}"
    );
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    db.finish().await;
}

#[tokio::test]
async fn a_row_naming_another_organisations_branch_is_refused() {
    let db = Db::start().await;
    let other = db.other_org("OTHER").await;
    db.exec(&format!(
        "INSERT INTO branches (id, name, organization_id) \
         VALUES ('Bother-00000000000001', 'Elsewhere', '{other}')"
    ))
    .await;

    // The row takes the default organisation (XC) but names OTHER's branch.
    assert_eq!(
        db.outcome(
            "INSERT INTO abandoned_requests (branch_id, request_id, at, actor) \
             VALUES ('Bother-00000000000001', 'cross-00000000000001', now(), 'owner')"
        )
        .await,
        FOREIGN_KEY_VIOLATION,
        "abandoned_requests: XC row, OTHER's branch"
    );
    assert_eq!(
        db.outcome(
            "INSERT INTO device_settings (branch_id, key, value) \
             VALUES ('Bother-00000000000001', 'printer', 'x')"
        )
        .await,
        FOREIGN_KEY_VIOLATION,
        "device_settings: XC row, OTHER's branch"
    );
    // An OTHER row naming XC's MAIN.
    assert_eq!(
        db.outcome(&format!(
            "INSERT INTO device_settings (branch_id, key, value, organization_id) \
             VALUES ('MAIN', 'printer', 'x', '{other}')"
        ))
        .await,
        FOREIGN_KEY_VIOLATION,
        "device_settings: OTHER row, XC's branch"
    );

    // The same rows with the matching organisation are accepted: the refusal is the organisation's.
    assert_eq!(
        db.outcome(&format!(
            "INSERT INTO abandoned_requests (branch_id, request_id, at, actor, organization_id) \
             VALUES ('Bother-00000000000001', 'cross-00000000000001', now(), 'owner', '{other}')"
        ))
        .await,
        "ok"
    );
    assert_eq!(
        db.outcome(&format!(
            "INSERT INTO device_settings (branch_id, key, value, organization_id) \
             VALUES ('Bother-00000000000001', 'printer', 'x', '{other}')"
        ))
        .await,
        "ok"
    );
    assert_eq!(
        db.outcome(
            "INSERT INTO device_settings (branch_id, key, value) VALUES ('MAIN', 'printer', 'x')"
        )
        .await,
        "ok"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_row_naming_an_organisation_that_does_not_exist_is_refused() {
    let db = Db::start().await;
    assert_eq!(
        db.outcome(
            "INSERT INTO branches (id, name, organization_id) \
             VALUES ('Bnowhere-000000000001', 'Nowhere', gen_random_uuid())"
        )
        .await,
        FOREIGN_KEY_VIOLATION,
        "branches"
    );
    assert_eq!(
        db.outcome(
            "INSERT INTO currencies (code, name_th, organization_id) \
             VALUES ('ZZZ', 'ทดสอบ', gen_random_uuid())"
        )
        .await,
        FOREIGN_KEY_VIOLATION,
        "currencies, a table without a branch"
    );
    db.finish().await;
}

// ── the OR-1 binary on migration 1 ──────────────────────────────────────────

#[tokio::test]
async fn a_branch_the_service_creates_has_its_id_as_code_and_no_hours() {
    let db = Db::start().await;
    let org = db.install_org().await;
    let service = db.branches();
    let now = Utc
        .timestamp_opt(1_791_288_000, 0)
        .single()
        .expect("an instant");
    let created = service
        .create(
            CreateBranch {
                request_id: "or1-create-00000001".to_owned(),
                name: "Silom".to_owned(),
                from_branch: BranchId::new("MAIN"),
            },
            "owner",
            now,
        )
        .await
        .expect("create a branch");

    let id = created.id.as_str().to_owned();
    assert_eq!(
        created.code.as_deref(),
        Some(id.as_str()),
        "code is the id (§9)"
    );
    assert_eq!(created.address, None);
    assert_eq!(created.timezone, "Asia/Bangkok");
    assert_eq!(created.opening_hours, None);
    assert!(!created.booking_enabled);

    let (code, organization): (Option<String>, String) =
        sqlx::query_as("SELECT code, organization_id::text FROM branches WHERE id = $1")
            .bind(&id)
            .fetch_one(&db.pool)
            .await
            .expect("the stored row");
    assert_eq!(code.as_deref(), Some(id.as_str()), "the stored code");
    assert_eq!(organization, org);

    let read = service.require(&created.id).await.expect("read it back");
    assert_eq!(read, created, "what create returns is what is stored");
    assert_eq!(
        read.hours().expect("hours").window(day("2026-10-12")),
        HoursWindow::NotSet,
        "a new branch does not open service until ADMIN sets its hours (B4)"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_branch_read_back_carries_its_profile_and_its_hours_in_its_timezone() {
    let db = Db::start().await;
    let hours = json!({
        "mon": [{ "open": "11:00", "close": "23:00" }],
        "tue": [{ "open": "11:00", "close": "23:00" }],
        "wed": [{ "open": "11:00", "close": "23:00" }],
        "thu": [{ "open": "11:00", "close": "23:00" }],
        "fri": [{ "open": "11:00", "close": "23:00" }],
        "sat": [{ "open": "11:00", "close": "23:00" }],
        "sun": [],
    });
    sqlx::query(
        "UPDATE branches SET timezone = 'Asia/Tokyo', opening_hours = $1, \
         address = '1 Silom Road', booking_enabled = true WHERE id = 'MAIN'",
    )
    .bind(&hours)
    .execute(&db.pool)
    .await
    .expect("set MAIN's profile");

    let service = db.branches();
    let main = service.require(&BranchId::new("MAIN")).await.expect("MAIN");
    assert_eq!(main.code.as_deref(), Some("MAIN"));
    assert_eq!(main.address.as_deref(), Some("1 Silom Road"));
    assert_eq!(main.timezone, "Asia/Tokyo");
    assert_eq!(main.opening_hours.as_ref(), Some(&hours));
    assert!(main.booking_enabled);

    let h = main.hours().expect("MAIN's hours");
    assert_eq!(
        h.window(day("2026-10-12")),
        HoursWindow::Open {
            open: utc("2026-10-12T02:00:00Z"),
            close: utc("2026-10-12T14:00:00Z"),
        },
        "11:00–23:00 in Tokyo, not in Bangkok"
    );
    assert_eq!(h.window(day("2026-10-18")), HoursWindow::Closed);

    let listed = service.list().await.expect("list");
    assert_eq!(listed.first(), Some(&main), "list() reads the same profile");
    db.finish().await;
}
