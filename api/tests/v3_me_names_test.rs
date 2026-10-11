//! Ticket #402 (V1-account-bar), the server part: the `me` view (`GET /api/v3/views/me`,
//! `api/src/presentation/http/v3/views/me.rs`) answers the signed-in account's own `name` and
//! `username`, so the account bar can show v2's `{name} · {username} · {role}`
//! (xchange-v2-source@4d0bdbb:src/devices-public/account-bar.js:8).
//!
//! Written before the implementation by the lane controller. Do not edit, delete or weaken.
//!
//! What it pins (lane controller decisions on #402, 2026-10-08):
//! - `Me` (`api/src/presentation/dto/v3/common.rs`) gains `name` and `username`, serialised
//!   under those names, as `Option<String>` (TS `string | null`).
//! - They come from the row of the account the session belongs to, read the way `/api/auth`
//!   `status` reads it (`api/src/presentation/http/auth.rs:93-100`, `user_json` `:65-74`): the
//!   owner's own `owners` row (v2 `src/auth.mjs:43-45` finds it through the owner session), and
//!   the employee's `employees` row. The owner's `id` stays the constant `OWNER_ID` (`"owner"`).
//! - There is one owner row at most: the repository refuses a second one
//!   (`api/crates/xc-auth/src/infrastructure/account/account_repository.rs:450-456`), so no test
//!   here makes two.
//!
//! The requests go through the server's whole route table (`bootstrap::configure`) with the real
//! session reader and real sign-ins, so how the view reaches the row is not pinned here.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` must name an existing database whose name ends
//! in `_test`. It fails without it; it does not skip. Each test works in a schema of its own.

mod auth_http_common;
mod common;

use std::path::Path;

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

#[actix_web::test]
async fn the_owner_sees_their_own_name_and_username() {
    let desk = Desk::start().await;
    rename_owner(&desk, "boss", "Somchai").await;
    let app = desk_app!(desk);
    let owner = owner_signs_in(&app).await;

    let data = me(&app, get_me().owner(&owner), "the owner").await;
    assert_eq!(
        data,
        json!({
            "kind": "owner", "id": "owner", "role": "OWNER", "branch": "MAIN",
            "branchName": "XC-B", "branches": [{ "id": "MAIN", "code": "MAIN", "name": "XC-B" }],
            "name": "Somchai", "username": "boss"
        })
    );

    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn a_staff_member_sees_their_own_name_and_username() {
    let desk = Desk::start().await;
    desk.branch("XC-B01").await;
    let app = desk_app!(desk);
    let owner = owner_signs_in(&app).await;
    let (nok_id, nok) = working_employee(&app, &desk, &owner, "nok", "FRONT", "XC-B01").await;
    let (_, _ploy) = working_employee(&app, &desk, &owner, "ploy", "RATES", "XC-B01").await;

    let data = me(&app, get_me().staff(&nok), "nok").await;
    assert_eq!(
        data,
        json!({
            "kind": "staff", "id": nok_id, "role": "FRONT", "branch": "XC-B01",
            "branchName": "XC-B01", "branches": [{ "id": "XC-B01", "code": "XC-B01", "name": "XC-B01" }],
            "name": "คุณ nok", "username": "nok"
        })
    );

    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn the_owner_and_a_staff_member_in_one_browser_answer_the_owners_row() {
    let desk = Desk::start().await;
    desk.branch("XC-B01").await;
    rename_owner(&desk, "boss", "Somchai").await;
    let app = desk_app!(desk);
    let owner = owner_signs_in(&app).await;
    let (_, nok) = working_employee(&app, &desk, &owner, "nok", "FRONT", "XC-B01").await;

    // The `me` view answers the owner session first (`EntryPath::Rpc`, as `rpc.rs`), and its
    // names are of the same account it answers for.
    let data = me(&app, get_me().owner(&owner).staff(&nok), "owner and staff").await;
    assert_eq!(data["kind"], "owner", "{data}");
    assert_eq!(data["name"], "Somchai", "{data}");
    assert_eq!(data["username"], "boss", "{data}");

    drop(app);
    desk.drop_schema().await;
}

#[test]
fn the_generated_me_type_has_name_and_username_as_string_or_null() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/src/shared/api/generated/common.ts");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let me = text
        .lines()
        .find(|l| l.starts_with("export type Me = "))
        .unwrap_or_else(|| panic!("an `export type Me` line in {}", path.display()));
    assert!(
        me.contains(" name: string | null,"),
        "Me has `name: string | null`: {me}"
    );
    assert!(
        me.contains(" username: string | null,"),
        "Me has `username: string | null`: {me}"
    );
    for kept in [
        " kind: string,",
        " id: string,",
        " role: string,",
        " branch: string,",
    ] {
        assert!(me.contains(kept), "Me keeps `{}`: {me}", kept.trim());
    }
}
