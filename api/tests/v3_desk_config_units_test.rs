//! Tests for ticket #537: currency unit_size error handling.
//!
//! Asserts that a currency with no units row (or an invalid unit size) returns an AppError
//! and never falls back to an invented 1.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use pos_error::AppError;
use rpc_common::{Desk, one_of_each_role, sign_in};
use xc_kernel::ids::BranchId;
use xchange::presentation::http::v3::desk::config::DeskConfigCore;

const ORIGIN: &str = "http://desk.test";
const MAIN: &str = "MAIN";

#[actix_web::test]
async fn currency_with_no_units_row_returns_error_never_one() {
    let desk = Desk::open().await;
    let state = desk.state();
    let branch = BranchId::new(MAIN);

    // Baseline: read succeeds when all currencies have their units row in currencies table
    let config_ok = DeskConfigCore::read(&state, &branch)
        .await
        .expect("desk config read succeeds initially");
    let usd_rate = config_ok
        .rates
        .iter()
        .find(|r| r.code == "USD")
        .expect("USD in rates");
    assert_eq!(usd_rate.unit_size, "1.0000");

    // Remove USD from denominations and currencies table behind the scenes
    sqlx::query("DELETE FROM denominations WHERE currency_code = 'USD'")
        .execute(&desk.db.pool)
        .await
        .expect("delete USD from denominations table");
    sqlx::query("DELETE FROM currencies WHERE code = 'USD'")
        .execute(&desk.db.pool)
        .await
        .expect("delete USD from currencies table");

    // Reading desk config must return an AppError, never an invented fallback of 1
    let err = DeskConfigCore::read(&state, &branch)
        .await
        .expect_err("reading desk config without USD units row must fail");

    match err {
        AppError::Internal(msg) => {
            assert!(
                msg.contains("missing currency unit_size for USD"),
                "expected missing unit_size error message, got: {msg}"
            );
        }
        other => panic!("expected AppError::Internal for missing currency units, got: {other:?}"),
    }

    desk.finish().await;
}

#[actix_web::test]
async fn http_endpoint_returns_server_error_when_units_row_is_missing() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let people = one_of_each_role();
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;

    // Delete EUR from denominations and currencies
    sqlx::query("DELETE FROM denominations WHERE currency_code = 'EUR'")
        .execute(&desk.db.pool)
        .await
        .expect("delete EUR from denominations");
    sqlx::query("DELETE FROM currencies WHERE code = 'EUR'")
        .execute(&desk.db.pool)
        .await
        .expect("delete EUR from currencies");

    // GET /api/v3/views/desk_config?branch=MAIN as OWNER
    let mut call = rpc_common::Call::post(
        ORIGIN,
        "/api/v3/views/desk_config?branch=MAIN",
        Vec::<u8>::new(),
    )
    .without("x-xchange-request")
    .without("content-type")
    .without("origin")
    .cookie(&cookies["OWNER"]);
    call.method = actix_web::http::Method::GET;

    let answer = app.send(call).await;
    assert_eq!(
        answer.status, 500,
        "missing units row must return 500 serverError, got {}: {}",
        answer.status, answer.text
    );
    let json = answer.json();
    assert_eq!(json["error"]["code"], "serverError");

    desk.finish().await;
}
