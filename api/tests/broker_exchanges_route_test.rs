//! Up-front tests for FD-3 (#974; design `docs/design/blueprint/broker-seam.md` §6.4, the wire of
//! Broker-Platform `docs/design/broker-platform.md` §12.3 at `04850fb9`): the route Broker pulls the
//! exchange feed from. Written by the ticket author before the implementation. Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! This test binary runs with `XC_BROKER_FEED_TOKEN` set (once, before any server is built). The
//! unset and blank cases run in their own binaries, because the token is read once at boot.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod broker_feed_common;
mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use std::sync::Once;

use broker_feed_common::{Row, completed, exchange, feed_row, get, ids, send};
use rpc_common::Desk;
use serde_json::{Value, json};

const TOKEN: &str = "fd3-feed-token-0123456789abcdef";

fn configure() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: called once, before any server of this binary reads the variable.
        unsafe { std::env::set_var("XC_BROKER_FEED_TOKEN", TOKEN) };
    });
}

macro_rules! open {
    () => {{
        configure();
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        (desk, app)
    }};
}

const T1: &str = "2026-10-06T01:00:00.000000Z";
const T2: &str = "2026-10-06T02:00:00.000000Z";
const T3: &str = "2026-10-06T03:00:00.000000Z";

fn assert_error_body(answer: &rpc_common::Answer, status: u16, label: &str) {
    assert_eq!(answer.status, status, "{label}: {:.300}", answer.text);
    let body: Value = serde_json::from_str(&answer.text)
        .unwrap_or_else(|_| panic!("{label}: a JSON body: {:.300}", answer.text));
    assert!(body["error"].is_string(), "{label}: {{error}}: {body}");
}

#[actix_web::test]
async fn the_right_token_without_since_reads_from_the_beginning() {
    let (desk, app) = open!();
    let a = exchange(&desk, 1).await;
    let b = exchange(&desk, 2).await;
    let c = exchange(&desk, 3).await;
    completed(&desk, &b, T2).await;
    completed(&desk, &c, T3).await;
    completed(&desk, &a, T1).await;

    let answer = send(&app, get("", Some(TOKEN))).await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    assert_eq!(answer.header("cache-control"), Some("no-store"));
    let page = answer.json();
    assert_eq!(
        ids(&page),
        [a, b, c.clone()],
        "oldest first, by (updated_at, id)"
    );
    assert_eq!(
        page["nextCursor"],
        json!(format!("{T3}_{c}")),
        "the last row's cursor"
    );
    desk.finish().await;
}

#[actix_web::test]
async fn the_page_is_brokers_wire_shape() {
    let (desk, app) = open!();
    let done = exchange(&desk, 1).await;
    let back = exchange(&desk, 2).await;
    feed_row(
        &desk,
        &Row {
            id: &done,
            customer_sub: Some("sub-1"),
            phone: Some("0899000111"),
            status: "COMPLETED",
            value_satang: 326_000,
            reversed_satang: 0,
            completed_at: T1,
            reversed_at: None,
            updated_at: T1,
        },
    )
    .await;
    feed_row(
        &desk,
        &Row {
            id: &back,
            customer_sub: None,
            phone: None,
            status: "REVERSED",
            value_satang: 326_000,
            reversed_satang: 100_000,
            completed_at: T1,
            reversed_at: Some(T2),
            updated_at: T2,
        },
    )
    .await;

    let page = send(&app, get("limit=200", Some(TOKEN))).await.json();
    let keys = |v: &Value| {
        let mut k: Vec<String> = v.as_object().expect("an object").keys().cloned().collect();
        k.sort();
        k
    };
    assert_eq!(keys(&page), ["items", "nextCursor"]);
    let items = page["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    for item in items {
        assert_eq!(
            keys(item),
            [
                "completedAt",
                "customerSub",
                "id",
                "phone",
                "reversedAt",
                "reversedSatang",
                "status",
                "updatedAt",
                "valueSatang"
            ]
        );
    }
    // Field by field, so a format drift names itself.
    let one = &items[0];
    assert_eq!(one["id"], json!(done));
    assert_eq!(one["customerSub"], json!("sub-1"));
    assert_eq!(one["phone"], json!("0899000111"));
    assert_eq!(one["status"], json!("COMPLETED"));
    assert!(one["valueSatang"].is_i64() && one["valueSatang"] == json!(326000));
    assert!(one["reversedSatang"].is_i64() && one["reversedSatang"] == json!(0));
    assert_eq!(one["reversedAt"], Value::Null);
    let two = &items[1];
    assert_eq!(two["status"], json!("REVERSED"));
    assert_eq!(two["customerSub"], Value::Null);
    assert_eq!(two["phone"], Value::Null);
    assert_eq!(two["reversedSatang"], json!(100000));
    for (item, field) in [
        (one, "completedAt"),
        (one, "updatedAt"),
        (two, "completedAt"),
        (two, "reversedAt"),
        (two, "updatedAt"),
    ] {
        let text = item[field]
            .as_str()
            .unwrap_or_else(|| panic!("{field} is a string"));
        assert!(text.ends_with('Z'), "{field} is UTC with Z: {text}");
        let parsed = chrono::DateTime::parse_from_rfc3339(text)
            .unwrap_or_else(|e| panic!("{field} is RFC 3339: {text}: {e}"));
        assert_eq!(parsed.offset().local_minus_utc(), 0, "{field}: {text}");
    }
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(two["reversedAt"].as_str().unwrap()).unwrap(),
        chrono::DateTime::parse_from_rfc3339(T2).unwrap()
    );
    desk.finish().await;
}

#[actix_web::test]
async fn limit_pages_and_next_cursor_continues_until_an_empty_page() {
    let (desk, app) = open!();
    let a = exchange(&desk, 1).await;
    let b = exchange(&desk, 2).await;
    let c = exchange(&desk, 3).await;
    completed(&desk, &a, T1).await;
    completed(&desk, &b, T2).await;
    completed(&desk, &c, T3).await;

    let first = send(&app, get("limit=2", Some(TOKEN))).await;
    assert_eq!(first.status, 200, "{:.300}", first.text);
    let first = first.json();
    assert_eq!(ids(&first), [a, b.clone()]);
    let cursor = first["nextCursor"].as_str().expect("a cursor").to_string();
    assert_eq!(cursor, format!("{T2}_{b}"));

    let second = send(&app, get(&format!("since={cursor}&limit=2"), Some(TOKEN)))
        .await
        .json();
    assert_eq!(ids(&second), std::slice::from_ref(&c));
    let cursor = second["nextCursor"].as_str().expect("a cursor").to_string();

    let third = send(&app, get(&format!("since={cursor}&limit=2"), Some(TOKEN))).await;
    assert_eq!(third.status, 200);
    assert_eq!(third.json(), json!({ "items": [], "nextCursor": null }));
    desk.finish().await;
}

#[actix_web::test]
async fn a_lookback_cursor_with_the_nil_uuid_is_accepted() {
    let (desk, app) = open!();
    let a = exchange(&desk, 1).await;
    let b = exchange(&desk, 2).await;
    completed(&desk, &a, T1).await;
    completed(&desk, &b, T3).await;
    // Broker's tick starts at "stored − 10 min": a time and the nil uuid.
    let since = "2026-10-06T02:50:00.000000Z_00000000-0000-0000-0000-000000000000";
    let answer = send(&app, get(&format!("since={since}"), Some(TOKEN))).await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    assert_eq!(ids(&answer.json()), [b]);
    desk.finish().await;
}

#[actix_web::test]
async fn an_empty_feed_is_an_empty_page() {
    let (desk, app) = open!();
    let answer = send(&app, get("", Some(TOKEN))).await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    assert_eq!(answer.header("cache-control"), Some("no-store"));
    assert_eq!(answer.json(), json!({ "items": [], "nextCursor": null }));
    desk.finish().await;
}

#[actix_web::test]
async fn a_bad_limit_or_since_is_400() {
    let (desk, app) = open!();
    for query in [
        "limit=0",
        "limit=201",
        "limit=-1",
        "limit=abc",
        "since=not-a-cursor",
        "since=2026-10-06T01:00:00.000000Z",
        "since=2026-10-06T01:00:00.000000Z_not-a-uuid",
        "since=yesterday_00000000-0000-0000-0000-000000000000",
    ] {
        let answer = send(&app, get(query, Some(TOKEN))).await;
        assert_error_body(&answer, 400, query);
    }
    desk.finish().await;
}

#[actix_web::test]
async fn a_missing_or_wrong_token_is_401_and_never_echoed() {
    let (desk, app) = open!();
    let a = exchange(&desk, 1).await;
    completed(&desk, &a, T1).await;
    let wrong = "wrong-token-value-xyz";
    for (token, label) in [
        (None, "no header"),
        (Some(""), "empty"),
        (Some(wrong), "wrong"),
        (Some("fd3-feed-token-0123456789abcde"), "a prefix"),
        (
            Some("fd3-feed-token-0123456789abcdef "),
            "with a trailing space",
        ),
    ] {
        let answer = send(&app, get("", token)).await;
        assert_error_body(&answer, 401, label);
        assert!(
            !answer.text.contains(TOKEN),
            "{label}: the body echoes the token"
        );
        assert!(
            !answer.text.contains(wrong),
            "{label}: the body echoes what was sent"
        );
        assert!(!answer.text.contains(&a), "{label}: no feed data leaks");
    }
    desk.finish().await;
}

#[actix_web::test]
async fn a_signed_in_owner_without_the_token_is_401() {
    let (desk, app) = open!();
    let mut conn = desk.db.pool.acquire().await.expect("a connection");
    let valid_from = chrono::Utc::now() - chrono::Duration::hours(1);
    let owner = xc_testdb::people::seed_person(
        &mut conn,
        "00000000-0000-0000-0000-000000000001",
        "Owner",
        &[
            xc_testdb::people::RoleRow {
                role_code: "EXECUTIVE",
                branch_id: None,
                valid_from,
                valid_to: None,
            },
            xc_testdb::people::RoleRow {
                role_code: "ADMIN",
                branch_id: None,
                valid_from,
                valid_to: None,
            },
        ],
    )
    .await
    .expect("seed owner");
    let token = xc_testdb::sessions::seed_session(
        &mut conn,
        &owner,
        None,
        chrono::Utc::now() + chrono::Duration::hours(1),
    )
    .await
    .expect("seed session");
    drop(conn);
    let cookie = xc_testdb::sessions::cookie(&token);
    let answer = send(&app, get("", None).cookie(&cookie)).await;
    assert_error_body(&answer, 401, "a session and no token");
    desk.finish().await;
}

#[actix_web::test]
async fn a_database_failure_is_500() {
    let (desk, app) = open!();
    sqlx::query("ALTER TABLE broker_exchange_feed RENAME TO broker_exchange_feed_gone")
        .execute(&desk.db.pool)
        .await
        .expect("hide the table");
    let answer = send(&app, get("", Some(TOKEN))).await;
    assert_error_body(&answer, 500, "the feed table is gone");
    sqlx::query("ALTER TABLE broker_exchange_feed_gone RENAME TO broker_exchange_feed")
        .execute(&desk.db.pool)
        .await
        .expect("restore the table");
    desk.finish().await;
}

/// Without `limit`, a page holds at most 200 rows (Broker asks `limit=200`, design §6.4).
#[actix_web::test]
async fn without_limit_a_page_holds_200_rows() {
    let (desk, app) = open!();
    sqlx::query(
        "WITH c AS ( \
           INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
           SELECT 'CUST-BULK-' || n, b.organization_id, 'CUST-BULK-' || n, 'Bulk', 'LIVE', now() \
           FROM branches b, generate_series(1, 201) n WHERE b.id = 'MAIN' RETURNING id), \
         t AS ( \
           INSERT INTO transactions (document_ref, branch_id, mode, submission_id, payload_hash, \
               customer_id, customer_name, nationality, staff_id, booking_no, notes, \
               value_date, paid_at, recorded_at, recorded_by, completed_at) \
           SELECT 'DOC-' || c.id, 'MAIN', 'DEMO', 'SUB-' || c.id, 'h', c.id, 'Bulk', 'THA', \
               'owner', '', '', \
               DATE '2026-10-06', now(), now(), 'owner', now() FROM c RETURNING id) \
         INSERT INTO broker_exchange_feed (transaction_id, organization_id, branch_id, status, \
             value_satang, reversed_satang, completed_at, updated_at) \
         SELECT t.id, b.organization_id, 'MAIN', 'COMPLETED', 326000, 0, now(), now() \
         FROM t, branches b WHERE b.id = 'MAIN'",
    )
    .execute(&desk.db.pool)
    .await
    .expect("seed 201 feed rows");
    let answer = send(&app, get("", Some(TOKEN))).await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    let page = answer.json();
    assert_eq!(page["items"].as_array().expect("items").len(), 200);
    assert!(page["nextCursor"].is_string(), "more to read");
    desk.finish().await;
}
