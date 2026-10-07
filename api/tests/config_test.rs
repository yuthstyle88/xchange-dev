//! Specification tests for T1a: layered configuration.
//!
//! `Settings::load_from(dir, env)` reads `base.yaml`, then the optional
//! `<APP_ENVIRONMENT>.yaml`, then `APP_*` variables (`__` separates nesting) —
//! the layering of `pos_infra::config::settings::Settings::load`. The environment
//! is passed in, so these tests never touch the process environment and can run
//! in parallel.

mod common;

use std::path::Path;

use common::{config_dir, env, settings};
use xchange::config::{Channel, Settings};

fn load_error(pairs: &[(&str, &str)]) -> String {
    match Settings::load_from(&config_dir(), env(pairs)) {
        Ok(loaded) => panic!("expected an error for {pairs:?}, loaded {loaded:?}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn defaults_load_from_the_committed_yaml() {
    let s = settings(&[]);

    assert_eq!(s.application.port, 8080);
    assert!(!s.application.host.is_empty());
    assert!(s.application.workers >= 1);
    assert!(s.application.shutdown_timeout_secs >= 1);
    assert_eq!(s.channel, Channel::Staging);
    assert_eq!(s.database.host, "localhost");
    assert_eq!(s.database.port, 5432);
    assert_eq!(s.database.database_name, "xchange");
    assert!(s.database.max_connections >= 1);
    assert!(s.database.acquire_timeout_secs >= 1);
    assert_eq!(s.telemetry.service_name, "xchange");
}

#[test]
fn channel_has_exactly_the_two_fleet_names() {
    assert_eq!(Channel::Staging.as_str(), "staging");
    assert_eq!(Channel::Release.as_str(), "release");
}

#[test]
fn an_app_variable_overrides_the_yaml_value() {
    let s = settings(&[
        ("APP_APPLICATION__PORT", "9191"),
        ("APP_APPLICATION__HOST", "10.1.2.3"),
        ("APP_DATABASE__HOST", "db.internal"),
        // A non-local host needs TLS (DB-LINK, design §7c).
        ("APP_DATABASE__REQUIRE_SSL", "true"),
        ("APP_DATABASE__PORT", "6543"),
        ("APP_DATABASE__DATABASE_NAME", "xchange_other"),
        ("APP_CHANNEL", "release"),
    ]);

    assert_eq!(s.application.port, 9191);
    assert_eq!(s.application.host, "10.1.2.3");
    assert_eq!(s.database.host, "db.internal");
    assert_eq!(s.database.port, 6543);
    assert_eq!(s.database.database_name, "xchange_other");
    assert_eq!(s.channel, Channel::Release);
    // Untouched keys keep their YAML value.
    assert_eq!(s.telemetry.service_name, "xchange");
}

#[test]
fn variables_without_the_app_prefix_are_ignored() {
    let s = settings(&[
        ("PORT", "9999"),
        ("APPLICATION__PORT", "9999"),
        ("CHANNEL", "release"),
        ("DATABASE__PORT", "1"),
    ]);

    assert_eq!(s.application.port, 8080);
    assert_eq!(s.channel, Channel::Staging);
    assert_eq!(s.database.port, 5432);
}

#[test]
fn the_environment_overlay_is_selected_by_app_environment() {
    assert_eq!(
        settings(&[("APP_ENVIRONMENT", "development")]).channel,
        Channel::Staging
    );
    assert_eq!(
        settings(&[("APP_ENVIRONMENT", "staging")]).channel,
        Channel::Staging
    );
    assert_eq!(
        settings(&[("APP_ENVIRONMENT", "production")]).channel,
        Channel::Release
    );
}

#[test]
fn an_app_variable_wins_over_the_environment_overlay() {
    let s = settings(&[
        ("APP_ENVIRONMENT", "production"),
        ("APP_CHANNEL", "staging"),
    ]);

    assert_eq!(s.channel, Channel::Staging);
}

#[test]
fn a_malformed_number_is_an_error_naming_the_key() {
    let message = load_error(&[("APP_APPLICATION__PORT", "not-a-port")]);
    assert!(
        message.contains("application.port"),
        "the error must name `application.port`: {message}"
    );

    let message = load_error(&[("APP_DATABASE__PORT", "70000")]);
    assert!(
        message.contains("database.port"),
        "the error must name `database.port`: {message}"
    );
}

#[test]
fn a_channel_outside_the_two_names_is_an_error_naming_the_key() {
    for bad in ["beta", "production", "Release", ""] {
        let message = load_error(&[("APP_CHANNEL", bad)]);
        assert!(
            message.contains("channel"),
            "`{bad}`: the error must name `channel`: {message}"
        );
    }
}

#[test]
fn an_unknown_environment_is_an_error_naming_the_variable() {
    // A typo must fail the boot, not silently load no overlay.
    let message = load_error(&[("APP_ENVIRONMENT", "prod")]);
    assert!(
        message.contains("APP_ENVIRONMENT") && message.contains("prod"),
        "the error must name APP_ENVIRONMENT and the bad value: {message}"
    );
}

#[test]
fn a_directory_without_base_yaml_is_an_error() {
    let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/no-such-configuration");

    let err = Settings::load_from(&missing, env(&[])).expect_err("base.yaml is required");

    assert!(
        err.to_string().contains("base"),
        "the error must name the missing file: {err}"
    );
}
