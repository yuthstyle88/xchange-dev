//! Ticket #451: pins the 'two owner rows -> no name' rule for the `me` view
//! (`GET /api/v3/views/me`, `api/src/presentation/http/v3/views/me.rs`).
//!
//! With two owner rows in the database, `me` must answer `name: null` and
//! `username: null`. It must never guess or pick the first row.

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
async fn two_owner_rows_answers_null_name_and_username() {
    let desk = Desk::start().await;
    // Desk::start creates the first owner ("boss").
    // Insert a second owner directly through SQL.
    insert_second_owner(&desk, "second_owner", "Second Owner").await;

    let app = desk_app!(desk);
    let owner = owner_signs_in(&app).await;

    let data = me(&app, get_me().owner(&owner), "two owners").await;
    assert_eq!(
        data,
        json!({
            "kind": "owner", "id": "owner", "role": "OWNER", "branch": "MAIN",
            "branchName": "XC-B", "branches": [{ "id": "MAIN", "code": "MAIN", "name": "XC-B" }],
            "name": null, "username": null
        })
    );

    drop(app);
    desk.drop_schema().await;
}
