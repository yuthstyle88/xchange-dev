//! Written before the implementation (ticket T6b-1). Do not edit, delete or weaken.
//!
//! Shared by the file tests: the database harness, a directory per test and the
//! v2 fixture.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing
//! database whose name ends in `_test`. When the variable is absent every test
//! FAILS; none of them skips. Each `TestDb` is a schema of its own with
//! `api/migrations/` applied, so tests run in parallel and the migration is
//! exercised from an empty schema every time. SQL in the migration and in the
//! implementation must therefore name tables without a schema prefix.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use serde::Deserialize;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use xc_kernel::ids::BranchId;

pub use xchange::infrastructure::files::file_service::{FileService, MAX_UPLOAD_BYTES, StoredFile};

/// Upper bound for anything that must finish.
pub const MUST_FINISH: Duration = Duration::from_secs(30);

pub const MAIN: &str = "MAIN";
pub const OWNER: &str = "owner-1";

// The sentences v2 answers with in this context.
pub const BAD_BRANCH: &str = "สาขาไม่ถูกต้อง";
pub const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
pub const UNSUPPORTED: &str = "ชนิดไฟล์ไม่รองรับ";
pub const TOO_LARGE: &str = "ไฟล์ใหญ่เกิน 15 MB";
pub const NOT_THAT_KIND: &str = "ชนิดไฟล์ไม่ตรงกับเนื้อหา";

/// The first bytes of a JPEG; enough for the service to take it as one.
pub const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F'];
pub const PDF: &[u8] = b"%PDF-1.7\n";

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A name no other test, test binary or run on this server has used.
pub fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

pub fn branch(id: &str) -> BranchId {
    BranchId::new(id)
}

/// 2026-10-06 12:00:00 UTC plus `seconds`, with microseconds that are not whole
/// milliseconds: what a wall clock hands the service.
pub fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_791_288_000 + seconds, 123_456_000)
        .single()
        .expect("a valid instant")
}

/// The same instant with everything below the millisecond dropped.
pub fn millis(t: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(t.timestamp_millis())
        .single()
        .expect("a valid instant")
}

/// The sentence of a refusal. Panics on success and on any error that is not
/// `AppError::Domain` — a refusal reaches the screen as its sentence and nothing else.
pub fn refusal<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected Err(AppError::Domain(<sentence>)), got {other:?}"),
    }
}

/// SHA-256 of `bytes` as PostgreSQL computes it, in lower-case hex — so the
/// tests need no hashing library of their own.
pub async fn sha256(pool: &PgPool, bytes: &[u8]) -> String {
    sqlx::query_scalar("SELECT encode(sha256($1), 'hex')")
        .bind(bytes)
        .fetch_one(pool)
        .await
        .expect("PostgreSQL computes a SHA-256")
}

/// What v2 names an uploaded file: a random UUID, version 4, lower case.
pub fn is_upload_key(key: &str) -> bool {
    let b = key.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            14 => *c == b'4',
            19 => matches!(c, b'8' | b'9' | b'a' | b'b'),
            _ => c.is_ascii_digit() || (b'a'..=b'f').contains(c),
        })
}

/// key, branch_id, owner_id, mime, sha256, created_at
pub type FileRow = (String, String, String, String, String, DateTime<Utc>);

pub struct TestDb {
    pub pool: PgPool,
    /// The upload directory of this test. It does not exist until something creates it.
    pub dir: PathBuf,
    schema: String,
    scratch: PathBuf,
}

/// Schemas being migrated at once; every one opens its own pool.
static STARTING: Semaphore = Semaphore::const_new(6);

impl TestDb {
    /// A pool confined to a fresh schema with `api/migrations/` applied, and the
    /// path of an upload directory nobody has created yet.
    pub async fn start() -> Self {
        let _slot = STARTING.acquire().await.expect("semaphore is never closed");
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The file tests need a real PostgreSQL and do not skip. Point it at an \
                 existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`. \
             Use a dedicated test database."
        );

        let schema = unique("t6b");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()))
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");

        let scratch = std::env::temp_dir().join(format!("xc-{schema}"));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("create a scratch directory");
        let scratch = scratch
            .canonicalize()
            .expect("the scratch directory exists");
        // two levels that do not exist yet: the service has to create them
        let dir = scratch.join("store").join("uploads");

        Self {
            pool,
            dir,
            schema,
            scratch,
        }
    }

    pub fn files(&self) -> FileService {
        FileService::new(self.pool.clone(), self.dir.clone())
    }

    /// Add a branch (the table is ticket T5a's; `MAIN` is there already).
    pub async fn seed_branch(&self, id: &str, name: &str) {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $2)")
            .bind(id)
            .bind(name)
            .execute(&self.pool)
            .await
            .expect("insert into branches");
    }

    /// A row and a file put there without the service — a receipt another
    /// ticket stores, or a file from another time.
    pub async fn seed_file(
        &self,
        key: &str,
        branch_id: &str,
        owner_id: &str,
        mime: &str,
        bytes: &[u8],
    ) {
        self.seed_row(key, branch_id, owner_id, mime).await;
        std::fs::create_dir_all(&self.dir).expect("create the upload directory");
        std::fs::write(self.dir.join(key), bytes).expect("write a seeded file");
    }

    pub async fn seed_row(&self, key: &str, branch_id: &str, owner_id: &str, mime: &str) {
        sqlx::query(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ($1, $2, $3, $4, 'seeded', $5)",
        )
        .bind(key)
        .bind(branch_id)
        .bind(owner_id)
        .bind(mime)
        .bind(millis(at(0)))
        .execute(&self.pool)
        .await
        .expect("insert into files");
    }

    pub async fn rows(&self) -> Vec<FileRow> {
        sqlx::query_as(
            "SELECT key, branch_id, owner_id, mime, sha256, created_at FROM files ORDER BY key",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read files")
    }

    pub async fn row(&self, key: &str) -> Option<FileRow> {
        sqlx::query_as(
            "SELECT key, branch_id, owner_id, mime, sha256, created_at FROM files WHERE key = $1",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .expect("read files")
    }

    /// Every name in the upload directory; empty when the directory is not there.
    pub fn names_on_disk(&self) -> BTreeSet<String> {
        match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries
                .map(|e| {
                    e.expect("a directory entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
            Err(e) => panic!("read {}: {e}", self.dir.display()),
        }
    }

    /// Drop the schema and the scratch directory. Reached only by a test that
    /// passed; a failed test leaves both behind for inspection.
    pub async fn finish(self) {
        self.pool.close().await;
        let url = std::env::var("TEST_DATABASE_URL").expect("checked in start");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
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
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

// ── the v2 fixture ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct Fixture {
    pub limit: usize,
    pub sentences: Vec<String>,
    pub cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
pub struct Case {
    pub label: String,
    /// The `Content-Type` v2 was sent; `None` when there was none.
    pub mime: Option<String>,
    /// The first bytes, in hex …
    pub head: String,
    /// … followed by this many zero bytes.
    pub pad: usize,
    pub size: usize,
    pub accepted: bool,
    pub error: Option<String>,
    pub sha256: Option<String>,
}

impl Case {
    pub fn bytes(&self) -> Vec<u8> {
        let mut out: Vec<u8> = (0..self.head.len() / 2)
            .map(|i| u8::from_str_radix(&self.head[2 * i..2 * i + 2], 16).expect("hex"))
            .collect();
        out.resize(out.len() + self.pad, 0);
        assert_eq!(out.len(), self.size, "fixture case '{}'", self.label);
        out
    }
}

pub fn fixture() -> Fixture {
    serde_json::from_str(include_str!("../fixtures/files/v2_upload_cases.json"))
        .expect("tests/fixtures/files/v2_upload_cases.json parses")
}
