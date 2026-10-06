//! The server purges expired sessions and past attempt windows at start and
//! then at a fixed interval (ticket #84).
//!
//! Three things are held here:
//! - the setting: `auth.purge_interval_secs`, one hour unless set otherwise;
//! - the timer, `bootstrap::purge_on_a_timer`: once at start, then every
//!   interval, and a purge that fails is logged and waited out — it never ends
//!   the timer, let alone the server. These tests run on Tokio's paused clock:
//!   no real time passes;
//! - the wiring: a server built by `Application::build` really purges its
//!   database. This one needs a real PostgreSQL (`TEST_DATABASE_URL`, a
//!   database whose name ends in `_test`, a role that may create databases)
//!   and FAILS without it.

mod common;

use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::account::repositories::account_repository::Purged;
use xchange::bootstrap::{self, Application};
use xchange::config::Settings;

const HOUR: Duration = Duration::from_secs(3600);

// ── the setting ─────────────────────────────────────────────────────────────

#[test]
fn the_interval_is_one_hour_unless_set() {
    // the committed configuration says so …
    assert_eq!(common::settings(&[]).auth.purge_interval_secs, 3600);
    let base = std::fs::read_to_string(common::config_dir().join("base.yaml")).expect("base.yaml");
    assert!(
        base.contains("purge_interval_secs: 3600"),
        "base.yaml names the setting and its default"
    );
    // … and a configuration that does not name it gets the same hour
    let dir = std::env::temp_dir().join(format!(
        "xchange-purge-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("a scratch configuration directory");
    let without: String = base
        .lines()
        .filter(|line| !line.starts_with("auth:") && !line.contains("purge_interval_secs"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!without.contains("auth"), "{without}");
    std::fs::write(dir.join("base.yaml"), without).expect("write base.yaml");
    let loaded = Settings::load_from(&dir, common::env(&[]));
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(
        loaded
            .expect("a configuration without `auth` loads")
            .auth
            .purge_interval_secs,
        3600
    );
}

#[test]
fn the_interval_is_set_from_the_environment_and_must_be_at_least_a_second() {
    let set = common::settings(&[("APP_AUTH__PURGE_INTERVAL_SECS", "60")]);
    assert_eq!(set.auth.purge_interval_secs, 60);

    for bad in ["0", "-5", "soon"] {
        let got = Settings::load_from(
            &common::config_dir(),
            common::env(&[("APP_AUTH__PURGE_INTERVAL_SECS", bad)]),
        );
        assert!(
            got.is_err(),
            "an interval of `{bad}` must be refused when the settings are loaded"
        );
    }
}

// ── the timer, on a paused clock ────────────────────────────────────────────

/// A purge that records when it was asked and fails on the calls named.
struct Recorder {
    asked: Mutex<Vec<DateTime<Utc>>>,
    fail_on: Vec<usize>,
}

impl Recorder {
    fn new(fail_on: &[usize]) -> Arc<Recorder> {
        Arc::new(Recorder {
            asked: Mutex::new(Vec::new()),
            fail_on: fail_on.to_vec(),
        })
    }

    fn asked(&self) -> Vec<DateTime<Utc>> {
        self.asked.lock().unwrap().clone()
    }

    async fn purge(&self, now: DateTime<Utc>) -> AppResult<Purged> {
        let call = {
            let mut asked = self.asked.lock().unwrap();
            asked.push(now);
            asked.len()
        };
        if self.fail_on.contains(&call) {
            Err(AppError::Internal(format!(
                "the database is away (call {call})"
            )))
        } else {
            Ok(Purged::default())
        }
    }
}

fn wall(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 7, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

/// The wall clock the timer is given: `wall(0)` plus the paused clock's time.
fn clock() -> impl Fn() -> DateTime<Utc> + Send + 'static {
    let started = tokio::time::Instant::now();
    move || wall(0) + chrono::Duration::from_std(started.elapsed()).unwrap()
}

fn start(every: Duration, recorder: &Arc<Recorder>) -> tokio::task::JoinHandle<()> {
    let recorder = recorder.clone();
    tokio::spawn(bootstrap::purge_on_a_timer(every, clock(), move |now| {
        let recorder = recorder.clone();
        async move { recorder.purge(now).await }
    }))
}

#[tokio::test(start_paused = true)]
async fn it_purges_at_start_and_then_once_every_interval() {
    let recorder = Recorder::new(&[]);
    let timer = start(HOUR, &recorder);

    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(recorder.asked(), vec![wall(0)], "once at start");

    tokio::time::sleep(HOUR - Duration::from_millis(2)).await;
    assert_eq!(recorder.asked().len(), 1, "not before the hour is over");

    tokio::time::sleep(Duration::from_millis(2)).await;
    assert_eq!(recorder.asked(), vec![wall(0), wall(3600)]);

    tokio::time::sleep(3 * HOUR).await;
    assert_eq!(
        recorder.asked(),
        vec![wall(0), wall(3600), wall(7200), wall(10_800), wall(14_400)],
        "the purge is given the clock's time of each round"
    );
    assert!(!timer.is_finished(), "the timer never ends by itself");
    timer.abort();
}

#[tokio::test(start_paused = true)]
async fn the_interval_is_the_one_it_was_given() {
    let recorder = Recorder::new(&[]);
    let timer = start(Duration::from_secs(60), &recorder);

    tokio::time::sleep(Duration::from_secs(60 * 10) + Duration::from_millis(1)).await;
    assert_eq!(recorder.asked().len(), 11, "at start and ten minutes of it");
    timer.abort();
}

#[tokio::test(start_paused = true)]
async fn a_purge_that_fails_is_waited_out_and_the_next_round_runs() {
    // the first one fails (the database is not there yet), and the third
    let recorder = Recorder::new(&[1, 3]);
    let timer = start(HOUR, &recorder);

    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(recorder.asked().len(), 1);
    assert!(
        !timer.is_finished(),
        "a failed purge at start ended the timer"
    );

    tokio::time::sleep(HOUR).await;
    assert_eq!(
        recorder.asked().len(),
        2,
        "the failed round is not retried early"
    );

    tokio::time::sleep(2 * HOUR).await;
    assert_eq!(
        recorder.asked(),
        vec![wall(0), wall(3600), wall(7200), wall(10_800)]
    );
    assert!(!timer.is_finished(), "a failed purge ended the timer");
    timer.abort();
}

// ── the wiring, against a real database ─────────────────────────────────────

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A database of this test's own, created and dropped by it.
struct Scratch {
    name: String,
    admin: PgPool,
    options: PgConnectOptions,
    settings: Settings,
}

impl Scratch {
    async fn create(interval_secs: &str) -> Scratch {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The purge wiring test needs a real PostgreSQL and does not skip. Point it at \
                 an existing database whose name ends in `_test`, e.g.\n\
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
        let rest = url.split_once("://").expect("a URL").1;
        let credentials = rest.rsplit_once('@').map(|(c, _)| c).unwrap_or("");
        let password = credentials.split_once(':').map(|(_, p)| p).unwrap_or("");

        let admin = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(15))
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("purge84_{}_{nanos}_{n}_test", std::process::id());
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("create a scratch database — the role needs CREATEDB");

        let settings = common::settings(&[
            ("APP_DATABASE__HOST", options.get_host()),
            ("APP_DATABASE__PORT", &options.get_port().to_string()),
            ("APP_DATABASE__USERNAME", options.get_username()),
            ("APP_DATABASE__PASSWORD", password),
            ("APP_DATABASE__DATABASE_NAME", &name),
            ("APP_DATABASE__REQUIRE_SSL", "false"),
            ("APP_APPLICATION__HOST", "127.0.0.1"),
            ("APP_APPLICATION__PORT", "0"),
            ("APP_APPLICATION__WORKERS", "1"),
            ("APP_AUTH__PURGE_INTERVAL_SECS", interval_secs),
        ]);
        Scratch {
            name,
            admin,
            options,
            settings,
        }
    }

    async fn inspect(&self) -> PgPool {
        PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(15))
            .connect_with(self.options.clone().database(&self.name))
            .await
            .expect("connect to the scratch database")
    }

    async fn drop(self) {
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

async fn insert_session(pool: &PgPool, hash: &str, expires: DateTime<Utc>) {
    sqlx::query(
        "INSERT INTO owner_sessions (hash, owner_id, version, expires) VALUES ($1, 'o1', 1, $2)",
    )
    .bind(hash)
    .bind(expires)
    .execute(pool)
    .await
    .expect("insert a session");
}

async fn session_hashes(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT hash FROM owner_sessions ORDER BY hash")
        .fetch_all(pool)
        .await
        .expect("read owner_sessions")
}

/// Asks until `wanted` is what the table holds, for at most twenty seconds.
async fn until_sessions_are(pool: &PgPool, wanted: &[&str], what: &str) {
    let started = std::time::Instant::now();
    loop {
        let got = session_hashes(pool).await;
        if got == wanted {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{what}: owner_sessions holds {got:?}, expected {wanted:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_purges_its_database_at_start_and_again_after_the_interval() {
    let scratch = Scratch::create("1").await;
    // the tables, before the server is there
    bootstrap::connect(&scratch.settings)
        .await
        .expect("migrate the scratch database")
        .close()
        .await;
    let pool = scratch.inspect().await;
    let now: DateTime<Utc> = SystemTime::now().into();
    sqlx::query(
        "INSERT INTO owners (id, username, name, password_hash, created_at) VALUES ('o1', 'boss', 'boss', '-', $1)",
    )
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert an owner");
    insert_session(&pool, "dead-1", now - chrono::Duration::hours(1)).await;
    insert_session(&pool, "live", now + chrono::Duration::hours(1)).await;
    sqlx::query(
        "INSERT INTO auth_attempts (key, window_start, count) VALUES ('k', $1, 3), ('k', $2, 1)",
    )
    .bind(now - chrono::Duration::hours(2))
    .bind(now + chrono::Duration::hours(2))
    .execute(&pool)
    .await
    .expect("insert attempt windows");

    let application = Application::build(scratch.settings.clone())
        .await
        .expect("the server starts");
    let server = tokio::spawn(application.run_until_stopped());

    until_sessions_are(&pool, &["live"], "at start").await;
    let windows: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_attempts")
        .fetch_one(&pool)
        .await
        .expect("count auth_attempts");
    assert_eq!(windows, 1, "the past window went, the one ahead stays");

    // a session that has run out since: the next round takes it
    insert_session(&pool, "dead-2", now - chrono::Duration::minutes(1)).await;
    until_sessions_are(&pool, &["live"], "after the interval").await;

    server.abort();
    let _ = server.await;
    pool.close().await;
    scratch.drop().await;
}
