//! Written before the implementation (row SET, design `docs/design/xchange-v3.md` §5.5.1,
//! *Data migration — `branch_settings` and `branch_setting_history`*). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! The migration `api/migrations/20261008100000_branch_settings.sql`, held to the four answers
//! of the design's data-migration section:
//!
//! 1. *Existing rows* — none change: the file creates two tables and touches nothing else.
//! 2. *Backfill* — none: on a database that already has branches the two tables are empty.
//! 3. *Deploy order* — it runs at start-up with the others (`sqlx::migrate!`), in its place
//!    after every migration on `main` at the time of writing.
//! 4. *What `down` loses* — there is no down migration; dropping the two tables by hand needs no
//!    `CASCADE` (nothing else refers to them) and leaves every other table as it was.
//!
//! And the columns and rules the design writes out.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod compat_common;
mod snapshot_common;

use std::path::{Path, PathBuf};

use snapshot_common::TestDb;

const FILE: &str = "20261008100000_branch_settings.sql";

fn migrations() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

fn the_file() -> String {
    let path = migrations().join(FILE);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the migration {} exists: {e}", path.display()))
}

/// (column, type, nullable) of a table of the test's schema, in column order.
async fn columns(db: &TestDb, table: &str) -> Vec<(String, String, String)> {
    sqlx::query_as(
        "SELECT column_name::text, data_type::text, is_nullable::text \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(&db.pool)
    .await
    .expect("read information_schema")
}

fn col(name: &str, kind: &str, nullable: &str) -> (String, String, String) {
    (name.to_string(), kind.to_string(), nullable.to_string())
}

async fn count(db: &TestDb, table: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("count {table}: {e}"))
}

async fn run(db: &TestDb, sql: &str) -> Result<(), String> {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
        .execute(&db.pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Item 3: the file is there, under the design's name, and sorts after every other migration.
/// Item 4: it has no down half.
#[test]
fn the_file_is_the_last_migration_and_has_no_down_half() {
    the_file();
    let mut names: Vec<String> = std::fs::read_dir(migrations())
        .expect("read api/migrations")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .into_string()
                .expect("UTF-8")
        })
        .collect();
    names.sort();
    assert!(
        names
            .iter()
            .all(|n| !n.contains(".down.") && !n.contains(".up.")),
        "no reversible migration: {names:?}"
    );
    let before: Vec<&String> = names.iter().filter(|n| n.as_str() < FILE).collect();
    assert!(
        before.iter().any(|n| n.starts_with("20261007190000_")),
        "it comes after the migrations on main at the time of writing: {names:?}"
    );
}

/// Item 1: two `CREATE TABLE` statements and no other statement — nothing that changes, fills or
/// drops what is already there.
#[test]
fn the_file_creates_two_tables_and_touches_nothing_else() {
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
        vec![
            "CREATE TABLE BRANCH_SETTINGS".to_string(),
            "CREATE TABLE BRANCH_SETTING_HISTORY".to_string()
        ],
        "{statements:#?}"
    );
}

/// Item 2: no backfill — `MAIN` and another branch exist, the two tables are empty.
#[tokio::test]
async fn the_tables_start_empty_on_a_database_with_branches() {
    let db = TestDb::start().await;
    db.seed_branch("AIRPORT", "Airport").await;
    assert!(count(&db, "branches").await >= 2);
    assert_eq!(count(&db, "branch_settings").await, 0);
    assert_eq!(count(&db, "branch_setting_history").await, 0);
    db.finish().await;
}

#[tokio::test]
async fn the_columns_are_the_designs() {
    let db = TestDb::start().await;
    assert_eq!(
        columns(&db, "branch_settings").await,
        vec![
            col("branch_id", "text", "NO"),
            col("name", "text", "NO"),
            col("version", "bigint", "NO"),
            col("body", "jsonb", "NO"),
            col("updated_at", "timestamp with time zone", "NO"),
            col("updated_by", "text", "NO"),
        ]
    );
    assert_eq!(
        columns(&db, "branch_setting_history").await,
        vec![
            col("branch_id", "text", "NO"),
            col("name", "text", "NO"),
            col("version", "bigint", "NO"),
            col("actor", "text", "NO"),
            col("at", "timestamp with time zone", "NO"),
            col("before", "jsonb", "NO"),
            col("after", "jsonb", "NO"),
        ]
    );
    db.finish().await;
}

/// The rules the table keeps whoever writes: a known branch, a name of the agreed shape, a
/// version from 1, one row per (branch, name), one history row per (branch, name, version), and
/// no history without its document.
#[tokio::test]
async fn the_tables_keep_their_rules() {
    let db = TestDb::start().await;
    let doc = |branch: &str, name: &str, version: i64| {
        format!(
            "INSERT INTO branch_settings VALUES ('{branch}', '{name}', {version}, '{{}}', now(), 'owner')"
        )
    };
    assert_eq!(run(&db, &doc("MAIN", "paymentMethods", 1)).await, Ok(()));
    assert!(
        run(&db, &doc("MAIN", "paymentMethods", 2)).await.is_err(),
        "one row per (branch, name)"
    );
    assert!(
        run(&db, &doc("NOWHERE", "paymentMethods", 1))
            .await
            .is_err(),
        "a known branch"
    );
    assert!(
        run(&db, &doc("MAIN", "zero", 0)).await.is_err(),
        "version from 1"
    );
    for bad in ["PaymentMethods", "payment-methods", "1st", ""] {
        assert!(
            run(&db, &doc("MAIN", bad, 1)).await.is_err(),
            "name {bad:?}"
        );
    }
    let forty = "a".repeat(40);
    assert_eq!(run(&db, &doc("MAIN", &forty, 1)).await, Ok(()));
    assert!(
        run(&db, &doc("MAIN", &"a".repeat(41), 1)).await.is_err(),
        "41 characters"
    );

    let change = |name: &str, version: i64| {
        format!(
            "INSERT INTO branch_setting_history VALUES ('MAIN', '{name}', {version}, 'owner', now(), '{{}}', '{{}}')"
        )
    };
    assert_eq!(run(&db, &change("paymentMethods", 1)).await, Ok(()));
    assert!(
        run(&db, &change("paymentMethods", 1)).await.is_err(),
        "one row per version"
    );
    assert!(
        run(&db, &change("neverWritten", 1)).await.is_err(),
        "no history without its document"
    );
    db.finish().await;
}

/// Item 4: dropping the two tables by hand needs no `CASCADE` and leaves the rest as it was.
#[tokio::test]
async fn dropping_the_two_tables_by_hand_loses_only_them() {
    let db = TestDb::start().await;
    run(
        &db,
        "INSERT INTO branch_settings VALUES ('MAIN', 'paymentMethods', 1, '[]', now(), 'owner')",
    )
    .await
    .expect("a document");
    let branches = count(&db, "branches").await;
    assert_eq!(
        run(&db, "DROP TABLE branch_setting_history").await,
        Ok(()),
        "nothing refers to the history"
    );
    assert_eq!(
        run(&db, "DROP TABLE branch_settings").await,
        Ok(()),
        "nothing else refers to the documents"
    );
    assert_eq!(count(&db, "branches").await, branches);
    db.finish().await;
}
