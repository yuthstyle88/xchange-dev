//! `rates` prints the branch's saved payment rules, not the defaults (K-HTTP #329).
//!
//! `PgKioskPorts::payment_rules` is the read only `rates` uses; returning the defaults there
//! went unnoticed until the route printed `PublicRates.payment` (PR 387 review). v2 prints all
//! five channels with their switch and no `account` (`payment-settings.mjs:18`, `publicMethods`),
//! so a channel the owner switched off is printed with `"enabled": false`.
//!
//! Needs `TEST_DATABASE_URL` naming a database ending `_test`.

mod common;
mod compat_common;
mod kiosk_http_common;
mod rpc_common;
mod snapshot_common;

use kiosk_http_common::{Counter, as_who};
use rpc_common::Call;
use serde_json::{Value, json};

async fn payment_methods(counter: &Counter, action: &str, payload: Value) -> Value {
    let call = Call::action(&counter.origin, "/api/payment-methods", action, payload);
    let answer = counter
        .app
        .send(as_who(call, &counter.cookies, "owner"))
        .await;
    assert_eq!(answer.status, 200, "{action}: {}", answer.text);
    answer.json()
}

/// The rows as `rates` prints them: the saved rows without `account`.
fn public(rows: &Value) -> Value {
    Value::Array(
        rows.as_array()
            .expect("methods is an array")
            .iter()
            .map(|row| {
                let mut row = row.clone();
                row.as_object_mut()
                    .expect("a row is an object")
                    .remove("account");
                row
            })
            .collect(),
    )
}

#[actix_web::test]
async fn rates_prints_a_switched_off_channel_as_off() {
    let counter = Counter::open(&[]).await;

    let before = counter.ask(None, "rates", json!({})).await;
    assert_eq!(before.status, 200, "{}", before.text);
    let defaults = before.json()["paymentMethods"].clone();
    assert_eq!(
        defaults.as_array().map(Vec::len),
        Some(5),
        "{}",
        before.text
    );

    let listed = payment_methods(&counter, "list", json!({ "branchId": "MAIN" })).await;
    let mut rows = listed["methods"].clone();
    let off = rows
        .as_array_mut()
        .expect("methods is an array")
        .iter_mut()
        .find(|row| row["enabled"] == json!(true) && row["id"] != json!("Cash"))
        .expect("a switched-on channel besides Cash");
    off["enabled"] = json!(false);
    let off_id = off["id"].clone();
    let saved = payment_methods(
        &counter,
        "save",
        json!({ "branchId": "MAIN", "version": listed["version"], "methods": rows }),
    )
    .await;

    let after = counter.ask(None, "rates", json!({})).await;
    assert_eq!(after.status, 200, "{}", after.text);
    let printed = after.json()["paymentMethods"].clone();
    assert_eq!(printed, public(&saved["methods"]), "{}", after.text);
    assert_ne!(printed, defaults, "rates still prints the defaults");
    let row = printed
        .as_array()
        .expect("paymentMethods is an array")
        .iter()
        .find(|row| row["id"] == off_id)
        .expect("the switched-off channel is still listed, as v2 lists it");
    assert_eq!(row["enabled"], json!(false), "{}", after.text);
    assert!(
        printed
            .as_array()
            .expect("array")
            .iter()
            .all(|r| r.get("account").is_none()),
        "rates never prints an account: {}",
        after.text
    );

    counter.finish().await;
}
