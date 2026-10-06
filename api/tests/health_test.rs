//! Specification tests for T1a: `GET /healthz` and `GET /health/ready`.
//!
//! `/healthz` must stay what v2 answers (`server.mjs:8262`:
//! `if (url.pathname === "/healthz") return Response.json({ ok: true });` — checked
//! before any method or host test). `/health/ready` is new and carries the fleet
//! versioning fields `version`, `build`, `builtAt`, `channel`.
//!
//! Every test drives `xchange::bootstrap::configure` through `actix_web::test`;
//! no port is bound and no database is needed.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use actix_web::http::{StatusCode, header};
use actix_web::{App, test};
use common::{FakeDatabase, injected_build, settings, state_with, unreachable_pool};
use serde_json::Value;
use xchange::bootstrap::{AppState, BuildInfo, configure};
use xchange::presentation::http::health::PROBE_TIMEOUT;

macro_rules! app {
    ($state:expr) => {{
        let state: AppState = $state;
        test::init_service(App::new().configure(move |cfg| configure(cfg, state))).await
    }};
}

fn content_type(resp: &actix_web::dev::ServiceResponse) -> String {
    resp.headers()
        .get(header::CONTENT_TYPE)
        .expect("a Content-Type header")
        .to_str()
        .expect("an ASCII Content-Type")
        .to_ascii_lowercase()
}

// ---------------------------------------------------------------- /healthz

#[actix_web::test]
async fn healthz_answers_exactly_what_v2_answers() {
    let app = app!(state_with(settings(&[]), FakeDatabase::Up));

    let resp =
        test::call_service(&app, test::TestRequest::get().uri("/healthz").to_request()).await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(content_type(&resp), "application/json");
    let body = test::read_body(resp).await;
    assert_eq!(&body[..], br#"{"ok":true}"#);
}

#[actix_web::test]
async fn healthz_ignores_the_method_like_v2() {
    // v2 answers before it looks at the method, so a POST gets the same reply.
    let app = app!(state_with(settings(&[]), FakeDatabase::Up));

    let resp =
        test::call_service(&app, test::TestRequest::post().uri("/healthz").to_request()).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = test::read_body(resp).await;
    assert_eq!(&body[..], br#"{"ok":true}"#);
}

#[actix_web::test]
async fn healthz_stays_ok_when_the_database_is_down() {
    // Liveness: v2's handler touches nothing, so a dead database must not turn it red.
    let app = app!(state_with(settings(&[]), FakeDatabase::Down));

    let resp =
        test::call_service(&app, test::TestRequest::get().uri("/healthz").to_request()).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = test::read_body(resp).await;
    assert_eq!(&body[..], br#"{"ok":true}"#);
}

// ----------------------------------------------------------- /health/ready

#[actix_web::test]
async fn ready_returns_200_with_the_four_identity_fields_when_the_database_answers() {
    let app = app!(state_with(settings(&[]), FakeDatabase::Up));

    let resp = test::call_service(
        &app,
        test::TestRequest::get().uri("/health/ready").to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(content_type(&resp), "application/json");
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["database"], true);
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["build"], injected_build().build.as_str());
    assert_eq!(body["builtAt"], injected_build().built_at.as_str());
    assert_eq!(body["channel"], "staging");
    // Wire DTOs are camelCase.
    assert!(
        body.get("built_at").is_none(),
        "snake_case key leaked: {body}"
    );
}

#[actix_web::test]
async fn ready_reports_the_identity_baked_into_this_build() {
    // No `with_build`: this is what a deployed binary reports.
    let state = AppState::new(Arc::new(settings(&[])), unreachable_pool())
        .with_database_probe(Arc::new(FakeDatabase::Up));
    let app = app!(state);

    let body: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get().uri("/health/ready").to_request(),
    )
    .await;

    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    let build = body["build"].as_str().expect("build is a string");
    assert!(!build.trim().is_empty(), "build must not be empty");
    assert_eq!(build, build.trim(), "build must carry no whitespace");
    let built_at = body["builtAt"].as_str().expect("builtAt is a string");
    chrono::DateTime::parse_from_rfc3339(built_at)
        .unwrap_or_else(|e| panic!("builtAt `{built_at}` is not RFC 3339: {e}"));
}

#[actix_web::test]
async fn baked_version_equals_the_manifest_version() {
    // The fleet rule: the version a binary reports must be the manifest's. Read
    // the manifest itself so a hard-coded string cannot pass after a release bump.
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("read api/Cargo.toml");
    let package = manifest
        .split("[package]")
        .nth(1)
        .expect("api/Cargo.toml has a [package] table");
    let declared = package
        .lines()
        .find_map(|line| line.trim().strip_prefix("version"))
        .and_then(|rest| rest.split('"').nth(1))
        .expect("[package] declares version = \"…\"");

    assert_eq!(BuildInfo::current().version, declared);
    assert_eq!(env!("CARGO_PKG_VERSION"), declared);
}

#[actix_web::test]
async fn baked_identity_is_complete() {
    let build = BuildInfo::current();
    assert!(!build.build.trim().is_empty(), "build must not be empty");
    chrono::DateTime::parse_from_rfc3339(&build.built_at)
        .unwrap_or_else(|e| panic!("builtAt `{}` is not RFC 3339: {e}", build.built_at));
}

#[actix_web::test]
async fn ready_channel_follows_configuration() {
    for (pairs, expected) in [
        (&[][..], "staging"),
        (&[("APP_CHANNEL", "release")][..], "release"),
        (&[("APP_ENVIRONMENT", "production")][..], "release"),
        (
            &[
                ("APP_ENVIRONMENT", "production"),
                ("APP_CHANNEL", "staging"),
            ][..],
            "staging",
        ),
    ] {
        let app = app!(state_with(settings(pairs), FakeDatabase::Up));
        let body: Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get().uri("/health/ready").to_request(),
        )
        .await;
        assert_eq!(body["channel"], expected, "environment {pairs:?}");
    }
}

#[actix_web::test]
async fn ready_returns_503_and_still_identifies_itself_when_the_database_is_down() {
    let app = app!(state_with(settings(&[]), FakeDatabase::Down));

    let resp = test::call_service(
        &app,
        test::TestRequest::get().uri("/health/ready").to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "degraded");
    assert_eq!(body["database"], false);
    // A node that is down must still say which build it is.
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["build"], injected_build().build.as_str());
    assert_eq!(body["builtAt"], injected_build().built_at.as_str());
    assert_eq!(body["channel"], "staging");
}

#[actix_web::test]
async fn ready_gives_up_after_the_probe_timeout_when_the_database_never_answers() {
    // Virtual time: the runtime jumps to the next timer whenever it is idle, so
    // this takes no wall-clock time. The outer guard turns "the handler has no
    // timeout" into a failure instead of a test that hangs forever.
    tokio::time::pause();
    let app = app!(state_with(settings(&[]), FakeDatabase::Hangs));
    let started = tokio::time::Instant::now();

    let resp = tokio::time::timeout(
        PROBE_TIMEOUT * 4,
        test::call_service(
            &app,
            test::TestRequest::get().uri("/health/ready").to_request(),
        ),
    )
    .await
    .expect("/health/ready must answer by itself when the database probe hangs");

    let waited = started.elapsed();
    assert!(
        waited >= PROBE_TIMEOUT && waited < PROBE_TIMEOUT + Duration::from_secs(1),
        "expected to wait PROBE_TIMEOUT ({PROBE_TIMEOUT:?}), waited {waited:?}"
    );
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "degraded");
    assert_eq!(body["database"], false);
}

#[actix_web::test]
async fn probe_timeout_is_short_enough_for_a_readiness_probe() {
    assert!(PROBE_TIMEOUT >= Duration::from_secs(1));
    assert!(PROBE_TIMEOUT <= Duration::from_secs(5));
}

#[actix_web::test]
async fn ready_returns_503_in_bounded_time_when_postgres_is_unreachable() {
    // The real probe: `AppState::new` must wire `/health/ready` to the pool it
    // was given. The pool points at a port nothing listens on.
    let state = AppState::new(Arc::new(settings(&[])), unreachable_pool());
    let app = app!(state);
    let started = Instant::now();

    let resp = test::call_service(
        &app,
        test::TestRequest::get().uri("/health/ready").to_request(),
    )
    .await;

    let waited = started.elapsed();
    assert!(
        waited < PROBE_TIMEOUT + Duration::from_secs(3),
        "took {waited:?} to notice an unreachable database"
    );
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "degraded");
    assert_eq!(body["database"], false);
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
}

// ------------------------------------------------------------ unknown route

#[actix_web::test]
async fn unknown_route_answers_like_v2() {
    // v2 `server.mjs:8291`: `return new Response("Not found", { status: 404 });`
    // — plain text, for every method.
    let app = app!(state_with(settings(&[]), FakeDatabase::Up));

    for req in [
        test::TestRequest::get().uri("/no-such-route"),
        test::TestRequest::post().uri("/api/no-such-route"),
        test::TestRequest::get().uri("/health"),
        test::TestRequest::get().uri("/health/live"),
    ] {
        let req = req.to_request();
        let label = format!("{} {}", req.method(), req.path());
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{label}");
        assert!(
            content_type(&resp).starts_with("text/plain"),
            "{label}: Content-Type {}",
            content_type(&resp)
        );
        let body = test::read_body(resp).await;
        assert_eq!(&body[..], b"Not found", "{label}");
    }
}
