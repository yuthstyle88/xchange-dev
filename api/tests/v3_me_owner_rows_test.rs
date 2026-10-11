//! Ticket #451: pins the 'two owner rows -> no name' rule for the `me` view
//! (`GET /api/v3/views/me`, `api/src/presentation/http/v3/views/me.rs`).
//!
//! **Rewritten for OR-7.7 #1030 by its ticket author (claude/89bd61d9).** `me` no longer reads `owners` at all
//! (`name` = `users.name`; no `username`), so with two owner rows it still answers `name: null` and never picks
//! either row; it answers the new fields and none of `kind`, `role` and `username`.

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

/// Inserts a second owner row directly through SQL (create_owner refuses a second owner).
async fn insert_second_owner(desk: &Desk, username: &str, name: &str) {
    let done = sqlx::query(
        "INSERT INTO owners (id, username, name, password_hash, active, version, created_at) \
         VALUES ($1, $2, $3, $4, true, 1, NOW())",
    )
    .bind(format!("owner-{username}"))
    .bind(username)
    .bind(name)
    .bind("dummy_hash")
    .execute(&desk.pool)
    .await
    .expect("insert second owner");
    assert_eq!(done.rows_affected(), 1, "inserted second owner {username}");
}

#[actix_web::test]
async fn two_owner_rows_still_answer_a_null_name() {
    let desk = Desk::start().await;
    // Desk::start creates the first owner ("boss").
    // Insert a second owner directly through SQL.
    insert_second_owner(&desk, "second_owner", "Second Owner").await;

    let app = desk_app!(desk);
    let owner = owner_signs_in(&app).await;

    let data = me(&app, get_me().owner(&owner), "two owners").await;
    let mut keys: Vec<&str> = data
        .as_object()
        .unwrap_or_else(|| panic!("me data is an object: {data}"))
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "branch",
            "branchName",
            "branches",
            "id",
            "name",
            "permissions",
            "roles"
        ],
        "{data}"
    );
    assert_eq!(data["id"], json!("owner"), "{data}");
    assert_eq!(
        data["name"],
        Value::Null,
        "neither owner row is picked: {data}"
    );
    assert_eq!(
        data["roles"],
        json!([{ "role": "ADMIN", "branchId": null }, { "role": "EXECUTIVE", "branchId": null }]),
        "{data}"
    );
    assert_eq!(data["branch"], json!("MAIN"), "{data}");

    drop(app);
    desk.drop_schema().await;
}
