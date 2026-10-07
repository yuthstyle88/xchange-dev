//! Written before the implementation (DB-LINK, docs/design/xchange-v3.md §7c
//! "The database link"). Do not edit, delete or weaken.
//!
//! `database.require_ssl = false` becomes sqlx `PgSslMode::Prefer` in the shared
//! `DatabaseSettings::without_db` — TLS only if the server offers it, never
//! checked. That is acceptable on the same host and nowhere else, so
//! `Settings::load_from` refuses a database host that is not local unless
//! `require_ssl` is true. Local: `localhost` (any letter case), a loopback IP
//! address (`127.0.0.0/8`, `::1`), or a path starting with `/` (a Unix socket
//! directory). Everything else is not local.

mod common;

use common::{config_dir, env, settings};
use xchange::config::Settings;

/// The error `Settings::load_from` returns for these variables; panics if they load.
fn load_error(pairs: &[(&str, &str)]) -> String {
    match Settings::load_from(&config_dir(), env(pairs)) {
        Ok(loaded) => panic!(
            "expected the database host {:?} to be refused without require_ssl",
            loaded.database.host
        ),
        Err(e) => e.to_string(),
    }
}

#[test]
fn a_remote_host_without_require_ssl_is_refused_naming_the_setting_and_the_host() {
    for host in ["db.internal", "10.0.0.5", "203.0.113.7", "2001:db8::1"] {
        let message = load_error(&[("APP_DATABASE__HOST", host)]);
        assert!(
            message.contains("require_ssl"),
            "the refusal for {host} must name database.require_ssl: {message}"
        );
        assert!(
            message.contains(host),
            "the refusal must name the host {host}: {message}"
        );
    }
}

#[test]
fn require_ssl_false_written_out_is_refused_the_same_way() {
    let message = load_error(&[
        ("APP_DATABASE__HOST", "db.internal"),
        ("APP_DATABASE__REQUIRE_SSL", "false"),
    ]);
    assert!(message.contains("require_ssl"), "{message}");
}

#[test]
fn a_remote_host_with_require_ssl_loads() {
    let s = settings(&[
        ("APP_DATABASE__HOST", "db.internal"),
        ("APP_DATABASE__REQUIRE_SSL", "true"),
    ]);
    assert_eq!(s.database.host, "db.internal");
    assert!(s.database.require_ssl);
}

#[test]
fn local_hosts_load_without_require_ssl() {
    for host in [
        "localhost",
        "LOCALHOST",
        "127.0.0.1",
        "127.0.0.53",
        "::1",
        "/var/run/postgresql",
    ] {
        let s = settings(&[
            ("APP_DATABASE__HOST", host),
            ("APP_DATABASE__REQUIRE_SSL", "false"),
        ]);
        assert_eq!(s.database.host, host);
        assert!(!s.database.require_ssl);
    }
}

#[test]
fn the_committed_default_host_still_loads() {
    // base.yaml: host "localhost", require_ssl false.
    let s = settings(&[]);
    assert_eq!(s.database.host, "localhost");
}

#[test]
fn names_that_only_look_local_are_not_local() {
    for host in [
        "localhost.example.com",
        "mylocalhost",
        "127.0.0.1.example.com",
        "0.0.0.0",
        "relative/socket/dir",
    ] {
        let message = load_error(&[("APP_DATABASE__HOST", host)]);
        assert!(message.contains("require_ssl"), "{host}: {message}");
    }
}
