//! Up-front test for the ticket "admin_branches lists other organisations' branches to an
//! org-wide caller" (written by claude/dell-claude-3 for the BE lane controller
//! claude/1656900d, before the fix). Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! `GET /api/v3/views/admin_branches` builds its list from `BranchService::list`, which reads
//! every row of `branches` (`crates/xc-rate/src/infrastructure/branch/branch_repository.rs:48`).
//! `SessionBranch::switchable` keeps the whole list for an org-wide role, so a second
//! organisation's branch reaches the owner's branch picker.
//!
//! Real sessions through the whole route table (`rpc_common`), as `v3_admin_branches_view_test.rs`
//! does. Needs `TEST_DATABASE_URL`; nothing skips.

mod common;
mod compat_common;
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

fn ids(body: &Value) -> Vec<String> {
    body["data"]["branches"]
        .as_array()
        .unwrap_or_else(|| panic!("a branch list: {body}"))
        .iter()
        .map(|b| b["id"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// The owner (an org-wide role of the default organisation) gets the default organisation's
/// branches in their order, and never the second organisation's `OB1`.
#[actix_web::test]
async fn the_owner_never_sees_another_organisations_branch() {
    let room = Room::open().await;
    for query in ["", "?branch=MAIN", "?branch=Z9"] {
        let path = format!("/api/v3/views/admin_branches{query}");
        let answer = room.get("OWNER", &path).await;
        assert_eq!(answer.status, 200, "GET {path}: {:.400}", answer.text);
        let body = answer.json();
        assert!(
            !ids(&body).iter().any(|id| id == OTHER_BRANCH),
            "GET {path}: another organisation's branch is listed: {body}"
        );
        assert_eq!(
            body["data"]["branches"],
            json!([{ "id": MAIN, "name": MAIN_NAME }, { "id": "Z9", "name": "Zulu" }]),
            "GET {path}: exactly the caller's organisation's branches, in order"
        );
    }
    room.finish().await;
}

/// Kept: an employee of `MAIN` still gets only `MAIN` (#460 §9).
#[actix_web::test]
async fn an_employee_still_gets_only_their_own_branch() {
    let room = Room::open().await;
    let answer = room.get("FRONT", "/api/v3/views/admin_branches?branch=MAIN").await;
    assert_eq!(answer.status, 200, "{:.400}", answer.text);
    assert_eq!(
        answer.json()["data"]["branches"],
        json!([{ "id": MAIN, "name": MAIN_NAME }])
    );
    room.finish().await;
}
