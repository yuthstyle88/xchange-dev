//! Ticket #1256 (RD-1): an `ETag` and `304` on the public rates route
//! `GET /api/public/orgs/{org}/branches/{code}/rates`. Written before the implementation by the
//! ticket author (claude/709ec749) on the order of the BE lane controller claude/ad05bf8d. Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rate-display.md` §4.1 (one test per row), D4, §5 RD-1.
//! - The tag is `"p<publication id>-s<0|1>"`, or `"none"` for a branch with no publication.
//! - An `If-None-Match` equal to the current tag answers `304` with an empty body and that tag.
//! - Otherwise `200` with today's body, the tag and `Cache-Control: no-store`.
//! - An unknown organisation or branch answers today's `404`, with no tag.
//! - The tag is built from the same `PublishedRatesData` the response serialises: one read, so a
//!   tag can never name a body it was not sent with.
//!
//! Seeds: `quote_common` (the harness `app_public_reads_test.rs` uses for this route). Its
//! `publish` writes a publication straight into the tables; an AUTO line's `source_snapshot`
//! carries `fetched_at` and `max_reference_age_hours`, which `Freshness` reads (`freshness.rs`), so
//! the stale flip is driven by moving that line's `fetched_at` past the 24 h limit with no new
//! publication. `api/tests/publication_seed` has no `fetched_at` in its snapshot, so it cannot
//! drive the flip.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

mod common;
mod quote_common;

use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, test as atest};
use quote_common::{Line, TestDb};
use serde_json::Value;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http::v3::published_rates::published_rates_data;

/// The seeded organisation of every migrated database (`20261011100000_organizations.sql`).
const XC: &str = "XC";

struct Answer {
    status: StatusCode,
    etag: Option<String>,
    cache_control: Option<String>,
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

    fn tag(&self) -> String {
        self.etag
            .clone()
            .unwrap_or_else(|| panic!("an ETag on {} {}", self.status, self.text()))
    }
}

/// One GET of the rates route through the whole route table, with `If-None-Match` when given.
async fn rates(db: &TestDb, org: &str, code: &str, if_none_match: Option<&str>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let app =
        atest::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state)))
            .await;
    let mut req = atest::TestRequest::get()
        .uri(&format!("/api/public/orgs/{org}/branches/{code}/rates"))
        .insert_header((header::HOST, "xc.test"));
    if let Some(tag) = if_none_match {
        req = req.insert_header((header::IF_NONE_MATCH, tag));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let value = |name: header::HeaderName| {
        resp.headers()
            .get(name)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
    };
    let etag = value(header::ETAG);
    let cache_control = value(header::CACHE_CONTROL);
    let body = atest::read_body(resp).await.to_vec();
    Answer {
        status,
        etag,
        cache_control,
        body,
    }
}

/// The tag the design gives a publication: `"p<id>-s<0|1>"`.
fn tag_for(publication_id: &str, stale: bool) -> String {
    format!("\"p{publication_id}-s{}\"", u8::from(stale))
}

const NONE_TAG: &str = "\"none\"";

/// The tag a `200` body names: its own `publication.id` and `stale`, or `"none"`.
fn tag_named_by(body: &Value) -> String {
    match body["publication"]["id"].as_str() {
        Some(id) => tag_for(id, body["stale"].as_bool().expect("a stale flag")),
        None => NONE_TAG.to_owned(),
    }
}

/// A branch of the seeded organisation `XC`.
async fn xc_branch(db: &TestDb, id: &str, code: &str) {
    db.exec(&format!(
        "INSERT INTO branches (id, name, code) VALUES ('{id}', 'Branch {code}', '{code}')"
    ))
    .await;
}

/// One AUTO USD line under `policy`, stale or not as `publish` leaves it (fetched 30 h ago).
fn auto_usd(policy: &str) -> Line {
    Line {
        code: "USD",
        unit_size: "1",
        buy: Some("35.10"),
        sell: Some("35.50"),
        stale_under: Some(policy.to_owned()),
    }
}

/// Moves the `fetched_at` (and `source_date`) of every line of `publication` to `hours_ago`.
async fn fetched_hours_ago(db: &TestDb, publication: &str, hours_ago: i64) {
    db.exec(&format!(
        "UPDATE published_rate_lines SET source_snapshot = source_snapshot \
             || jsonb_build_object( \
                 'fetched_at', to_char((now() - interval '{hours_ago} hours') AT TIME ZONE 'UTC', \
                     'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                 'source_date', to_char((now() - interval '{hours_ago} hours') AT TIME ZONE 'UTC', \
                     'YYYY-MM-DD')) \
         WHERE publication_id = '{publication}'"
    ))
    .await;
}

// §4.1 row 1
#[tokio::test]
async fn a_get_without_if_none_match_answers_200_the_same_body_a_publication_tag_and_no_store() {
    let db = TestDb::start().await;
    let published = db
        .publish(
            "MAIN",
            &[
                Line::manual("USD", "1", "35.10", "35.50"),
                Line::manual("JPY", "100", "23.40", "24.10"),
            ],
        )
        .await;

    let answer = rates(&db, XC, "MAIN", None).await;

    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    assert_eq!(
        answer.etag.as_deref(),
        Some(tag_for(&published.id, false).as_str()),
        "the tag is \"p<publication id>-s0\" for a fresh publication"
    );
    assert_eq!(
        answer.cache_control.as_deref(),
        Some("no-store"),
        "no-store stays"
    );
    let shared = published_rates_data(&db.pool, &BranchId::new("MAIN"))
        .await
        .expect("the shared reader");
    assert_eq!(
        answer.json(),
        serde_json::to_value(&shared).expect("serialise"),
        "the body is exactly today's"
    );
    db.finish().await;
}

// §4.1 row 2
#[tokio::test]
async fn a_repeat_with_that_tag_answers_304_with_no_body_and_the_same_tag() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[Line::manual("USD", "1", "35.10", "35.50")])
        .await;
    let first = rates(&db, XC, "MAIN", None).await;
    let tag = first.tag();

    let again = rates(&db, XC, "MAIN", Some(&tag)).await;

    assert_eq!(
        again.status,
        StatusCode::NOT_MODIFIED,
        "nothing changed: {}",
        again.text()
    );
    assert!(
        again.body.is_empty(),
        "a 304 has no body, got {} bytes: {}",
        again.body.len(),
        again.text()
    );
    assert_eq!(again.etag.as_deref(), Some(tag.as_str()), "the same tag");
    db.finish().await;
}

// §4.1 row 3
#[tokio::test]
async fn a_new_publication_answers_200_with_the_new_body_and_a_new_tag() {
    let db = TestDb::start().await;
    let old = db
        .publish("MAIN", &[Line::manual("USD", "1", "35.10", "35.50")])
        .await;
    let old_tag = rates(&db, XC, "MAIN", None).await.tag();
    assert_eq!(old_tag, tag_for(&old.id, false));

    let new = db
        .publish("MAIN", &[Line::manual("USD", "1", "35.20", "35.60")])
        .await;
    let answer = rates(&db, XC, "MAIN", Some(&old_tag)).await;

    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    let body = answer.json();
    assert_eq!(body["publication"]["id"], new.id.as_str());
    assert_eq!(body["lines"][0]["buyRate"], "35.20000000");
    assert_eq!(
        answer.etag.as_deref(),
        Some(tag_for(&new.id, false).as_str())
    );
    assert_ne!(answer.tag(), old_tag);
    db.finish().await;
}

// §4.1 row 4
#[tokio::test]
async fn the_reference_ageing_past_its_limit_flips_the_tag_to_s1_with_no_new_publication() {
    let db = TestDb::start().await;
    let policy = db.policy("MAIN", None, None).await;
    let published = db.publish("MAIN", &[auto_usd(&policy)]).await;
    fetched_hours_ago(&db, &published.id, 1).await;

    let fresh = rates(&db, XC, "MAIN", None).await;
    assert_eq!(fresh.status, StatusCode::OK, "{}", fresh.text());
    assert_eq!(fresh.json()["stale"], false, "fetched 1 h ago under 24 h");
    assert_eq!(
        fresh.etag.as_deref(),
        Some(tag_for(&published.id, false).as_str())
    );

    // the same publication; only time has passed for its reference
    fetched_hours_ago(&db, &published.id, 30).await;
    let aged = rates(&db, XC, "MAIN", Some(&fresh.tag())).await;

    assert_eq!(
        aged.status,
        StatusCode::OK,
        "the -s0 tag no longer matches: {}",
        aged.text()
    );
    let body = aged.json();
    assert_eq!(body["stale"], true, "fetched 30 h ago under 24 h: {body}");
    assert_eq!(body["publication"]["id"], published.id.as_str());
    assert_eq!(
        aged.etag.as_deref(),
        Some(tag_for(&published.id, true).as_str()),
        "the tag ends -s1"
    );

    let repeat = rates(&db, XC, "MAIN", Some(&aged.tag())).await;
    assert_eq!(repeat.status, StatusCode::NOT_MODIFIED, "{}", repeat.text());
    db.finish().await;
}

// §4.1 row 5
#[tokio::test]
async fn a_branch_with_no_publication_answers_the_none_tag_and_then_304() {
    let db = TestDb::start().await;
    xc_branch(&db, "b-empty", "EMPTY").await;

    let first = rates(&db, XC, "EMPTY", None).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(
        first.json(),
        serde_json::json!({ "publication": null, "sourceDate": null, "stale": false, "lines": [] })
    );
    assert_eq!(first.etag.as_deref(), Some(NONE_TAG));
    assert_eq!(first.cache_control.as_deref(), Some("no-store"));

    let again = rates(&db, XC, "EMPTY", Some(NONE_TAG)).await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED, "{}", again.text());
    assert!(again.body.is_empty(), "{}", again.text());
    assert_eq!(again.etag.as_deref(), Some(NONE_TAG));

    // the first publication ends the "none" tag
    let published = db
        .publish("b-empty", &[Line::manual("USD", "1", "35.10", "35.50")])
        .await;
    let after = rates(&db, XC, "EMPTY", Some(NONE_TAG)).await;
    assert_eq!(after.status, StatusCode::OK, "{}", after.text());
    assert_eq!(
        after.etag.as_deref(),
        Some(tag_for(&published.id, false).as_str())
    );
    db.finish().await;
}

// §4.1 row 6
#[tokio::test]
async fn an_unknown_organisation_or_branch_answers_todays_404_with_no_tag() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[Line::manual("USD", "1", "35.10", "35.50")])
        .await;
    let main_tag = rates(&db, XC, "MAIN", None).await.tag();

    let cases = [
        (
            "an unknown organisation",
            rates(&db, "NOPE", "MAIN", None).await,
        ),
        ("an unknown branch", rates(&db, XC, "NOPE", None).await),
        (
            "an unknown branch with a valid tag",
            rates(&db, XC, "NOPE", Some(&main_tag)).await,
        ),
        (
            "an unknown branch with the none tag",
            rates(&db, XC, "NOPE", Some(NONE_TAG)).await,
        ),
    ];
    let first = cases[0].1.body.clone();
    for (what, answer) in &cases {
        assert_eq!(
            answer.status,
            StatusCode::NOT_FOUND,
            "{what}: {}",
            answer.text()
        );
        assert_eq!(answer.etag, None, "{what}: a 404 has no tag");
        assert_eq!(answer.json()["error"]["code"], "notFound", "{what}");
        assert_eq!(answer.body, first, "{what}: today's one 404 body");
    }
    db.finish().await;
}

// §4.1 row 7
#[tokio::test]
async fn another_branchs_tag_answers_200() {
    let db = TestDb::start().await;
    xc_branch(&db, "b-north", "NORTH").await;
    let main = db
        .publish("MAIN", &[Line::manual("USD", "1", "35.10", "35.50")])
        .await;
    let north = db
        .publish("b-north", &[Line::manual("USD", "1", "35.30", "35.70")])
        .await;
    let main_tag = rates(&db, XC, "MAIN", None).await.tag();
    assert_eq!(main_tag, tag_for(&main.id, false));

    let answer = rates(&db, XC, "NORTH", Some(&main_tag)).await;

    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    assert_eq!(answer.json()["publication"]["id"], north.id.as_str());
    assert_eq!(
        answer.etag.as_deref(),
        Some(tag_for(&north.id, false).as_str())
    );
    db.finish().await;
}

// §4.1 row 8: the tag names the body it is sent with
#[tokio::test]
async fn every_200_tag_names_the_publication_and_stale_flag_of_its_own_body() {
    let db = TestDb::start().await;
    xc_branch(&db, "b-empty", "EMPTY").await;
    let policy = db.policy("MAIN", None, None).await;
    db.publish("MAIN", &[Line::manual("USD", "1", "35.10", "35.50")])
        .await;
    let stale = db.publish("MAIN", &[auto_usd(&policy)]).await;

    let mut seen = Vec::new();
    for (org, code) in [(XC, "MAIN"), (XC, "EMPTY")] {
        let answer = rates(&db, org, code, None).await;
        assert_eq!(answer.status, StatusCode::OK, "{code}: {}", answer.text());
        let body = answer.json();
        assert_eq!(answer.tag(), tag_named_by(&body), "{code}: {body}");
        seen.push(answer.tag());
    }
    assert_eq!(
        seen,
        vec![tag_for(&stale.id, true), NONE_TAG.to_owned()],
        "the stale publication and the empty branch"
    );
    db.finish().await;
}

// §4.1 row 8: the branch's current publication, not its newest version
#[tokio::test]
async fn the_tag_follows_the_publication_the_body_reads_not_the_newest_version() {
    let db = TestDb::start().await;
    let first = db
        .publish("MAIN", &[Line::manual("USD", "1", "35.10", "35.50")])
        .await;
    let second = db
        .publish("MAIN", &[Line::manual("USD", "1", "35.20", "35.60")])
        .await;
    // the branch points back at the first publication; the second stays the newest version
    db.exec(&format!(
        "UPDATE branches SET current_publication_id = '{}' WHERE id = 'MAIN'",
        first.id
    ))
    .await;

    let answer = rates(&db, XC, "MAIN", None).await;

    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    let body = answer.json();
    assert_eq!(body["publication"]["id"], first.id.as_str());
    assert_eq!(answer.tag(), tag_named_by(&body));
    assert_ne!(answer.tag(), tag_for(&second.id, false));

    let repeat = rates(&db, XC, "MAIN", Some(&tag_for(&second.id, false))).await;
    assert_eq!(
        repeat.status,
        StatusCode::OK,
        "the newest version's tag is not this body's: {}",
        repeat.text()
    );
    db.finish().await;
}
