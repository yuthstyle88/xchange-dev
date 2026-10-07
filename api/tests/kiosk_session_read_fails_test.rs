//! The route's own answer when the session read fails behind it (#228 item 16).
//!
//! `POST /api/kiosk` reads the caller's session before it judges the action.
//! When that read fails at the database, the route must answer its own server
//! error — never treat the failure as "nobody signed in" and serve a signed-in
//! employee as an anonymous kiosk customer, for whom `rates` would still
//! answer the desk's rates.
//!
//! The failure is made real, not mocked: after a Front signs in through
//! [`Counter::open`], `employee_sessions` is renamed so the read cannot
//! succeed; the same `rates` call that answered 200 before must then answer
//! the route's 500 with [`SERVER_ERROR`] and `Cache-Control: no-store`, while
//! a call with no cookie still answers 200, because no session is read for
//! it — the 500 is shown to come from the session read. The table is put
//! back before `finish`.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod compat_common;
mod kiosk_http_common;
mod rpc_common;
mod snapshot_common;

use kiosk_http_common::{Counter, SERVER_ERROR, person};
use serde_json::json;

/// `rates` carrying a signed-in Front's cookie while `employee_sessions`
/// cannot be read: the route's own 500, the whole answer — status, the body
/// [`SERVER_ERROR`] byte for byte, and `Cache-Control: no-store`.
#[actix_web::test]
async fn rates_with_a_staff_cookie_is_the_routes_500_when_the_session_read_fails() {
    let counter = Counter::open(&[person("FR1", "FRONT", 0x11)]).await;

    // the same call before the table is broken answers the rates
    let before = counter.ask(Some("FR1"), "rates", json!({})).await;
    assert_eq!(before.status, 200, "before the break: {}", before.text);

    sqlx::query("ALTER TABLE employee_sessions RENAME TO employee_sessions_gone")
        .execute(&counter.desk.db.pool)
        .await
        .expect("rename employee_sessions so the session read fails");

    let refused = counter.ask(Some("FR1"), "rates", json!({})).await;
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
    let customer = counter.ask(None, "rates", json!({})).await;
    assert_eq!(
        customer.status, 200,
        "no session is read: {}",
        customer.text
    );

    sqlx::query("ALTER TABLE employee_sessions_gone RENAME TO employee_sessions")
        .execute(&counter.desk.db.pool)
        .await
        .expect("put employee_sessions back");

    counter.finish().await;
}
