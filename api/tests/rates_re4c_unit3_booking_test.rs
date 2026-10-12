//! Ticket #757 (RE-4c) — error propagation on `/api/bookings` list action.
//! A booking stored with `unit_size` 3 returns HTTP 500 because `CompatRate::per_one`
//! refuses a unit size that is not a power of 10.

mod common;
mod compat_common;
mod rates_re4_common;
mod rpc_common;
mod snapshot_common;

use compat_common::Person;
use rpc_common::{Call, Desk, sign_in};
use serde_json::json;

const ORIGIN: &str = "http://desk.test";
const OWNER_ROUTE: &str = "/api/bookings";
const MAIN: &str = "MAIN";

fn owner() -> Person {
    Person {
        key: "OWNER".to_string(),
        kind: "owner".to_string(),
        id: "owner".to_string(),
        username: "owner".to_string(),
        name: "Owner".to_string(),
        role: "OWNER".to_string(),
        branch_id: None,
        must_change: false,
    }
}

#[tokio::test]
async fn booking_with_unit_size_3_answers_500_on_list() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let people = vec![owner()];
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;

    // Insert a booking with unit_size 3
    sqlx::query(
        "INSERT INTO v2_bookings (id, branch_id, request_id, request_hash, name, phone, type, code, \
         amount, rate, status, created_at, created_by, expires_at, confirmed_at, confirmed_by, \
         unit_size) \
         VALUES ($1, $2, $3, 'hash', 'Test Unit3', '0800000000', 'BUY', 'USD', 100, 32.1, \
         'CONFIRMED', now(), 'owner', now() + interval '1 day', now(), 'owner', 3)",
    )
    .bind("BK-UNIT3-0001")
    .bind(MAIN)
    .bind("req-unit3-1")
    .execute(&desk.db.pool)
    .await
    .expect("insert booking with unit_size 3");

    let body = json!({
        "action": "list",
        "payload": {
            "branchId": MAIN,
        }
    })
    .to_string();

    let call = Call::post(ORIGIN, OWNER_ROUTE, body).cookie(&cookies["OWNER"]);
    let answer = app.send(call).await;

    assert_eq!(
        answer.status, 500,
        "a booking stored with unit_size 3 answers 500 on list"
    );
}
