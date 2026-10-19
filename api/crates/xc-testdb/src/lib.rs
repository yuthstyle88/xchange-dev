//! Test databases cloned from a migrated template (#2867).
//!
//! The shared core every api test harness calls, instead of `CREATE SCHEMA` plus
//! `Migrator::run` of all of `api/migrations/` per test:
//! - `template_name(dir)` names the template after a hash of the migrations set;
//! - `fresh_db(dir, prefix)` builds that template once (under an advisory lock, with no
//!   connection left open to it), then `CREATE DATABASE <unique>_test TEMPLATE <template>`
//!   and connects to the copy;
//! - `TestDatabase::drop` closes the pool and drops the copy.
//!
pub mod people;

use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::ensure;
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool};

const MUST_FINISH: Duration = Duration::from_secs(30);

static SEQ: AtomicU64 = AtomicU64::new(0);

/// One test's own database, copied from the template.
pub struct TestDatabase {
    /// A pool connected to the copy.
    pub pool: PgPool,
    /// The copy's database name; it ends in `_test`.
    pub name: String,
}

impl TestDatabase {
    /// Close the pool and drop the copy.
    pub async fn drop(self) -> anyhow::Result<()> {
        ensure!(
            self.name.ends_with("_test"),
            "refusing to drop database '{}' not ending in `_test`",
            self.name
        );
        self.pool.close().await;
        let url = std::env::var("TEST_DATABASE_URL")?;
        check_test_database_url(&url)?;
        let options = PgConnectOptions::from_str(&url)?;
        let mut admin = PgConnection::connect_with(&options).await?;
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            self.name
        )))
        .execute(&mut admin)
        .await?;
        admin.close().await?;
        Ok(())
    }
}

/// `xchange_tpl_<12 lowercase hex>`: a hash over every `*.sql` file name and its bytes in
/// `migrations_dir`, in name order. Changes whenever a migration is added, removed or edited.
pub fn template_name(migrations_dir: &Path) -> std::io::Result<String> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(migrations_dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "sql")
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            files.push((name.to_owned(), path));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (name, path) in files {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        let bytes = std::fs::read(path)?;
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    let digest = hasher.finalize();
    let hex = u64::from_be_bytes([
        0, 0, digest[0], digest[1], digest[2], digest[3], digest[4], digest[5],
    ]);
    Ok(format!("xchange_tpl_{hex:012x}"))
}

/// `Ok` when the database the URL names ends in `_test`, otherwise an error. `fresh_db` calls it
/// first, so every harness inherits the guard `api/tests/snapshot_common` has today.
pub fn check_test_database_url(url: &str) -> anyhow::Result<()> {
    let options = PgConnectOptions::from_str(url)?;
    let database = options.get_database().unwrap_or("");
    ensure!(
        database.ends_with("_test"),
        "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`"
    );
    Ok(())
}

/// A fresh database for one test. Reads `TEST_DATABASE_URL`, whose database name must end in
/// `_test` (otherwise an error); builds the template for `migrations_dir` if it does not exist
/// yet; copies it to `<prefix>_<unique>_test`; connects to the copy.
pub async fn fresh_db(migrations_dir: &Path, prefix: &str) -> anyhow::Result<TestDatabase> {
    let url = std::env::var("TEST_DATABASE_URL")?;
    check_test_database_url(&url)?;
    let options = PgConnectOptions::from_str(&url)?;

    let template = template_name(migrations_dir)?;
    let hex = template.strip_prefix("xchange_tpl_").unwrap_or("0");
    let lock_key = u64::from_str_radix(hex, 16)? as i64;

    let mut admin = PgConnection::connect_with(&options).await?;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(lock_key)
        .execute(&mut admin)
        .await?;

    let build_res = ensure_template(&mut admin, &options, migrations_dir, &template).await;
    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(lock_key)
        .execute(&mut admin)
        .await;
    build_res?;

    let copy_name = unique_copy_name(prefix);
    sqlx::query(AssertSqlSafe(format!(
        "CREATE DATABASE {copy_name} TEMPLATE {template}"
    )))
    .execute(&mut admin)
    .await?;
    admin.close().await?;

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(MUST_FINISH)
        .connect_with(options.database(&copy_name))
        .await?;

    Ok(TestDatabase {
        pool,
        name: copy_name,
    })
}

async fn ensure_template(
    admin: &mut PgConnection,
    options: &PgConnectOptions,
    migrations_dir: &Path,
    template: &str,
) -> anyhow::Result<()> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(template)
            .fetch_one(&mut *admin)
            .await?;
    if exists {
        return Ok(());
    }

    let building = format!("{template}_b");
    sqlx::query(AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {building} WITH (FORCE)"
    )))
    .execute(&mut *admin)
    .await?;
    sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {building}")))
        .execute(&mut *admin)
        .await?;

    let tpl_pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(MUST_FINISH)
        .connect_with(options.clone().database(&building))
        .await?;
    let migrate_res = Migrator::new(migrations_dir).await?.run(&tpl_pool).await;
    tpl_pool.close().await;
    migrate_res?;

    sqlx::query(AssertSqlSafe(format!(
        "ALTER DATABASE {building} RENAME TO {template}"
    )))
    .execute(&mut *admin)
    .await?;
    Ok(())
}

fn unique_copy_name(prefix: &str) -> String {
    let mut clean: String = prefix
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if clean.is_empty() || clean.as_bytes()[0].is_ascii_digit() {
        clean.insert(0, 't');
    }
    clean.truncate(16);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{clean}_{}_{nanos}_{seq}_test", std::process::id())
}
