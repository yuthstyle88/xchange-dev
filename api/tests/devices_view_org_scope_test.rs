//! Up-front test for the ticket "the devices view lists every organisation's branches"
//! (written by claude/dell-claude-3 for the BE lane controller claude/1656900d, before the fix).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `GET /api/v3/views/devices` answers `branches` from
//! `SELECT id, name FROM branches ORDER BY ...` with no organisation filter
//! (`api/src/presentation/http/v3/views/devices.rs:66`), so a second organisation's branch id
//! and name reach the devices screen's branch picker.
//!
//! Real sessions through the whole route table (`rpc_common`), as `v3_devices_view_test.rs`
//! does. Needs `TEST_DATABASE_URL`; nothing skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use actix_web::http::Method;
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";
const MAIN: &str = "MAIN";
/// The name the migration gives `MAIN`.
const MAIN_NAME: &str = "XC-B";
const OTHER_ORG: &str = "00000000-0000-0000-0000-0000000000b2";
const OTHER_BRANCH: &str = "OB1";

fn get(path: &str) -> Call {
    let mut call = Call::post(ORIGIN, path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    /// `MAIN` (from the migration) and `Z9` in the default organisation; a second organisation
    /// with its own branch `OB1`; the owner and one employee of every staff role in `MAIN`.
    async fn open() -> Room {
        let desk = Desk::open().await;
        sqlx::query(
            "INSERT INTO branches (id, name, created_at) \
             VALUES ('Z9', 'Zulu', '2026-10-07T01:00:00Z'::timestamptz)",
        )
        .execute(&desk.db.pool)
        .await
        .expect("insert branch Z9");
        sqlx::query(
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ($1::uuid, 'ORG2', 'Asia/Bangkok', now())",
        )
        .bind(OTHER_ORG)
        .execute(&desk.db.pool)
        .await
        .expect("insert the second organisation");
        sqlx::query(
            "INSERT INTO branches (id, name, created_at, organization_id) \
             VALUES ($1, 'Other org branch', '2026-10-07T02:00:00Z'::timestamptz, $2::uuid)",
        )
        .bind(OTHER_BRANCH)
        .bind(OTHER_ORG)
        .execute(&desk.db.pool)
        .await
        .expect("insert the other organisation's branch");
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        Room { desk, app, cookies }
    }

    async fn get(&self, who: &str, path: &str) -> Answer {
        self.app.send(get(path).cookie(&self.cookies[who])).await
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn branches(body: &Value) -> Value {
    body["data"]["branches"].clone()
}

/// Everyone who may read the devices view gets the caller's organisation's branches, in the
/// view's order (`MAIN` first, then by creation), and never the second organisation's `OB1`.
#[actix_web::test]
async fn the_devices_view_lists_only_the_callers_organisations_branches() {
    let room = Room::open().await;
    let expected = json!([{ "id": MAIN, "name": MAIN_NAME }, { "id": "Z9", "name": "Zulu" }]);
    for (who, query) in [
        ("OWNER", "?branch=MAIN"),
        ("OWNER", "?branch=Z9"),
        ("SYSTEM", ""),
    ] {
        let path = format!("/api/v3/views/devices{query}");
        let answer = room.get(who, &path).await;
        assert_eq!(
            answer.status, 200,
            "GET {path} as {who}: {:.400}",
            answer.text
        );
        let body = answer.json();
        let list = branches(&body);
        assert!(
            list.as_array()
                .is_some_and(|l| l.iter().all(|b| b["id"] != json!(OTHER_BRANCH))),
            "GET {path} as {who}: another organisation's branch is listed: {body}"
        );
        assert_eq!(
            list, expected,
            "GET {path} as {who}: exactly the caller's organisation's branches"
        );
    }
    room.finish().await;
}
