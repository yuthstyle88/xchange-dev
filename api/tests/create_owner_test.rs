//! Ticket T4b — the `xchange create-owner --username <u>` command (design §6).
//! Provided with the ticket — do not edit, delete or weaken; add tests in a file of your own.
//!
//! Each test runs the real binary against a database of its own, created through
//! `TEST_DATABASE_URL` (a database whose name ends in `_test`; the role must be allowed to
//! create databases). Without it the tests fail; none of them skips. A schema per test, as the
//! crate tests use, is not available here: the command migrates wherever its settings point.

use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;

const MUST_FINISH: Duration = Duration::from_secs(120);
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    name: String,
    admin: PgPool,
    options: PgConnectOptions,
    env: Vec<(&'static str, String)>,
}

impl Scratch {
    async fn create(label: &str) -> Scratch {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The create-owner tests need a real PostgreSQL and do not skip. Point it at an \
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
        // `postgres://user[:password]@host[:port]/database` — the password is the one part
        // sqlx does not hand back.
        let rest = url.split_once("://").expect("a URL").1;
        let credentials = rest.rsplit_once('@').map(|(c, _)| c).unwrap_or("");
        let password = credentials.split_once(':').map(|(_, p)| p).unwrap_or("");

        let admin = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(15))
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
        sweep(&admin).await;

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("t4b_{label}_{}_{nanos}_{n}_test", std::process::id());
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("create a scratch database — the role needs CREATEDB");

        let env = vec![
            ("APP_ENVIRONMENT", "development".to_owned()),
            (
                "APP_CONFIG_DIR",
                format!("{}/configuration", env!("CARGO_MANIFEST_DIR")),
            ),
            ("APP_DATABASE__HOST", options.get_host().to_owned()),
            ("APP_DATABASE__PORT", options.get_port().to_string()),
            ("APP_DATABASE__USERNAME", options.get_username().to_owned()),
            ("APP_DATABASE__PASSWORD", password.to_owned()),
            ("APP_DATABASE__DATABASE_NAME", name.clone()),
            ("APP_DATABASE__REQUIRE_SSL", "false".to_owned()),
            ("APP_DATABASE__ACQUIRE_TIMEOUT_SECS", "5".to_owned()),
            // If the command wrongly started the server it would bind here, not on 8080.
            ("APP_APPLICATION__HOST", "127.0.0.1".to_owned()),
            ("APP_APPLICATION__PORT", "0".to_owned()),
            ("APP_APPLICATION__WORKERS", "1".to_owned()),
        ];
        Scratch {
            name,
            admin,
            options,
            env,
        }
    }

    /// Runs `xchange <args>` with `stdin` on its standard input and waits for it to end.
    fn run(&self, args: &[&str], stdin: &str) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xchange"));
        for (key, _) in std::env::vars().filter(|(k, _)| k.starts_with("APP_")) {
            command.env_remove(key);
        }
        let mut child = command
            .args(args)
            .envs(self.env.iter().map(|(k, v)| (*k, v.as_str())))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start the xchange binary");
        // Write the password and close the pipe: the command reads to the end of the line or
        // of the input, and must not wait for a terminal.
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).ok();
        let started = Instant::now();
        loop {
            if child.try_wait().expect("wait for xchange").is_some() {
                return child.wait_with_output().expect("collect the output");
            }
            if started.elapsed() > MUST_FINISH {
                child.kill().ok();
                let output = child.wait_with_output().expect("collect the output");
                panic!(
                    "`xchange {}` did not end within {MUST_FINISH:?} — a command must do its work \
                     and exit, never start the server.\nstdout: {}\nstderr: {}",
                    args.join(" "),
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// A connection of the test's own into the scratch database: what is really there.
    async fn inspect(&self) -> PgPool {
        PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(15))
            .connect_with(self.options.clone().database(&self.name))
            .await
            .expect("connect to the scratch database")
    }

    /// (username, name, password_hash, active) of every owner; empty when the table is not there.
    async fn owners(&self) -> Vec<(String, String, String, bool)> {
        let pool = self.inspect().await;
        let exists: bool = sqlx::query_scalar("SELECT to_regclass('owners') IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
        let rows = if exists {
            sqlx::query_as(
                "SELECT username, name, password_hash, active FROM owners ORDER BY username",
            )
            .fetch_all(&pool)
            .await
            .expect("read owners")
        } else {
            Vec::new()
        };
        pool.close().await;
        rows
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

/// A test that fails before its last line leaves its database behind; every new one first
/// removes those older than ten minutes. Errors are ignored (another test may be doing the same).
async fn sweep(admin: &PgPool) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let names: Vec<String> =
        sqlx::query_scalar("SELECT datname FROM pg_database WHERE datname LIKE 't4b\\_%\\_test'")
            .fetch_all(admin)
            .await
            .unwrap_or_default();
    for name in names {
        // t4b_<label>_<pid>_<nanoseconds>_<n>_test
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

fn text(output: &Output) -> String {
    format!(
        "exit: {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

const PASSWORD: &str = "correct horse battery";

#[tokio::test(flavor = "multi_thread")]
async fn create_owner_reads_the_password_from_standard_input_and_makes_the_owner() {
    let db = Scratch::create("makes").await;
    assert!(
        db.owners().await.is_empty(),
        "an empty database to begin with"
    );

    let output = db.run(
        &["create-owner", "--username", " Boss.01 "],
        &format!("{PASSWORD}\n"),
    );
    assert!(
        output.status.success(),
        "the command succeeds\n{}",
        text(&output)
    );
    for stream in [&output.stdout, &output.stderr] {
        assert!(
            !String::from_utf8_lossy(stream).contains(PASSWORD),
            "the password is never printed\n{}",
            text(&output)
        );
    }

    let owners = db.owners().await;
    assert_eq!(
        owners.len(),
        1,
        "the migrations ran and one owner was written"
    );
    let (username, name, hash, active) = &owners[0];
    assert_eq!(username, "boss.01", "trimmed and lower-cased");
    assert_eq!(name, "boss.01");
    assert!(*active);
    assert!(hash.starts_with("$argon2id$v=19$"), "stored: {hash}");
    let memory: u32 = hash
        .split('$')
        .nth(3)
        .and_then(|params| params.split(',').find_map(|p| p.strip_prefix("m=")))
        .and_then(|m| m.parse().ok())
        .unwrap_or_else(|| panic!("the memory cost in {hash}"));
    assert!(
        memory >= 19_456,
        "the command hashes with the cost for real accounts: {hash}"
    );
    let hasher = PasswordHasher::new();
    assert!(
        hasher.verify(PASSWORD, hash),
        "the stored hash is of the line that was typed, spaces inside it kept, the line end not"
    );
    assert!(!hasher.verify(&format!("{PASSWORD}\n"), hash));
    db.drop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn create_owner_takes_a_password_that_ends_without_a_line_end_or_with_crlf() {
    for (label, typed) in [
        ("noeol", PASSWORD.to_owned()),
        ("crlf", format!("{PASSWORD}\r\n")),
    ] {
        let db = Scratch::create(label).await;
        let output = db.run(&["create-owner", "--username", "boss"], &typed);
        assert!(output.status.success(), "{label}\n{}", text(&output));
        let owners = db.owners().await;
        assert_eq!(owners.len(), 1, "{label}");
        assert!(
            PasswordHasher::new().verify(PASSWORD, &owners[0].2),
            "{label}"
        );
        db.drop().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn create_owner_is_refused_when_an_owner_exists() {
    let db = Scratch::create("second").await;
    let first = db.run(
        &["create-owner", "--username", "boss"],
        &format!("{PASSWORD}\n"),
    );
    assert!(first.status.success(), "{}", text(&first));
    let before = db.owners().await;

    for username in ["boss", "another"] {
        let again = db.run(
            &["create-owner", "--username", username],
            "another-password-9\n",
        );
        assert!(
            !again.status.success(),
            "a second owner is refused\n{}",
            text(&again)
        );
        assert!(
            !again.stderr.is_empty(),
            "the refusal says why on standard error"
        );
    }
    assert_eq!(
        db.owners().await,
        before,
        "the first owner and its password are untouched"
    );
    db.drop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn create_owner_refuses_what_the_service_refuses_and_writes_nothing() {
    let db = Scratch::create("refuses").await;
    for (args, stdin) in [
        (vec!["create-owner", "--username", "boss"], "short\n"),
        (vec!["create-owner", "--username", "boss"], ""),
        (
            vec!["create-owner", "--username", "b!"],
            "correct horse battery\n",
        ),
        (
            vec!["create-owner", "--username"],
            "correct horse battery\n",
        ),
        (vec!["create-owner"], "correct horse battery\n"),
        (vec!["create-owner", "boss"], "correct horse battery\n"),
        (
            vec!["create-owner", "--username", "boss", "--unknown"],
            "correct horse battery\n",
        ),
    ] {
        let output = db.run(&args, stdin);
        assert!(
            !output.status.success(),
            "`xchange {}` fails\n{}",
            args.join(" "),
            text(&output)
        );
        assert!(
            !output.stderr.is_empty(),
            "`xchange {}` says why on standard error",
            args.join(" ")
        );
        assert!(
            db.owners().await.is_empty(),
            "`xchange {}` writes no owner",
            args.join(" ")
        );
    }
    db.drop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_command_fails_instead_of_starting_the_server() {
    let db = Scratch::create("unknown").await;
    for args in [
        vec!["create-owners"],
        vec!["serve-something"],
        vec!["--username", "boss"],
    ] {
        let output = db.run(&args, "correct horse battery\n");
        assert!(
            !output.status.success(),
            "`xchange {}` fails\n{}",
            args.join(" "),
            text(&output)
        );
        assert!(!output.stderr.is_empty());
    }
    assert!(db.owners().await.is_empty());
    db.drop().await;
}
