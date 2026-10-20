//! S6 of the FE survey section B (#3409; design `docs/design/blueprint/section-b-gaps.md` §S6, PR 3387;
//! `customer-identity-consent.md:163`: the consent texts are a public read for sign-up).
//!
//! 🔴 Written by the ticket writer claude/studio-claude-4 before the implementation. Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! `GET /api/public/orgs/{org}/consent_texts?lang=th|en|zh`, no session, answers the newest
//! published version of each purpose in that language as `[{purpose, version, body}]`, with an
//! `ETag`; an unchanged `If-None-Match` answers 304. Unknown org ⇒ 404; a bad `lang` ⇒ 400.
//!
//! Harness: `quote_common` (as `public_rates_etag_test.rs`). Needs `TEST_DATABASE_URL` naming a
//! database whose name ends in `_test`.

mod common;
mod quote_common;

use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, test as atest};
use quote_common::TestDb;
use serde_json::{Value, json};
use xchange::bootstrap::{self, AppState};

/// The seeded organisation of every migrated database (`20261011100000_organizations.sql`).
const XC: &str = "XC";
const XC_ID: &str = "00000000-0000-0000-0000-000000000001";

struct Answer {
    status: StatusCode,
    etag: Option<String>,
    body: Vec<u8>,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!("a JSON body ({e}): {}", String::from_utf8_lossy(&self.body))
        })
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// One GET of the route through the whole route table, with no session.
async fn texts(db: &TestDb, org: &str, query: &str, if_none_match: Option<&str>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let app =
        atest::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state)))
            .await;
    let mut req = atest::TestRequest::get()
        .uri(&format!("/api/public/orgs/{org}/consent_texts{query}"))
        .insert_header((header::HOST, "xc.test"));
    if let Some(tag) = if_none_match {
        req = req.insert_header((header::IF_NONE_MATCH, tag));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let etag = resp
        .headers()
        .get(header::ETAG)
        .map(|v| v.to_str().expect("an ASCII header").to_owned());
    let body = atest::read_body(resp).await.to_vec();
    Answer { status, etag, body }
}

async fn publish(db: &TestDb, org_id: &str, purpose: &str, version: i32, lang: &str, body: &str) {
    db.exec(&format!(
        "INSERT INTO consent_texts (organization_id, purpose, version, lang, body, published_at) \
         VALUES ('{org_id}', '{purpose}', {version}, '{lang}', '{body}', now())"
    ))
    .await;
}

/// The answer as a sorted list of `purpose v<version>: <body>`.
fn listed(a: &Answer) -> Vec<String> {
    let mut out: Vec<String> = a
        .json()
        .as_array()
        .unwrap_or_else(|| panic!("an array: {}", a.text()))
        .iter()
        .map(|t| {
            format!(
                "{} v{}: {}",
                t["purpose"].as_str().unwrap_or("?"),
                t["version"],
                t["body"].as_str().unwrap_or("?")
            )
        })
        .collect();
    out.sort();
    out
}

/// B1 + B2: no session at all; only the newest version of each purpose in that language.
#[tokio::test]
async fn s6_a_visitor_reads_the_newest_terms_and_marketing_texts_of_a_language() {
    let db = TestDb::start().await;
    publish(&db, XC_ID, "TERMS", 1, "th", "terms one").await;
    publish(&db, XC_ID, "TERMS", 2, "th", "terms two").await;
    publish(&db, XC_ID, "MARKETING", 1, "th", "marketing one").await;
    publish(&db, XC_ID, "TERMS", 3, "en", "terms three en").await;

    let a = texts(&db, XC, "?lang=th", None).await;

    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    assert_eq!(
        listed(&a),
        vec!["MARKETING v1: marketing one", "TERMS v2: terms two"],
        "the newest per purpose, Thai only"
    );
    db.finish().await;
}

/// A language with no text answers an empty list, not an error.
#[tokio::test]
async fn s6_a_language_with_no_text_answers_an_empty_list() {
    let db = TestDb::start().await;
    publish(&db, XC_ID, "TERMS", 1, "th", "terms one").await;

    let a = texts(&db, XC, "?lang=zh", None).await;

    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    assert_eq!(a.json(), json!([]));
    db.finish().await;
}

/// A bad or missing language ⇒ 400; an unknown organisation ⇒ 404.
#[tokio::test]
async fn s6_a_bad_language_is_400_and_an_unknown_organisation_is_404() {
    let db = TestDb::start().await;
    for query in ["?lang=fr", ""] {
        let a = texts(&db, XC, query, None).await;
        assert_eq!(a.status, StatusCode::BAD_REQUEST, "{query:?}: {}", a.text());
    }
    let a = texts(&db, "NOPE", "?lang=th", None).await;
    assert_eq!(a.status, StatusCode::NOT_FOUND, "{}", a.text());
    db.finish().await;
}

/// B3: another organisation's texts are never in the answer.
#[tokio::test]
async fn s6_another_organisations_texts_are_never_answered() {
    let db = TestDb::start().await;
    db.second_organisation("00000000-0000-0000-0000-0000000000b2", "B2", "B2-MAIN")
        .await;
    publish(
        &db,
        "00000000-0000-0000-0000-0000000000b2",
        "TERMS",
        9,
        "th",
        "other org",
    )
    .await;

    let a = texts(&db, XC, "?lang=th", None).await;

    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    assert_eq!(a.json(), json!([]), "XC has no text; B2's is not XC's");
    db.finish().await;
}

/// B4: an unchanged `If-None-Match` ⇒ 304; a new version changes the tag.
#[tokio::test]
async fn s6_an_unchanged_tag_answers_304_and_a_new_version_changes_it() {
    let db = TestDb::start().await;
    publish(&db, XC_ID, "TERMS", 1, "th", "terms one").await;

    let first = texts(&db, XC, "?lang=th", None).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    let tag = first.etag.clone().expect("an ETag on 200");

    let again = texts(&db, XC, "?lang=th", Some(&tag)).await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED, "{}", again.text());

    publish(&db, XC_ID, "TERMS", 2, "th", "terms two").await;
    let after = texts(&db, XC, "?lang=th", Some(&tag)).await;
    assert_eq!(after.status, StatusCode::OK, "{}", after.text());
    assert_ne!(
        after.etag.as_deref(),
        Some(tag.as_str()),
        "a new version, a new tag"
    );
    db.finish().await;
}
