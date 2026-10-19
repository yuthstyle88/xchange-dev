//! OR-7.m (#1027) provided tests for the two seams that outlive password login:
//! - `xc_auth::infrastructure::attempts::PgAttemptCounter` (ticket T2), the device agent's rate-limit
//!   counter on `auth_attempts`;
//! - `xc_auth::application::user::directory::user_names` (ticket T1), names by `users.id`.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in `_test`).
//! Without it they FAIL; none skips. Implementers must not edit this file.

use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::application::user::directory::user_names;
use xc_auth::infrastructure::attempts::PgAttemptCounter;

const MUST_FINISH: Duration = Duration::from_secs(30);
const ORG: &str = "00000000-0000-0000-0000-000000000001";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip")
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(database.ends_with("_test"), "'{database}' does not end in `_test`");
        let schema = unique("or7m_seams");
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
            .max_connections(8)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        Self { pool, schema }
    }

    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }

    async fn user(&self, name: &str, status: &str) -> String {
        sqlx::query_scalar::<_, String>(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, $3, 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(name)
        .bind(status)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("insert user {name}: {e}"))
    }

    async fn windows(&self) -> Vec<(String, DateTime<Utc>, i32)> {
        sqlx::query_as::<_, (String, DateTime<Utc>, i32)>(
            "SELECT key, window_start, count FROM auth_attempts ORDER BY key, window_start",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read auth_attempts")
    }
}

fn at(minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 19, 6, minute, 0).single().expect("a time")
}

// ------------------------------------------------------------ T2: PgAttemptCounter

#[tokio::test]
async fn increment_counts_per_key_and_window_from_one() {
    let db = TestDb::start().await;
    let counter = PgAttemptCounter::new(db.pool.clone());

    assert_eq!(counter.increment("AGENT:a", at(0)).await.expect("1st"), 1);
    assert_eq!(counter.increment("AGENT:a", at(0)).await.expect("2nd"), 2);
    assert_eq!(counter.increment("AGENT:a", at(0)).await.expect("3rd"), 3);
    // Another key, and another window of the same key, each start at 1.
    assert_eq!(counter.increment("PAIR:a", at(0)).await.expect("other key"), 1);
    assert_eq!(counter.increment("AGENT:a", at(15)).await.expect("next window"), 1);

    assert_eq!(
        db.windows().await,
        vec![
            ("AGENT:a".to_owned(), at(0), 3),
            ("AGENT:a".to_owned(), at(15), 1),
            ("PAIR:a".to_owned(), at(0), 1),
        ]
    );
    db.finish().await;
}

#[tokio::test]
async fn concurrent_increments_lose_no_count() {
    let db = TestDb::start().await;
    let counter = PgAttemptCounter::new(db.pool.clone());

    let mut tasks = Vec::new();
    for _ in 0..20 {
        let counter = counter.clone();
        tasks.push(tokio::spawn(async move { counter.increment("AGENT:c", at(0)).await }));
    }
    let mut answers = Vec::new();
    for task in tasks {
        answers.push(task.await.expect("joined").expect("counted"));
    }
    answers.sort_unstable();
    assert_eq!(answers, (1..=20).collect::<Vec<i32>>());
    db.finish().await;
}

#[tokio::test]
async fn purge_before_deletes_only_earlier_windows() {
    let db = TestDb::start().await;
    let counter = PgAttemptCounter::new(db.pool.clone());
    counter.increment("k", at(0)).await.expect("w0");
    counter.increment("k", at(15)).await.expect("w15");
    counter.increment("k", at(30)).await.expect("w30");
    counter.increment("other", at(15)).await.expect("other w15");

    assert_eq!(counter.purge_before(at(30)).await.expect("purge"), 3);
    assert_eq!(db.windows().await, vec![("k".to_owned(), at(30), 1)]);
    assert_eq!(counter.purge_before(at(30)).await.expect("purge again"), 0);
    db.finish().await;
}

// ------------------------------------------------------------ T1: user_names

#[tokio::test]
async fn user_names_answers_users_by_id_and_skips_the_rest() {
    let db = TestDb::start().await;
    let anna = db.user("Anna", "ACTIVE").await;
    let bo = db.user("Bo", "DISABLED").await;
    let _carl = db.user("Carl", "ACTIVE").await;

    let ids = vec![
        anna.clone(),
        bo.clone(),
        anna.clone(),
        "owner".to_owned(),
        "emp-1".to_owned(),
        "00000000-0000-0000-0000-00000000dead".to_owned(),
    ];
    let names = user_names(&db.pool, &ids).await.expect("names");

    assert_eq!(names.len(), 2, "only the two users asked for: {names:?}");
    assert_eq!(names.get(&anna).map(String::as_str), Some("Anna"));
    assert_eq!(names.get(&bo).map(String::as_str), Some("Bo"), "a disabled user still answers");
    db.finish().await;
}

#[tokio::test]
async fn user_names_of_nothing_is_empty() {
    let db = TestDb::start().await;
    db.user("Anna", "ACTIVE").await;
    assert!(user_names(&db.pool, &[]).await.expect("names").is_empty());
    db.finish().await;
}
