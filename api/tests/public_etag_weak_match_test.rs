//! Written before the implementation by the sub-controller claude/studio-claude-16 (controller
//! claude/553782). Do not edit, delete or weaken.
//!
//! Ticket #5422: the public ETag routes compare `If-None-Match` weakly (RFC 7232 §3.2), so a
//! poller behind a proxy that gzips (nginx turns the strong tag into `W/"…"`) is answered `304`.
//!
//! What this pins, for each of `GET /api/public/orgs/{org}/branches/{code}/rates`,
//! `GET /api/public/orgs/{org}/branches` and `GET /api/public/orgs/{org}/consent_texts?lang=th`
//! (#5422 §4):
//! - E1: the exact strong tag ⇒ 304;
//! - E2: `W/` + the tag ⇒ 304;
//! - E3: a list holding the tag (`"x", W/"<tag>"`) ⇒ 304;
//! - E4: `*` ⇒ 304;
//! - E5: another tag ⇒ 200 with the body.
//!
//! Every case goes through the whole route table, as `app_public_reads_test.rs` does; the test
//! names no helper the implementation adds. Needs a real PostgreSQL (`TEST_DATABASE_URL`).

mod common;
mod quote_common;

use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, test as atest};
use quote_common::{Line, TestDb};
use xchange::bootstrap::{self, AppState};

/// The seeded organisation of every migrated database.
const XC: &str = "XC";

struct Answer {
    status: StatusCode,
    etag: Option<String>,
    body: Vec<u8>,
}

/// One GET through the whole route table, with `If-None-Match` when given.
async fn get(db: &TestDb, uri: &str, if_none_match: Option<&str>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let app =
        atest::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state)))
            .await;
    let mut req = atest::TestRequest::get()
        .uri(uri)
        .insert_header((header::HOST, "xc.test"));
    if let Some(tag) = if_none_match {
        req = req.insert_header((header::IF_NONE_MATCH, tag));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let etag = resp
        .headers()
        .get(header::ETAG)
        .map(|v| v.to_str().expect("an ASCII ETag").to_owned());
    let body = atest::read_body(resp).await.to_vec();
    Answer { status, etag, body }
}

/// E1–E5 on `uri`; panics with every case that answered wrongly.
async fn weak_match_on(db: &TestDb, uri: &str) {
    let first = get(db, uri, None).await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "{uri}: {}",
        String::from_utf8_lossy(&first.body)
    );
    let tag = first
        .etag
        .clone()
        .unwrap_or_else(|| panic!("{uri}: the first answer has an ETag"));
    assert!(
        tag.starts_with('"') && tag.ends_with('"'),
        "{uri}: the app's own tag is strong and quoted, got {tag}"
    );

    let weak = format!("W/{tag}");
    let list = format!("\"x\", W/{tag}");
    let cases: [(&str, String, StatusCode); 5] = [
        ("E1 exact strong tag", tag.clone(), StatusCode::NOT_MODIFIED),
        ("E2 W/ + the tag", weak, StatusCode::NOT_MODIFIED),
        ("E3 a list holding the tag", list, StatusCode::NOT_MODIFIED),
        ("E4 *", "*".to_owned(), StatusCode::NOT_MODIFIED),
        (
            "E5 another tag",
            "\"not-this-one\"".to_owned(),
            StatusCode::OK,
        ),
    ];

    let mut wrong = Vec::new();
    for (case, sent, want) in cases {
        let answer = get(db, uri, Some(&sent)).await;
        if answer.status != want {
            wrong.push(format!(
                "{case}: If-None-Match {sent} answered {}, want {want}",
                answer.status
            ));
            continue;
        }
        if want == StatusCode::OK && answer.body.is_empty() {
            wrong.push(format!("{case}: 200 with an empty body"));
        }
        if answer.etag.as_deref() != Some(tag.as_str()) {
            wrong.push(format!(
                "{case}: answered ETag {:?}, want {tag}",
                answer.etag
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{uri} (#5422, RFC 7232 §3.2 weak comparison):\n{}",
        wrong.join("\n")
    );
}

#[tokio::test]
async fn the_rates_route_answers_304_to_a_weak_or_listed_tag() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[Line::manual("USD", "1", "35.10", "35.50")])
        .await;
    weak_match_on(&db, &format!("/api/public/orgs/{XC}/branches/MAIN/rates")).await;
    db.finish().await;
}

#[tokio::test]
async fn the_branches_route_answers_304_to_a_weak_or_listed_tag() {
    let db = TestDb::start().await;
    weak_match_on(&db, &format!("/api/public/orgs/{XC}/branches")).await;
    db.finish().await;
}

#[tokio::test]
async fn the_consent_texts_route_answers_304_to_a_weak_or_listed_tag() {
    let db = TestDb::start().await;
    weak_match_on(&db, &format!("/api/public/orgs/{XC}/consent_texts?lang=th")).await;
    db.finish().await;
}
