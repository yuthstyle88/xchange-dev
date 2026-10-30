//! #5629 — `GET /_xc/device-agents` (the owner's pairing page) and an organisation-wide EXECUTIVE
//! with no ADMIN (design `docs/design/org-wide-actor-branch.md` §13, rows P-1..P-2).
//! Written before the implementation by the sub-controller (claude/studio-claude-17). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! The page's API, `POST /api/device-setup`, needs `SystemSettings`, which an EXECUTIVE alone holds.
//! The page itself admits only `owner_standing()` (EXECUTIVE + ADMIN, `device_setup.rs:66`), so an
//! EXECUTIVE alone is sent to `/login` from a page whose every action it may take. Under the rule of
//! §13 (V.9's OWNER maps to EXECUTIVE, RBAC §11.2) the page admits it too. Every other caller keeps
//! today's answer (`device_setup_http_test::the_owner_s_page_is_served_to_the_owner_only`).
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use chrono::{DateTime, Duration, Utc};
use rpc_common::{Call, Desk, one_of_each_role, sign_in};

const ORIGIN: &str = "http://desk.test";
const PAGE: &str = "/_xc/device-agents";
const EXEC_ID: &str = "00000000-0000-0000-0000-00000000e5e2";

/// A session cookie for an organisation-wide EXECUTIVE with no other row, at `MAIN`.
async fn executive_alone(desk: &Desk) -> String {
    let pool = &desk.db.pool;
    let org: String =
        sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = 'MAIN'")
            .fetch_one(pool)
            .await
            .expect("MAIN's organisation");
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by, created_at) \
         VALUES ($1::uuid, $2::uuid, 'exec.alone', 'ACTIVE', 'test', '2026-10-01T00:00:00Z')",
    )
    .bind(EXEC_ID)
    .bind(&org)
    .execute(pool)
    .await
    .expect("insert the executive");
    let mut conn = pool.acquire().await.expect("acquire");
    xc_testdb::people::seed_roles(
        &mut conn,
        EXEC_ID,
        &org,
        &[xc_testdb::people::RoleRow {
            role_code: "EXECUTIVE",
            branch_id: None,
            valid_from: DateTime::<Utc>::UNIX_EPOCH,
            valid_to: None,
        }],
    )
    .await
    .expect("seed EXECUTIVE organisation-wide");
    let token = xc_testdb::sessions::seed_session(
        &mut conn,
        EXEC_ID,
        Some("MAIN"),
        Utc::now() + Duration::hours(1),
    )
    .await
    .expect("sign the executive in");
    xc_testdb::sessions::cookie(&token)
}

fn get(cookie: &str) -> Call {
    let mut call = Call::post(ORIGIN, PAGE, "")
        .without("x-xchange-request")
        .without("content-type");
    call.method = actix_web::http::Method::GET;
    call.cookie(cookie)
}

/// P-1: an EXECUTIVE alone gets the page: 200, HTML, never cached.
#[actix_web::test]
async fn p1_an_executive_alone_gets_the_owner_s_page() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let cookie = executive_alone(&desk).await;
    let page = app.send(get(&cookie)).await;
    assert_eq!(page.status, 200, "P-1: {:.300}", page.text);
    assert!(
        page.header("content-type")
            .is_some_and(|c| c.starts_with("text/html")),
        "P-1: {:?}",
        page.header("content-type")
    );
    assert!(
        page.header("cache-control")
            .is_some_and(|c| c.contains("no-store")),
        "P-1: never cached"
    );
    desk.finish().await;
}

/// P-2: the owner still gets it; every staff role (ADMIN at a branch included) is still sent to
/// `/login`.
#[actix_web::test]
async fn p2_the_owner_and_staff_are_unchanged() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let people = one_of_each_role();
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;
    for (key, cookie) in &cookies {
        let answer = app.send(get(cookie)).await;
        if key == "OWNER" {
            assert_eq!(answer.status, 200, "P-2 {key}: {:.200}", answer.text);
        } else {
            assert_eq!(answer.status, 303, "P-2 {key}: {:.200}", answer.text);
            assert!(
                answer
                    .header("location")
                    .is_some_and(|l| l.ends_with("/login")),
                "P-2 {key}: {:?}",
                answer.header("location")
            );
        }
    }
    desk.finish().await;
}
