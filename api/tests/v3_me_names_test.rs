//! Ticket #402 (V1-account-bar), the server part, **rewritten for OR-7.7 #1030 by its ticket author
//! (claude/89bd61d9)**. #402 had the `me` view (`GET /api/v3/views/me`) answer `name` and `username` from the
//! `owners`/`employees` rows. The owner decided on 2026-10-10 ("roles + permissions") that `me` answers `id`,
//! `name`, `roles` and `permissions`, with `name` = `users.name` read by `users.id`, and **no** `kind`, `role` or
//! `username`. So these tests now pin that `me` no longer reads `owners` or `employees`: an old password cookie,
//! whose actor has no `users` row, answers `name: null` even though its `owners`/`employees` row has a name.
//!
//! The requests go through the server's whole route table (`bootstrap::configure`) with the real session reader
//! and real sign-ins. The Identity-session cases (a `users.name` that follows a rename) are in
//! `v3_me_roles_permissions_test.rs`.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` must name an existing database whose name ends
//! in `_test`. It fails without it; it does not skip. Each test works in a schema of its own.

mod auth_http_common;
mod common;

use actix_web::http::Method;
use auth_http_common::*;
use serde_json::{Value, json};

/// `GET /api/v3/views/me` with the given cookies; answers the `data` of a 200.
async fn me(app: &TestApp, call: Call, what: &str) -> Value {
    let answer = send(app, call).await;
    assert_eq!(
        answer.status, 200,
        "{what}: me answers 200: {}",
        answer.text
    );
    let body = answer.json();
    assert_eq!(
        body["changed"], true,
        "{what}: a first read is changed: {body}"
    );
    body["data"].clone()
}

fn get_me() -> Call {
    Call {
        method: Some(Method::GET),
        path: "/api/v3/views/me".to_owned(),
        proto: Some("https".to_owned()),
        ..Call::default()
    }
}

/// Gives an owner row a display name of its own, different from its username.
async fn rename_owner(desk: &Desk, username: &str, name: &str) {
    let done = sqlx::query("UPDATE owners SET name = $1 WHERE username = $2")
        .bind(name)
        .bind(username)
        .execute(&desk.pool)
        .await
        .expect("rename the owner");
    assert_eq!(done.rows_affected(), 1, "the owner {username} exists");
}

/// The keys `me` answers (OR-7.7 #1030).
fn me_keys(data: &Value) -> Vec<String> {
    let mut keys: Vec<String> = data
        .as_object()
        .unwrap_or_else(|| panic!("me data is an object: {data}"))
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

const NEW_KEYS: [&str; 7] = [
    "branch",
    "branchName",
    "branches",
    "id",
    "name",
    "permissions",
    "roles",
];

#[actix_web::test]
async fn the_owner_cookie_no_longer_reads_the_owners_row() {
    let desk = Desk::start().await;
    rename_owner(&desk, "boss", "Somchai").await;
    let app = desk_app!(desk);
    let owner = owner_signs_in(&app).await;

    let data = me(&app, get_me().owner(&owner), "the owner").await;
    assert_eq!(me_keys(&data), NEW_KEYS, "{data}");
    assert_eq!(data["id"], json!("owner"), "{data}");
    assert_eq!(
        data["name"],
        Value::Null,
        "not the owners row's \"Somchai\": {data}"
    );
    assert_eq!(
        data["roles"],
        json!([{ "role": "ADMIN", "branchId": null }, { "role": "EXECUTIVE", "branchId": null }]),
        "{data}"
    );

    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn a_staff_cookie_no_longer_reads_the_employees_row() {
    let desk = Desk::start().await;
    desk.branch("XC-B01").await;
    let app = desk_app!(desk);
    let owner = owner_signs_in(&app).await;
    let (nok_id, nok) = working_employee(&app, &desk, &owner, "nok", "FRONT", "XC-B01").await;
    let (_, _ploy) = working_employee(&app, &desk, &owner, "ploy", "RATES", "XC-B01").await;

    let data = me(&app, get_me().staff(&nok), "nok").await;
    assert_eq!(me_keys(&data), NEW_KEYS, "{data}");
    assert_eq!(data["id"], json!(nok_id), "{data}");
    assert_eq!(
        data["name"],
        Value::Null,
        "not the employees row's \"คุณ nok\": {data}"
    );
    assert_eq!(
        data["roles"],
        json!([{ "role": "TELLER", "branchId": "XC-B01" }]),
        "{data}"
    );
    assert_eq!(data["branch"], json!("XC-B01"), "{data}");

    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn the_owner_and_a_staff_member_in_one_browser_answer_the_owner() {
    let desk = Desk::start().await;
    desk.branch("XC-B01").await;
    rename_owner(&desk, "boss", "Somchai").await;
    let app = desk_app!(desk);
    let owner = owner_signs_in(&app).await;
    let (_, nok) = working_employee(&app, &desk, &owner, "nok", "FRONT", "XC-B01").await;

    // The `me` view answers the owner session first (`EntryPath::Rpc`, as `rpc.rs`).
    let data = me(&app, get_me().owner(&owner).staff(&nok), "owner and staff").await;
    assert_eq!(data["id"], json!("owner"), "{data}");
    assert_eq!(
        data["roles"],
        json!([{ "role": "ADMIN", "branchId": null }, { "role": "EXECUTIVE", "branchId": null }]),
        "{data}"
    );

    drop(app);
    desk.drop_schema().await;
}
