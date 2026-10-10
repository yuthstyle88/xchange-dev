//! RA-2 (#980, design `docs/design/blueprint/broker-seam.md` §5.4) — the Broker attribution
//! handler is registered through PL-0a's seam (`bootstrap/outbox_handlers/broker_attribution.rs`,
//! read here through `outbox_handlers::registry`, as `Application::build` reads it)
//! only when both `XC_BROKER_BASE_URL` and `XC_BROKER_SERVICE_TOKEN` are set and not blank.
//! Written before the implementation by the ticket author (claude/89bd61d9). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! No database: the pool is a lazy one nothing listens behind (`common::unreachable_pool`).

mod common;

use std::sync::Arc;

use xchange::bootstrap::AppState;
use xchange::bootstrap::outbox_handlers;
use xchange::config::{BrokerSettings, Settings};

const TOKEN: &str = "tok-SECRET-4be1d0c97a3f";

fn settings_with(pairs: &[(&str, &str)]) -> Settings {
    common::settings(pairs)
}

fn state(settings: Settings) -> AppState {
    AppState::new(Arc::new(settings), common::unreachable_pool())
}

#[tokio::test]
async fn both_variables_set_register_the_broker_handler() {
    let settings = settings_with(&[
        ("XC_BROKER_BASE_URL", "http://broker.test"),
        ("XC_BROKER_SERVICE_TOKEN", TOKEN),
    ]);
    assert_eq!(
        settings.broker,
        Some(BrokerSettings {
            base_url: "http://broker.test".to_owned(),
            service_token: TOKEN.to_owned(),
        })
    );
    let state = state(settings);

    // through the generated table, as `Application::build` builds the worker's registry
    let registry = outbox_handlers::registry(&state).expect("no clash");
    let handler = registry
        .handler("broker.attribution")
        .expect("configured: broker_attribution.rs registers its handler");
    assert_eq!(handler.kind().destination, "broker");
    assert_eq!(handler.kind().aggregate_type, "referral_attribution");
}

#[tokio::test]
async fn a_missing_or_blank_variable_registers_nothing() {
    let cases: [&[(&str, &str)]; 6] = [
        &[],
        &[("XC_BROKER_BASE_URL", "http://broker.test")],
        &[("XC_BROKER_SERVICE_TOKEN", TOKEN)],
        &[
            ("XC_BROKER_BASE_URL", ""),
            ("XC_BROKER_SERVICE_TOKEN", TOKEN),
        ],
        &[
            ("XC_BROKER_BASE_URL", "http://broker.test"),
            ("XC_BROKER_SERVICE_TOKEN", "   "),
        ],
        &[
            ("XC_BROKER_BASE_URL", "  "),
            ("XC_BROKER_SERVICE_TOKEN", " "),
        ],
    ];
    for pairs in cases {
        let settings = settings_with(pairs);
        assert_eq!(settings.broker, None, "{pairs:?}: not configured");
        let state = state(settings);
        let registry = outbox_handlers::registry(&state).expect("boots without Broker");
        assert!(
            registry.handler("broker.attribution").is_none(),
            "{pairs:?}: rows wait and expire after 7 days"
        );
    }
}

#[tokio::test]
async fn the_settings_never_print_the_token() {
    let settings = settings_with(&[
        ("XC_BROKER_BASE_URL", "http://broker.test"),
        ("XC_BROKER_SERVICE_TOKEN", TOKEN),
    ]);
    assert!(settings.broker.is_some(), "configured");
    let printed = format!("{settings:?}");
    assert!(
        printed.contains("http://broker.test"),
        "the base URL may show: {printed}"
    );
    assert!(!printed.contains(TOKEN), "the token never shows: {printed}");
}
