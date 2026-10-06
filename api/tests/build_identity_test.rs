//! Integration test verifying that `/health/ready` reports the build identity
//! of the current checkout.
//!
//! Never share CARGO_TARGET_DIR between worktrees (the test would read another tree's stamp).

mod common;

use std::sync::Arc;

use actix_web::{App, test};
use common::{FakeDatabase, settings, unreachable_pool};
use serde_json::Value;
use xchange::bootstrap::{AppState, configure};

#[actix_web::test]
async fn health_endpoint_reports_current_build_identity() {
    let state = AppState::new(Arc::new(settings(&[])), unreachable_pool())
        .with_database_probe(Arc::new(FakeDatabase::Up));
    let app = test::init_service(App::new().configure(move |cfg| configure(cfg, state))).await;

    let body: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get().uri("/health/ready").to_request(),
    )
    .await;

    let reported_build = body["build"].as_str().expect("build must be a string");

    if let Ok(expected_sha) = std::env::var("BUILD_SHA") {
        let expected_sha = expected_sha.trim();
        if !expected_sha.is_empty() {
            println!(
                "SKIPPED: BUILD_SHA is set in environment ({expected_sha}); verifying build equals BUILD_SHA"
            );
            assert_eq!(reported_build, expected_sha);
            return;
        }
    }

    let head_output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git rev-parse HEAD must execute");
    assert!(head_output.status.success(), "git rev-parse HEAD failed");
    let head_sha = String::from_utf8_lossy(&head_output.stdout)
        .trim()
        .to_string();

    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .expect("git status --porcelain must execute");
    assert!(
        status_output.status.success(),
        "git status --porcelain failed"
    );
    let is_dirty = !String::from_utf8_lossy(&status_output.stdout)
        .trim()
        .is_empty();

    let expected_build = if is_dirty {
        format!("{head_sha}-dirty")
    } else {
        head_sha
    };

    assert_eq!(reported_build, expected_build);
}
