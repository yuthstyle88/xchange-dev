//! The route's own answer when the session read fails behind it (#228 item 16).
//!
//! `POST /api/kiosk` reads the caller's session before it judges the action.
//! When that read fails at the database, the route must answer its own server
//! error — never treat the failure as "nobody signed in" and serve a signed-in
//! teller as an anonymous kiosk customer, for whom `rates` would still answer
//! the desk's rates.
//!
//! Amended for OR-7.m (#1027) by the sub-controller claude/ba2128ed: the
//! password sessions are gone, so the teller signs in through a Blueprint
//! session (`__Host-xc_session`, a `user_sessions` row from
//! [`xc_testdb::sessions::seed_session`]) and `user_sessions` is the table
//! that is broken. The failure is made real, not mocked: the same `rates` call
//! that answered 200 before must then answer the route's 500 with
//! [`SERVER_ERROR`] and `Cache-Control: no-store`, while a call with no cookie
//! still answers 200, because no session is read for it — the 500 is shown to
//! come from the session read. The table is put back before `finish`.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod kiosk_http_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use kiosk_http_common::{Counter, SERVER_ERROR, kiosk};
use rpc_common::MAIN;
use serde_json::json;

/// `rates` carrying a signed-in teller's Blueprint cookie while `user_sessions`
/// cannot be read: the route's own 500, the whole answer — status, the body
/// [`SERVER_ERROR`] byte for byte, and `Cache-Control: no-store`.
#[actix_web::test]
async fn rates_with_a_staff_cookie_is_the_routes_500_when_the_session_read_fails() {
    let counter = Counter::open(&[]).await;
    let pool = &counter.desk.db.pool;

    let teller: String = sqlx::query_scalar(
        "WITH b AS (SELECT id, organization_id FROM branches WHERE id = $1), \
         u AS (INSERT INTO users (organization_id, name, status, created_by) \
               SELECT organization_id, 'Teller of the test', 'ACTIVE', 'test' FROM b \
               RETURNING id, organization_id) \
         INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
         SELECT u.id, 'TELLER', u.organization_id, b.id FROM u, b RETURNING user_id::text",
    )
    .bind(MAIN)
    .fetch_one(pool)
    .await
    .expect("seed a teller at MAIN");
    let mut conn = pool.acquire().await.expect("a connection");
    let token = xc_testdb::sessions::seed_session(
        &mut conn,
        &teller,
        Some(MAIN),
        chrono::Utc::now() + chrono::Duration::hours(1),
    )
    .await
    .expect("seed the teller's session");
    drop(conn);
    let cookie = xc_testdb::sessions::cookie(&token);
    let as_teller = || kiosk(&counter.origin, "rates", json!({})).cookie(&cookie);

    // the same call before the table is broken answers the rates
    let before = counter.app.send(as_teller()).await;
    assert_eq!(before.status, 200, "before the break: {}", before.text);

    sqlx::query("ALTER TABLE user_sessions RENAME TO user_sessions_gone")
        .execute(pool)
        .await
        .expect("rename user_sessions so the session read fails");

    let refused = counter.app.send(as_teller()).await;
    let customer = counter.ask(None, "rates", json!({})).await;

    sqlx::query("ALTER TABLE user_sessions_gone RENAME TO user_sessions")
        .execute(pool)
        .await
        .expect("put user_sessions back");

    assert_eq!(
        refused.status, 500,
        "the session read failed behind the route: {}",
        refused.text
    );
    refused.assert_refused(500, SERVER_ERROR, "the route's own error");
    assert_eq!(
        refused.header("cache-control"),
        Some("no-store"),
        "the failure is never cached"
    );

    // no cookie: no session is read, so the customer's rates still answer
    assert_eq!(
        customer.status, 200,
        "no session is read: {}",
        customer.text
    );

    counter.finish().await;
}
