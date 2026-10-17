//! Helpers shared by the T1a specification tests.
//!
//! Nothing here needs a running database or a bound port: the readiness probe is
//! replaced by a fake through `AppState::with_database_probe`, and the pool that
//! `AppState::new` wants is a lazy one pointed at a port nothing listens on.

#![allow(dead_code)]

use std::future::Future;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use xchange::bootstrap::{AppState, BuildInfo, DatabaseProbe};
use xchange::config::Settings;

static TEST_DOCS_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The root directory for temporary test document stores (`<temp_dir>/xc-test-docs`).
pub fn test_docs_root() -> PathBuf {
    std::env::temp_dir().join("xc-test-docs")
}

fn pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        std::process::Command::new("ps")
            .args(["-p", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(true)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// Removes `<root>/<pid>` children whose `<pid>` is a `u32` other than the
/// current process ID and whose process is no longer alive.
pub fn reap_dead_test_docs(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let own_pid = std::process::id();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if pid != own_pid && !pid_alive(pid) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The committed `api/configuration` directory.
pub fn config_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("configuration")
}

/// Turns `[("APP_X", "1")]` into the owned pairs `Settings::load_from` takes.
pub fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// Settings from the committed YAML plus exactly the given environment.
///
/// Gives every test settings a fresh temporary `verification.store_dir` outside
/// the repo (issue #1729) unless `APP_VERIFICATION__STORE_DIR` is explicitly
/// provided in `pairs`.
pub fn settings(pairs: &[(&str, &str)]) -> Settings {
    let mut env_pairs = env(pairs);
    if !env_pairs
        .iter()
        .any(|(k, _)| k == "APP_VERIFICATION__STORE_DIR")
    {
        static REAP_ONCE: std::sync::Once = std::sync::Once::new();
        let root = test_docs_root();
        REAP_ONCE.call_once(|| reap_dead_test_docs(&root));
        let n = TEST_DOCS_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp_dir = root.join(std::process::id().to_string()).join(n.to_string());
        let _ = std::fs::create_dir_all(&temp_dir);
        env_pairs.push((
            "APP_VERIFICATION__STORE_DIR".to_string(),
            temp_dir.to_string_lossy().into_owned(),
        ));
    }
    Settings::load_from(&config_dir(), env_pairs)
        .expect("the committed configuration must load with this environment")
}

/// A port that was free a moment ago and has no listener now.
pub fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = listener.local_addr().expect("local address").port();
    drop(listener);
    port
}

/// A pool that never connected and cannot: it opens no socket until first used.
/// Must be called inside a Tokio runtime (every `#[actix_web::test]` is).
pub fn unreachable_pool() -> PgPool {
    let url = format!(
        "postgres://xchange:xchange@127.0.0.1:{}/xchange",
        closed_port()
    );
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(1))
        .connect_lazy(&url)
        .expect("a lazy pool needs no server")
}

/// What the fake database answers when `/health/ready` asks.
#[derive(Debug, Clone, Copy)]
pub enum FakeDatabase {
    Up,
    Down,
    /// Never answers — the handler's own timeout has to end the request.
    Hangs,
}

impl DatabaseProbe for FakeDatabase {
    fn ping(&self) -> Pin<Box<dyn Future<Output = bool> + '_>> {
        let answer = *self;
        Box::pin(async move {
            match answer {
                FakeDatabase::Up => true,
                FakeDatabase::Down => false,
                FakeDatabase::Hangs => std::future::pending::<bool>().await,
            }
        })
    }
}

/// A build identity no real build can produce, so a handler that reports its own
/// constants instead of the state's is caught.
pub fn injected_build() -> BuildInfo {
    BuildInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        build: "0123456789abcdef0123456789abcdef01234567".to_string(),
        built_at: "2001-02-03T04:05:06Z".to_string(),
    }
}

/// State with the injected build identity and the given fake database.
pub fn state_with(settings: Settings, database: FakeDatabase) -> AppState {
    AppState::new(Arc::new(settings), unreachable_pool())
        .with_build(injected_build())
        .with_database_probe(Arc::new(database))
}

/// CI-4 (#651): the server refuses to start without PII keys (U3, `bootstrap/mod.rs`), so a test that starts it
/// installs the test key first. `tests/bootstrap_pii_key_test.rs` deliberately does not.
pub fn install_test_pii_keys() {
    let keys =
        xc_kernel::pii::PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
            .expect("the test PII key parses");
    xc_customer::install_pii_keys(std::sync::Arc::new(keys)).expect("one test PII key per process");
}
