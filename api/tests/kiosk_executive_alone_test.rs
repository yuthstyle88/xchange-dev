//! #5629 — `/api/kiosk` and an organisation-wide EXECUTIVE with no ADMIN (design
//! `docs/design/org-wide-actor-branch.md` §13, rows K-1..K-5).
//! Written before the implementation by the sub-controller (claude/studio-claude-17). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! An EXECUTIVE alone holds `RatesDraft`, `SignedIn` and `QueueOperate` (RBAC §11.2), so the kiosk
//! policy admits its staff actions. Today `kiosk_actor_from_callers` (`kiosk.rs:97`) hands the
//! service `KioskActor::Customer`, because neither `owner_standing` nor `staff_standing` answers
//! it, and `notifications` answers 500 (`kiosk.rs:481-503`). It must act at the kiosk as the
//! owner does (`KioskRole::Owner`, #5618 K-1), under its own user id, and its employee line says
//! `EXECUTIVE`.

mod common;
mod compat_common;
#[macro_use]
mod kiosk_http_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use chrono::{DateTime, Duration, Utc};
use kiosk_http_common::Counter;
use serde_json::json;

const EXEC_ID: &str = "00000000-0000-0000-0000-00000000e5e1";
const EXEC_NAME: &str = "exec.alone";
const EXEC_KEY: &str = "exec-alone";

/// Employs an organisation-wide EXECUTIVE with no other row and signs it in at `MAIN`.
async fn with_an_executive_alone(counter: &mut Counter) {
    let pool = &counter.desk.db.pool;
    let org: String =
        sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = 'MAIN'")
            .fetch_one(pool)
            .await
            .expect("MAIN's organisation");
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by, created_at) \
         VALUES ($1::uuid, $2::uuid, $3, 'ACTIVE', 'test', '2026-10-01T00:00:00Z')",
    )
    .bind(EXEC_ID)
    .bind(&org)
    .bind(EXEC_NAME)
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
    counter
        .cookies
        .insert(EXEC_KEY.to_string(), xc_testdb::sessions::cookie(&token));
}

/// K-1: `notifications` answers 200, and its employee line is the executive's own.
#[actix_web::test]
async fn k1_an_executive_alone_reads_its_notifications() {
    let mut counter = Counter::open(&[]).await;
    with_an_executive_alone(&mut counter).await;
    let answer = counter
        .ask(Some(EXEC_KEY), "notifications", json!({}))
        .await;
    assert_eq!(answer.status, 200, "K-1: {}", answer.text);
    let employee = &answer.json()["employee"];
    assert_eq!(employee["id"], EXEC_ID, "K-1: {}", answer.text);
    assert_eq!(employee["username"], EXEC_NAME, "K-1: {}", answer.text);
    assert_eq!(employee["name"], EXEC_NAME, "K-1: {}", answer.text);
    assert_eq!(employee["role"], "EXECUTIVE", "K-1: {}", answer.text);
    counter.finish().await;
}

/// K-2: the owner's employee line is unchanged.
#[actix_web::test]
async fn k2_the_owner_s_notifications_are_unchanged() {
    let counter = Counter::open(&[]).await;
    let answer = counter.ask(Some("owner"), "notifications", json!({})).await;
    assert_eq!(answer.status, 200, "K-2: {}", answer.text);
    let employee = &answer.json()["employee"];
    assert_eq!(employee["id"], "owner", "K-2: {}", answer.text);
    assert_eq!(employee["role"], "OWNER", "K-2: {}", answer.text);
    counter.finish().await;
}

/// K-3: `manage` (the owner's or a rate manager's) answers an EXECUTIVE alone as it answers the
/// owner: 200 with the currency table.
#[actix_web::test]
async fn k3_an_executive_alone_manages_the_kiosk_as_the_owner() {
    let mut counter = Counter::open(&[]).await;
    with_an_executive_alone(&mut counter).await;
    let owner = counter.ask(Some("owner"), "manage", json!({})).await;
    assert_eq!(owner.status, 200, "K-3 owner: {}", owner.text);
    let exec = counter.ask(Some(EXEC_KEY), "manage", json!({})).await;
    assert_eq!(exec.status, 200, "K-3: {}", exec.text);
    assert_eq!(
        exec.json()["currencies"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(0),
        owner.json()["currencies"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(usize::MAX),
        "K-3: the same table as the owner's"
    );
    counter.finish().await;
}

/// K-4: `list` (staff, `SignedIn`) answers an EXECUTIVE alone as it answers the owner: 200.
/// Today the service is handed `KioskActor::Customer`.
#[actix_web::test]
async fn k4_an_executive_alone_lists_the_requests_as_the_owner() {
    let mut counter = Counter::open(&[]).await;
    with_an_executive_alone(&mut counter).await;
    let owner = counter.ask(Some("owner"), "list", json!({})).await;
    assert_eq!(owner.status, 200, "K-4 owner: {}", owner.text);
    let exec = counter.ask(Some(EXEC_KEY), "list", json!({})).await;
    assert_eq!(exec.status, 200, "K-4: {}", exec.text);
    counter.finish().await;
}

/// K-5: an EXECUTIVE alone cancels a waiting request for a staff reason, as the owner may. Today
/// the service is handed `KioskActor::Customer`, and a customer may not cancel for a staff reason
/// (`KioskMachine::cancel`, `NotCancellable`).
#[actix_web::test]
async fn k5_an_executive_alone_cancels_a_waiting_request_for_a_staff_reason() {
    let mut counter = Counter::open(&[kiosk_http_common::person("FR1", "FRONT", 0x51)]).await;
    let id = counter
        .waiting_request("00000000-0000-4000-8000-000000562905")
        .await;
    with_an_executive_alone(&mut counter).await;
    let answer = counter
        .ask(
            Some(EXEC_KEY),
            "cancel",
            json!({ "id": id, "reason": "DUPLICATE" }),
        )
        .await;
    assert_eq!(answer.status, 200, "K-5: {}", answer.text);
    let status: String = sqlx::query_scalar("SELECT status FROM kiosk_requests WHERE id = $1")
        .bind(&id)
        .fetch_one(&counter.desk.db.pool)
        .await
        .expect("read the request");
    assert_eq!(status, "CANCELLED", "K-5");
    counter.finish().await;
}
