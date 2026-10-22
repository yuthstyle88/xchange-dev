//! KWQ-B1 — `GET /api/public/orgs/{org}/branches/{code}/queue-display` (design
//! `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D10, §4.3a W9, §4.4 B17): the public,
//! numbers-only calling display, through the whole route table on a real PostgreSQL.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! The answer's shape is the header of `api/src/presentation/http/public/queue_display.rs`.
//! Seeds: `quote_common::TestDb` (as `public_rates_etag_test.rs`), the seeded organisation `XC`.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

mod common;
mod quote_common;

use std::collections::BTreeSet;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, test as atest};
use quote_common::TestDb;
use serde_json::{Value, json};
use xchange::bootstrap::{self, AppState};

const XC: &str = "XC";

struct Answer {
    status: StatusCode,
    cache_control: Option<String>,
    body: Value,
    text: String,
}

async fn display(db: &TestDb, org: &str, code: &str, since: Option<&str>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let app =
        atest::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state)))
            .await;
    let uri = match since {
        Some(s) => format!("/api/public/orgs/{org}/branches/{code}/queue-display?since={s}"),
        None => format!("/api/public/orgs/{org}/branches/{code}/queue-display"),
    };
    let req = atest::TestRequest::get()
        .uri(&uri)
        .insert_header((header::HOST, "xc.test"))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let cache_control = resp
        .headers()
        .get(header::CACHE_CONTROL)
        .map(|v| v.to_str().expect("ASCII").to_owned());
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer {
        status,
        cache_control,
        body,
        text,
    }
}

/// A branch of `XC` with its service status row at `version`.
async fn branch(db: &TestDb, id: &str, code: &str, version: i64) {
    db.exec(&format!(
        "INSERT INTO branches (id, name, code) VALUES ('{id}', 'Branch {code}', '{code}')"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO branch_service_status (branch_id, organization_id, queue_open, queue_version) \
         SELECT id, organization_id, true, {version} FROM branches WHERE id = '{id}' \
         ON CONFLICT (branch_id) DO UPDATE SET queue_version = {version}"
    ))
    .await;
}

/// A walk-in ticket `W<seq>` of `branch` in `status`, called `called_min_ago` minutes ago (or
/// never), dated `days_ago` days before the branch's local today, with a customer on it.
async fn ticket(
    db: &TestDb,
    branch: &str,
    seq: i32,
    status: &str,
    called_min_ago: Option<i64>,
    days_ago: i64,
) {
    let called = called_min_ago.map_or("NULL".to_owned(), |m| {
        format!("now() - interval '{m} minutes'")
    });
    db.exec(&format!(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, \
             sequence, display_number, status, customer_id, issued_at, called_at, idempotency_key) \
         SELECT gen_random_uuid(), b.organization_id, b.id, \
             (now() AT TIME ZONE b.timezone)::date - {days_ago}, 'WALK_IN', {seq}, \
             'W' || lpad('{seq}', 3, '0'), '{status}', 'cust-secret-{seq}', \
             now() - interval '2 hours', {called}, 'key-{branch}-{seq}-{days_ago}' \
         FROM branches b WHERE b.id = '{branch}'"
    ))
    .await;
}

fn numbers(list: &Value) -> Vec<String> {
    list.as_array()
        .expect("a list")
        .iter()
        .map(|t| t["displayNumber"].as_str().expect("displayNumber").to_owned())
        .collect()
}

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().expect("an object").keys().cloned().collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_display_shows_todays_called_numbers_and_the_last_called_and_nothing_else() {
    let db = TestDb::start().await;
    branch(&db, "QD1", "QD1", 7).await;
    branch(&db, "QD2", "QD2", 1).await;
    ticket(&db, "QD1", 1, "COMPLETED", Some(30), 0).await;
    ticket(&db, "QD1", 2, "SERVING", Some(20), 0).await;
    ticket(&db, "QD1", 3, "CALLED", Some(10), 0).await;
    ticket(&db, "QD1", 4, "WAITING", None, 0).await;
    ticket(&db, "QD1", 5, "CANCELLED", None, 0).await;
    ticket(&db, "QD1", 9, "CALLED", Some(5), 1).await; // yesterday's: not shown
    ticket(&db, "QD2", 8, "CALLED", Some(1), 0).await; // another branch: not shown

    let ans = display(&db, XC, "QD1", None).await;
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(ans.cache_control.as_deref(), Some("no-store"));
    assert_eq!(keys(&ans.body), set(&["version", "changed", "data"]), "{}", ans.text);
    assert_eq!(ans.body["version"], json!(7));
    assert_eq!(ans.body["changed"], json!(true));
    assert_eq!(keys(&ans.body["data"]), set(&["called", "recent"]));

    let called = &ans.body["data"]["called"];
    assert_eq!(numbers(called), vec!["W003", "W002"], "newest call first");
    assert_eq!(called[0]["status"], json!("CALLED"));
    assert_eq!(called[1]["status"], json!("SERVING"));
    for item in called.as_array().unwrap() {
        assert_eq!(keys(item), set(&["displayNumber", "status", "calledAt"]), "{item}");
        chrono::DateTime::parse_from_rfc3339(item["calledAt"].as_str().expect("calledAt"))
            .expect("RFC 3339");
    }
    let recent = &ans.body["data"]["recent"];
    assert_eq!(numbers(recent), vec!["W003", "W002", "W001"]);
    for item in recent.as_array().unwrap() {
        assert_eq!(keys(item), set(&["displayNumber", "calledAt"]), "{item}");
    }
    assert!(
        !ans.text.contains("cust-secret"),
        "no customer ever reaches the public display: {}",
        ans.text
    );
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn recent_holds_the_last_five_called() {
    let db = TestDb::start().await;
    branch(&db, "QD1", "QD1", 3).await;
    for seq in 1..=7 {
        ticket(&db, "QD1", seq, "COMPLETED", Some(100 - i64::from(seq)), 0).await;
    }
    let ans = display(&db, XC, "QD1", None).await;
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(
        numbers(&ans.body["data"]["recent"]),
        vec!["W007", "W006", "W005", "W004", "W003"]
    );
    assert_eq!(numbers(&ans.body["data"]["called"]), Vec::<String>::new());
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn since_the_current_version_answers_unchanged_and_an_older_one_the_data() {
    let db = TestDb::start().await;
    branch(&db, "QD1", "QD1", 12).await;
    ticket(&db, "QD1", 1, "CALLED", Some(1), 0).await;

    let same = display(&db, XC, "QD1", Some("12")).await;
    assert_eq!(same.status, StatusCode::OK, "{}", same.text);
    assert_eq!(same.body, json!({ "version": 12, "changed": false }));

    let older = display(&db, XC, "QD1", Some("11")).await;
    assert_eq!(older.body["changed"], json!(true));
    assert_eq!(numbers(&older.body["data"]["called"]), vec!["W001"]);

    let bad = display(&db, XC, "QD1", Some("abc")).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_org_or_branch_answers_the_public_not_found() {
    let db = TestDb::start().await;
    branch(&db, "QD1", "QD1", 1).await;
    // the same body the public rates route answers for an unknown branch
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let app =
        atest::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state)))
            .await;
    let req = atest::TestRequest::get()
        .uri("/api/public/orgs/XC/branches/NOPE/rates")
        .insert_header((header::HOST, "xc.test"))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let rates_status = resp.status();
    let rates_body = atest::read_body(resp).await;

    for (org, code) in [("XC", "NOPE"), ("NOPE", "QD1")] {
        let ans = display(&db, org, code, None).await;
        assert_eq!(ans.status, rates_status, "{}", ans.text);
        assert_eq!(ans.text.as_bytes(), &rates_body[..]);
    }
    db.finish().await;
}
