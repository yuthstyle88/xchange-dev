//! Tests for ticket #537 (re-expected for #5864): `unitSize` comes from the published rate line,
//! not a separate `currencies` lookup.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use rpc_common::{Desk, one_of_each_role, sign_in};
use serde_json::json;
use xc_kernel::ids::BranchId;
use xchange::presentation::http::v3::desk::config::DeskConfigCore;

const ORIGIN: &str = "http://desk.test";
const MAIN: &str = "MAIN";

#[actix_web::test]
async fn currency_with_no_units_row_returns_error_never_one() {
    let desk = Desk::open_without_cash().await;
    let state = desk.state();
    let branch = BranchId::new(MAIN);

    snapshot_common::publish_rate_per_one(&desk.db.pool, MAIN, "USD", "32.10", "32.45").await;
    snapshot_common::publish_rate_per_one(&desk.db.pool, MAIN, "JPY", "0.235", "0.240").await;

    // Alter currencies.unit_size behind the scenes; DeskConfigCore::read must still return the
    // published line's own unit_size ("1.0000" for USD, "100.0000" for JPY).
    sqlx::query("UPDATE currencies SET unit_size = 999 WHERE code IN ('USD', 'JPY')")
        .execute(&desk.db.pool)
        .await
        .expect("alter currencies.unit_size");

    let config_ok = DeskConfigCore::read(&state, &branch)
        .await
        .expect("desk config reads unit_size from published line");
    let usd_rate = config_ok
        .rates
        .iter()
        .find(|r| r.code == "USD")
        .expect("USD in rates");
    assert_eq!(usd_rate.unit_size, "1.0000");
    let jpy_rate = config_ok
        .rates
        .iter()
        .find(|r| r.code == "JPY")
        .expect("JPY in rates");
    assert_eq!(jpy_rate.unit_size, "100.0000");

    desk.finish().await;
}

#[actix_web::test]
async fn http_endpoint_returns_server_error_when_units_row_is_missing() {
    let desk = Desk::open_without_cash().await;
    let app = rpc_app!(desk);
    let people = one_of_each_role();
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;

    snapshot_common::publish_rate_per_one(&desk.db.pool, MAIN, "EUR", "38.10", "38.90").await;
    snapshot_common::publish_rate_per_one(&desk.db.pool, MAIN, "JPY", "0.235", "0.240").await;
    sqlx::query("UPDATE currencies SET unit_size = 777 WHERE code IN ('EUR', 'JPY')")
        .execute(&desk.db.pool)
        .await
        .expect("alter currencies.unit_size");

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
        answer.status, 200,
        "desk_config reads unitSize from the published line: {}: {}",
        answer.status, answer.text
    );
    let rates = answer.json()["data"]["rates"]
        .as_array()
        .expect("rates is a list")
        .clone();
    let eur = rates
        .iter()
        .find(|r| r["code"] == json!("EUR"))
        .expect("EUR");
    assert_eq!(eur["unitSize"], json!("1.0000"));
    let jpy = rates
        .iter()
        .find(|r| r["code"] == json!("JPY"))
        .expect("JPY");
    assert_eq!(jpy["unitSize"], json!("100.0000"));

    desk.finish().await;
}
