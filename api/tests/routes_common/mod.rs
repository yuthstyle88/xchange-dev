//! Helpers for the T1b specification tests: a database of its own for every
//! test that lets `bootstrap::connect` migrate, and a one-request HTTP client.
//! Provided with ticket T1b — do not edit.
//!
//! Why a database per test and not a schema per test, as the crate tests use:
//! `bootstrap::connect` takes `Settings` and migrates wherever those settings
//! point — the database's default schema. Pointing it at the shared test
//! database would leave tables in its `public` schema for every later run.
//! So each test creates `<name>_<unique>_test` through `TEST_DATABASE_URL`,
//! hands `connect` settings that name it, and drops it afterwards. The role in
//! `TEST_DATABASE_URL` must be allowed to create databases (the CI service and
//! a throw-away container both are).

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xchange::config::Settings;

use super::common::settings;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

/// The parts of `TEST_DATABASE_URL` that `Settings` wants one by one.
struct Server {
    host: String,
    port: String,
    username: String,
    password: String,
    options: PgConnectOptions,
}

fn server() -> Server {
    let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "TEST_DATABASE_URL is not set.\n\
             The bootstrap tests need a real PostgreSQL and do not skip. Point it at an \
             existing database whose name ends in `_test`, e.g.\n\
             TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
        )
    });
    let options =
        PgConnectOptions::from_str(&url).expect("TEST_DATABASE_URL is not a PostgreSQL URL");
    let database = options.get_database().unwrap_or("").to_owned();
    assert!(
        database.ends_with("_test"),
        "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`."
    );
    // `postgres://user[:password]@host[:port]/database` — the password is the one
    // part sqlx does not hand back
    let rest = url.split_once("://").expect("a URL").1;
    let credentials = rest.rsplit_once('@').map(|(c, _)| c).unwrap_or("");
    let password = credentials
        .split_once(':')
        .map(|(_, p)| p)
        .unwrap_or("")
        .to_owned();
    Server {
        host: options.get_host().to_owned(),
        port: options.get_port().to_string(),
        username: options.get_username().to_owned(),
        password,
        options,
    }
}

/// A test that fails before its last line leaves its scratch database behind.
/// Every new one first removes those older than ten minutes; errors are ignored
/// (another test binary may be removing the same ones).
async fn sweep(admin: &PgPool) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let names: Vec<String> =
        sqlx::query_scalar("SELECT datname FROM pg_database WHERE datname LIKE 't1b\\_%\\_test'")
            .fetch_all(admin)
            .await
            .unwrap_or_default();
    for name in names {
        // t1b_<label>_<pid>_<nanoseconds>_<n>_test
        let parts: Vec<&str> = name.split('_').collect();
        let made: Option<u128> = parts
            .len()
            .checked_sub(3)
            .and_then(|i| parts[i].parse().ok());
        if made.is_some_and(|made| now.saturating_sub(made) > 600 * 1_000_000_000) {
            let _ = sqlx::query(AssertSqlSafe(format!(
                "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
            )))
            .execute(admin)
            .await;
        }
    }
}

/// An empty database that exists only for one test.
pub struct ScratchDatabase {
    pub name: String,
    admin: PgPool,
    server: Server,
}

impl ScratchDatabase {
    /// `label` must start with `t1b_` (the sweep looks for that prefix).
    pub async fn create(label: &str) -> ScratchDatabase {
        assert!(label.starts_with("t1b_"), "{label}");
        let server = server();
        let admin = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(15))
            .connect_with(server.options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
        sweep(&admin).await;
        let name = format!("{}_test", unique(label));
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("create a scratch database — the role needs CREATEDB");
        ScratchDatabase {
            name,
            admin,
            server,
        }
    }

    /// Settings that point the server at this database, on port 0 (any free port).
    pub fn settings(&self) -> Settings {
        settings(&[
            ("APP_DATABASE__HOST", self.server.host.as_str()),
            ("APP_DATABASE__PORT", self.server.port.as_str()),
            ("APP_DATABASE__USERNAME", self.server.username.as_str()),
            ("APP_DATABASE__PASSWORD", self.server.password.as_str()),
            ("APP_DATABASE__DATABASE_NAME", self.name.as_str()),
            ("APP_DATABASE__ACQUIRE_TIMEOUT_SECS", "5"),
            ("APP_APPLICATION__HOST", "127.0.0.1"),
            ("APP_APPLICATION__PORT", "0"),
            ("APP_APPLICATION__WORKERS", "1"),
        ])
    }

    /// A connection of the test's own into the scratch database: what is really there.
    pub async fn inspect(&self) -> PgPool {
        PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(15))
            .connect_with(self.server.options.clone().database(&self.name))
            .await
            .expect("connect to the scratch database")
    }

    pub async fn drop(self) {
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            self.name
        )))
        .execute(&self.admin)
        .await
        .expect("drop the scratch database");
        self.admin.close().await;
    }
}

/// Settings whose database is a port nothing listens on.
pub fn settings_without_a_database(port: u16) -> Settings {
    let port = port.to_string();
    settings(&[
        ("APP_DATABASE__HOST", "127.0.0.1"),
        ("APP_DATABASE__PORT", port.as_str()),
        ("APP_DATABASE__ACQUIRE_TIMEOUT_SECS", "1"),
        ("APP_APPLICATION__HOST", "127.0.0.1"),
        ("APP_APPLICATION__PORT", "0"),
        ("APP_APPLICATION__WORKERS", "1"),
    ])
}

/// How many migration files `api/migrations` holds.
pub fn migration_files() -> i64 {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .filter(|f| f.path().extension().is_some_and(|x| x == "sql"))
        .count() as i64
}

/// One `GET` over a real socket: (status line, whole response text).
pub fn http_get(port: u16, path: &str) -> (String, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to the server");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set a read timeout");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .expect("send the request");
    let mut text = String::new();
    stream
        .read_to_string(&mut text)
        .expect("read the response to its end");
    let status = text.lines().next().unwrap_or("").to_owned();
    (status, text)
}
