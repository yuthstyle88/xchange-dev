//! #2867 up-front tests: one migrated template per migrations set, one copy per test.
//!
//! Written by the ticket author before the code. 🔴 The implementer must not edit, delete or
//! weaken them.
//!
//! They need a real PostgreSQL: `TEST_DATABASE_URL` must name an existing database whose name
//! ends in `_test` (CI's per-run Postgres provides `xchange_test`). Without it they FAIL; none of
//! them skips, the same rule as `api/tests/snapshot_common`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use xc_testdb::{check_test_database_url, fresh_db, template_name};

fn migrations() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

fn migration_count(dir: &Path) -> i64 {
    std::fs::read_dir(dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "sql")
        })
        .count() as i64
}

/// A pool on the `TEST_DATABASE_URL` database itself, to look at `pg_database` and
/// `pg_stat_activity` from outside the copies.
async fn admin() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL is not set: these tests need a real PostgreSQL and do not skip");
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to TEST_DATABASE_URL")
}

async fn database_exists(admin: &PgPool, name: &str) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
        .bind(name)
        .fetch_one(admin)
        .await
        .unwrap()
}

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A copy of `api/migrations/` plus one extra migration, in a fresh temp dir.
fn migrations_plus_one(table: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "xc-testdb-{}-{nanos}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for entry in std::fs::read_dir(migrations()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|x| x == "sql") {
            std::fs::copy(&path, dir.join(path.file_name().unwrap())).unwrap();
        }
    }
    std::fs::write(
        dir.join("29991231235959_xc_testdb_probe.sql"),
        format!("CREATE TABLE {table} (id integer PRIMARY KEY);\n"),
    )
    .unwrap();
    dir
}

#[tokio::test]
async fn a_fresh_database_has_every_migration_applied() {
    let db = fresh_db(&migrations(), "t2867a").await.unwrap();
    let applied: Option<i64> = sqlx::query_scalar(
        "SELECT count(*) FROM _sqlx_migrations WHERE success \
         AND to_regclass('public._sqlx_migrations') IS NOT NULL",
    )
    .fetch_one(&db.pool)
    .await
    .ok();
    assert_eq!(
        applied,
        Some(migration_count(&migrations())),
        "every file of api/migrations is recorded as applied in the copy"
    );
    let branches: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.branches')::text")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(branches.as_deref(), Some("branches"));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn each_test_gets_its_own_database_named_test() {
    let a = fresh_db(&migrations(), "t2867b").await.unwrap();
    let b = fresh_db(&migrations(), "t2867b").await.unwrap();
    assert_ne!(a.name, b.name, "two calls give two databases");
    assert!(a.name.ends_with("_test") && b.name.ends_with("_test"));
    let base: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&admin().await)
        .await
        .unwrap();
    assert_ne!(
        a.name, base,
        "the copy is not the TEST_DATABASE_URL database itself"
    );
    let current: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&a.pool)
        .await
        .unwrap();
    assert_eq!(
        current, a.name,
        "the pool is connected to the copy it names"
    );
    a.drop().await.unwrap();
    b.drop().await.unwrap();
}

#[tokio::test]
async fn a_row_written_in_one_copy_is_not_seen_in_another() {
    let a = fresh_db(&migrations(), "t2867c").await.unwrap();
    let b = fresh_db(&migrations(), "t2867c").await.unwrap();
    sqlx::query("CREATE TABLE xc_testdb_isolation (id integer)")
        .execute(&a.pool)
        .await
        .unwrap();
    let seen: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.xc_testdb_isolation')::text")
            .fetch_one(&b.pool)
            .await
            .unwrap();
    assert_eq!(
        seen, None,
        "a table made in one copy does not exist in the other"
    );
    a.drop().await.unwrap();
    b.drop().await.unwrap();
}

#[tokio::test]
async fn the_template_name_follows_the_migrations() {
    let base = template_name(&migrations()).unwrap();
    let again = template_name(&migrations()).unwrap();
    let changed = template_name(&migrations_plus_one("xc_testdb_probe_name")).unwrap();
    assert_eq!(base, again, "the same migrations give the same template");
    assert_ne!(base, changed, "one more migration gives another template");
    for name in [&base, &changed] {
        let hex = name
            .strip_prefix("xchange_tpl_")
            .unwrap_or_else(|| panic!("{name} starts with xchange_tpl_"));
        assert_eq!(hex.len(), 12, "{name}: 12 hex digits");
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }
}

#[test]
fn editing_a_migration_changes_the_template_name() {
    let dir = migrations_plus_one("xc_testdb_probe_edit");
    let before = template_name(&dir).unwrap();
    std::fs::write(
        dir.join("29991231235959_xc_testdb_probe.sql"),
        "CREATE TABLE xc_testdb_probe_edit (id bigint PRIMARY KEY);\n",
    )
    .unwrap();
    let after = template_name(&dir).unwrap();
    assert_ne!(
        before, after,
        "same file names, edited contents: another template"
    );
}

#[tokio::test]
async fn a_changed_migrations_set_gets_its_own_template_with_the_new_table() {
    let dir = migrations_plus_one("xc_testdb_probe_rebuild");
    let db = fresh_db(&dir, "t2867e").await.unwrap();
    let probe: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.xc_testdb_probe_rebuild')::text")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(probe.as_deref(), Some("xc_testdb_probe_rebuild"));
    let template = template_name(&dir).unwrap();
    assert!(
        database_exists(&admin().await, &template).await,
        "{template} was built"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn no_connection_is_left_open_to_the_template() {
    let db = fresh_db(&migrations(), "t2867f").await.unwrap();
    let template = template_name(&migrations()).unwrap();
    let admin = admin().await;
    assert!(
        database_exists(&admin, &template).await,
        "{template} exists after fresh_db"
    );
    let open: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname = $1")
        .bind(&template)
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(
        open, 0,
        "a connected template makes the next clone wait and fail"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn drop_removes_the_copy() {
    let db = fresh_db(&migrations(), "t2867g").await.unwrap();
    let name = db.name.clone();
    let admin = admin().await;
    assert!(database_exists(&admin, &name).await);
    db.drop().await.unwrap();
    assert!(
        !database_exists(&admin, &name).await,
        "{name} is gone after drop"
    );
}

#[tokio::test]
async fn eight_tests_starting_at_once_each_get_a_copy() {
    let mut handles = Vec::new();
    for _ in 0..8 {
        handles.push(tokio::spawn(async {
            let db = fresh_db(&migrations(), "t2867h").await.unwrap();
            let n: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
                .fetch_one(&db.pool)
                .await
                .unwrap_or(-1);
            let name = db.name.clone();
            db.drop().await.unwrap();
            (name, n)
        }));
    }
    let mut names = Vec::new();
    for h in handles {
        let (name, n) = h.await.unwrap();
        assert_eq!(
            n,
            migration_count(&migrations()),
            "{name} is fully migrated"
        );
        names.push(name);
    }
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 8, "eight distinct copies");
}

#[test]
fn a_database_url_not_ending_in_test_is_refused() {
    for url in [
        "postgres://postgres@127.0.0.1:5432/postgres",
        "postgres://postgres@127.0.0.1:5432/xchange",
        "postgres://postgres@127.0.0.1:5432/test_xchange",
    ] {
        assert!(
            check_test_database_url(url).is_err(),
            "{url} must be refused"
        );
    }
    for url in [
        "postgres://postgres@127.0.0.1:5432/xchange_test",
        "postgres://postgres:pw@localhost/xchange_test?sslmode=disable",
    ] {
        assert!(
            check_test_database_url(url).is_ok(),
            "{url} must be accepted"
        );
    }
}
