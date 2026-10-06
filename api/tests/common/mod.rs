//! Helpers shared by the T1a specification tests.
//!
//! Nothing here needs a running database or a bound port: the readiness probe is
//! replaced by a fake through `AppState::with_database_probe`, and the pool that
//! `AppState::new` wants is a lazy one pointed at a port nothing listens on.

#![allow(dead_code)]

use std::future::Future;
use std::net::TcpListener;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use xchange::bootstrap::{AppState, BuildInfo, DatabaseProbe};
use xchange::config::Settings;

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
pub fn settings(pairs: &[(&str, &str)]) -> Settings {
    Settings::load_from(&config_dir(), env(pairs))
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
