//! Ticket #1255 (RD-0): the migration `api/migrations/20261112100000_organization_hosts.sql`, held to the
//! four answers of `docs/design/blueprint/rate-display.md` §7 (data migration) and to its table, in the form of
//! `branch_settings_migration_test.rs`.
//!
//! Written before the implementation by the test-writer claude/07d3765f, on the BE lane controller
//! claude/ad05bf8d's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! 1. *Existing rows*: no existing table changes. The file creates one table and touches nothing else, and
//!    every other table's columns, constraints and indexes are the same before and after it.
//! 2. *Backfill*: none. The new table starts empty.
//! 3. *Deploy order*: it runs at boot with the others, after every migration on `main` when it was written
//!    (`20261111180000_drop_rates.sql`). A later migration may follow it, so nothing here assumes it is the last.
//! 4. *What `down` loses*: there is no down half, only a commented `-- down:` block. Dropping the table by
//!    hand needs no `CASCADE`.
//!
//! The table is §7's: `host text PRIMARY KEY CHECK (host = lower(host) AND host <> '')`,
//! `organization_id uuid NOT NULL REFERENCES organizations (id)`, `created_at timestamptz NOT NULL DEFAULT now()`.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod quote_common;

use std::path::{Path, PathBuf};

use quote_common::TestDb;

const FILE: &str = "20261112100000_organization_hosts.sql";
const VERSION: i64 = 20261112100000;
/// Main's last migration when this was written.
const BEFORE: &str = "20261111180000_drop_rates.sql";
const XC_ID: &str = "00000000-0000-0000-0000-000000000001";

fn migrations() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

fn the_file() -> String {
    let path = migrations().join(FILE);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the migration {} exists: {e}", path.display()))
}

fn migration_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(migrations())
        .expect("read api/migrations")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .into_string()
                .expect("UTF-8")
        })
        .filter(|n| n.ends_with(".sql"))
        .collect();
    names.sort();
    names
}

async fn run(db: &TestDb, sql: &str) -> Result<(), String> {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
        .execute(&db.pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Every column, constraint and index of every table in the test's schema except `except`, one line each,
/// sorted: what "unchanged" is measured against.
async fn schema_shape(db: &TestDb, except: &str) -> Vec<String> {
    let columns: Vec<(String, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT table_name::text, column_name::text, data_type::text, is_nullable::text, column_default::text \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name <> $1 AND table_name <> '_sqlx_migrations'",
    )
    .bind(except)
    .fetch_all(&db.pool)
    .await
    .expect("read columns");
    let constraints: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT rel.relname::text, con.conname::text, pg_get_constraintdef(con.oid) \
         FROM pg_constraint con JOIN pg_class rel ON rel.oid = con.conrelid \
         WHERE rel.relnamespace = current_schema()::regnamespace AND rel.relname <> $1",
    )
    .bind(except)
    .fetch_all(&db.pool)
    .await
    .expect("read constraints");
    let indexes: Vec<(String, String)> = sqlx::query_as(
        "SELECT tablename::text, replace(indexdef, current_schema() || '.', '') FROM pg_indexes \
         WHERE schemaname = current_schema() AND tablename <> $1 AND tablename <> '_sqlx_migrations'",
    )
    .bind(except)
    .fetch_all(&db.pool)
    .await
    .expect("read indexes");
    let mut shape: Vec<String> = columns
        .into_iter()
        .map(|(t, c, k, n, d)| format!("column {t}.{c} {k} null={n} default={d:?}"))
        .chain(
            constraints
                .into_iter()
                .map(|(t, c, d)| format!("constraint {t}.{c} {d}")),
        )
        .chain(indexes.into_iter().map(|(t, d)| format!("index {t} {d}")))
        .collect();
    shape.sort();
    shape
}

/// Item 3: the file is there under the design's name and sorts after main's last migration when it was
/// written (a later migration may follow it). Item 4: no reversible migration exists.
#[test]
fn the_file_sorts_after_mains_last_migration_and_has_no_down_half() {
    the_file();
    let names = migration_names();
    assert!(
        names
            .iter()
            .all(|n| !n.contains(".down.") && !n.contains(".up.")),
        "no reversible migration: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == BEFORE),
        "{BEFORE} is on main: {names:?}"
    );
    assert!(FILE > BEFORE, "{FILE} sorts after {BEFORE}");
}

/// Item 4: the file ends with a commented `-- down:` block that names the drop; nothing in it runs.
#[test]
fn the_file_carries_only_a_commented_down_block() {
    let file = the_file();
    assert!(
        file.lines().any(|l| l.trim() == "-- down:"),
        "a `-- down:` comment line:\n{file}"
    );
    assert!(
        file.lines()
            .any(|l| l.trim().starts_with("--") && l.contains("DROP TABLE organization_hosts")),
        "the commented drop:\n{file}"
    );
    assert!(
        !file
            .lines()
            .filter(|l| !l.trim_start().starts_with("--"))
            .any(|l| l.to_uppercase().contains("DROP ")),
        "no uncommented DROP:\n{file}"
    );
}

/// Item 1: one `CREATE TABLE organization_hosts` statement, and no other statement.
#[test]
fn the_file_creates_one_table_and_touches_nothing_else() {
    let sql: String = the_file()
        .lines()
        .filter(|l| !l.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    let statements: Vec<String> = sql
        .split(';')
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .collect();
    let heads: Vec<String> = statements
        .iter()
        .map(|s| s.split(" (").next().unwrap_or("").to_uppercase())
        .collect();
    assert_eq!(
        heads,
        vec!["CREATE TABLE ORGANIZATION_HOSTS".to_string()],
        "{statements:#?}"
    );
}

/// Item 1, measured: every other table's columns, constraints and indexes are the same before and after.
#[tokio::test]
async fn every_existing_table_is_unchanged() {
    let before = TestDb::empty().await;
    let last_before = migration_names()
        .iter()
        .rfind(|n| n.as_str() < FILE)
        .map(|n| {
            n.split('_')
                .next()
                .expect("a version")
                .parse::<i64>()
                .expect("a numeric version")
        })
        .expect("a migration before this one");
    before.migrate_through(Some(last_before)).await;
    let after = TestDb::empty().await;
    after.migrate_through(Some(VERSION)).await;

    assert_eq!(
        schema_shape(&before, "organization_hosts").await,
        schema_shape(&after, "organization_hosts").await,
        "only organization_hosts is new"
    );
    let has_table = |db: &TestDb| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>("SELECT to_regclass('organization_hosts') IS NOT NULL")
                .fetch_one(&pool)
                .await
                .expect("look the table up")
        }
    };
    assert!(!has_table(&before).await, "not there before {VERSION}");
    assert!(has_table(&after).await, "there after {VERSION}");
    before.finish().await;
    after.finish().await;
}

/// Item 2: no backfill; the table starts empty.
#[tokio::test]
async fn the_table_starts_empty() {
    let db = TestDb::start().await;
    assert_eq!(
        db.text("SELECT count(*)::text FROM organization_hosts")
            .await
            .as_deref(),
        Some("0")
    );
    db.finish().await;
}

/// §7's columns, in order.
#[tokio::test]
async fn the_columns_are_the_designs() {
    let db = TestDb::start().await;
    let columns: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT column_name::text, data_type::text, is_nullable::text \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'organization_hosts' \
         ORDER BY ordinal_position",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read information_schema");
    let col = |n: &str, k: &str| (n.to_string(), k.to_string(), "NO".to_string());
    assert_eq!(
        columns,
        vec![
            col("host", "text"),
            col("organization_id", "uuid"),
            col("created_at", "timestamp with time zone"),
        ]
    );
    db.finish().await;
}

/// §7's rules, whoever writes: one row per host, lower-case and non-empty, a known organisation, and
/// `created_at` filled in.
#[tokio::test]
async fn the_table_keeps_its_rules() {
    let db = TestDb::start().await;
    let row = |host: &str, org: &str| {
        format!("INSERT INTO organization_hosts (host, organization_id) VALUES ('{host}', '{org}')")
    };
    assert_eq!(run(&db, &row("rates.example.test", XC_ID)).await, Ok(()));
    assert_eq!(
        run(&db, &row("rates.example.test:8443", XC_ID)).await,
        Ok(()),
        "a host with its port is its own row"
    );
    assert!(
        run(&db, &row("rates.example.test", XC_ID)).await.is_err(),
        "one row per host"
    );
    assert!(
        run(&db, &row("Rates.Example.Test", XC_ID)).await.is_err(),
        "lower-case only"
    );
    assert!(run(&db, &row("", XC_ID)).await.is_err(), "not empty");
    assert!(
        run(
            &db,
            &row("other.example.test", "00000000-0000-0000-0000-00000000dead")
        )
        .await
        .is_err(),
        "a known organisation"
    );
    assert!(
        run(
            &db,
            "INSERT INTO organization_hosts (host) VALUES ('no-org.example.test')"
        )
        .await
        .is_err(),
        "organization_id is required"
    );
    assert_eq!(
        db.text(
            "SELECT (created_at IS NOT NULL)::text FROM organization_hosts \
             WHERE host = 'rates.example.test'"
        )
        .await
        .as_deref(),
        Some("true"),
        "created_at defaults to now()"
    );
    db.finish().await;
}

/// Item 4: dropping the table by hand needs no `CASCADE` and loses only its rows.
#[tokio::test]
async fn dropping_the_table_by_hand_needs_no_cascade() {
    let db = TestDb::start().await;
    run(
        &db,
        &format!(
            "INSERT INTO organization_hosts (host, organization_id) VALUES ('rates.example.test', '{XC_ID}')"
        ),
    )
    .await
    .expect("a host row");
    let organizations = db.text("SELECT count(*)::text FROM organizations").await;
    assert_eq!(
        run(&db, "DROP TABLE organization_hosts").await,
        Ok(()),
        "nothing refers to the table"
    );
    assert_eq!(
        db.text("SELECT count(*)::text FROM organizations").await,
        organizations
    );
    db.finish().await;
}
