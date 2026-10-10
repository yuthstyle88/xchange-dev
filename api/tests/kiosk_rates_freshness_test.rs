//! RF-5 (#877, design `docs/design/blueprint/rates.md` §4.10): the kiosk JSON `rates` carries the branch's current
//! publication's freshness, `freshness: {stale, sourceDate, publishedAt}`, computed by RF-1's one rule, appended after
//! every v2 key so v2's kept kiosk page is unaffected.
//!
//! Written before the implementation by the ticket author (claude/dell-claude-4). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! `Counter::open` publishes the dollar as a MANUAL line with no reference. The cases here then write the reference
//! keys the publisher writes (`fetched_at`, `max_reference_age_hours`, `source_date`) into that line's
//! `source_snapshot`, so they do not depend on the reference fetcher. The clock is the wall clock: references are 30 h
//! or 1 h old against a 24 h limit.
//!
//! Needs `TEST_DATABASE_URL` naming a database ending `_test`.

mod common;
mod compat_common;
mod kiosk_http_common;
mod rpc_common;
mod snapshot_common;

use chrono::{DateTime, Duration, Utc};
use kiosk_http_common::Counter;
use serde_json::{Value, json};
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Side};
use xc_rate::domain::publication::published_rates::PublishedRates;
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;

const MAIN: &str = "MAIN";

/// The kiosk's `rates` answer: the parsed JSON and the raw text.
async fn rates(counter: &Counter) -> (Value, String) {
    let answer = counter.ask(None, "rates", json!({})).await;
    assert_eq!(answer.status, 200, "rates: {}", answer.text);
    (answer.json(), answer.text.clone())
}

/// The `freshness` object of the `rates` answer; a missing key fails here, by name.
async fn freshness(counter: &Counter) -> Value {
    let (json, text) = rates(counter).await;
    json.get("freshness")
        .cloned()
        .unwrap_or_else(|| panic!("the kiosk `rates` answer has no `freshness`: {text}"))
}

/// Merges reference keys into every line of MAIN's current publication.
async fn give_every_line(pool: &PgPool, fetched_hours_ago: i64, max_hours: i64, source_date: &str) {
    let keys = json!({
        "fetched_at": (Utc::now() - Duration::hours(fetched_hours_ago)).to_rfc3339(),
        "max_reference_age_hours": max_hours,
        "source_date": source_date,
    });
    let changed = sqlx::query(
        "UPDATE published_rate_lines SET source_snapshot = source_snapshot || $1 \
         WHERE publication_id = (SELECT current_publication_id FROM branches WHERE id = $2)",
    )
    .bind(&keys)
    .bind(MAIN)
    .execute(pool)
    .await
    .expect("write the reference keys");
    assert!(
        changed.rows_affected() > 0,
        "MAIN has a current publication with lines"
    );
}

/// Adds one more line, `currency`, to MAIN's current publication, with its own reference.
async fn add_line(
    pool: &PgPool,
    currency: &str,
    fetched_hours_ago: i64,
    max_hours: i64,
    source_date: &str,
) {
    let snapshot = json!({
        "buy_mode": "MANUAL",
        "sell_mode": "MANUAL",
        "fetched_at": (Utc::now() - Duration::hours(fetched_hours_ago)).to_rfc3339(),
        "max_reference_age_hours": max_hours,
        "source_date": source_date,
    });
    sqlx::query(
        "INSERT INTO published_rate_lines (id, publication_id, currency_code, unit_size, buy_rate, sell_rate, \
         source_snapshot) \
         SELECT gen_random_uuid(), current_publication_id, $1, 1, 37.10000000, 37.90000000, $2 \
         FROM branches WHERE id = $3",
    )
    .bind(currency)
    .bind(&snapshot)
    .bind(MAIN)
    .execute(pool)
    .await
    .expect("add a line");
}

/// MAIN's current publication's `published_at`.
async fn published_at(pool: &PgPool) -> DateTime<Utc> {
    sqlx::query_scalar(
        "SELECT p.published_at FROM branches b JOIN rate_publications p ON p.id = b.current_publication_id \
         WHERE b.id = $1",
    )
    .bind(MAIN)
    .fetch_one(pool)
    .await
    .expect("MAIN has a current publication")
}

/// `publishedAt` must be RFC 3339 naming the publication's instant.
fn assert_published_at(fresh: &Value, expected: DateTime<Utc>) {
    let text = fresh["publishedAt"]
        .as_str()
        .unwrap_or_else(|| panic!("publishedAt is a text: {fresh}"));
    let at = DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("publishedAt {text:?} is RFC 3339: {e}"))
        .with_timezone(&Utc);
    assert_eq!(
        at, expected,
        "publishedAt is the publication's published_at"
    );
}

/// The quote path's answer: stale on either side of `currency`.
async fn quote_path_stale(pool: &PgPool, currency: &str) -> bool {
    let reader = PgPublishedRates::new(pool.clone());
    let mut stale = false;
    for side in [Side::Buy, Side::Sell] {
        let mut tx = pool.begin().await.expect("a transaction");
        let got = reader
            .line_in(
                &mut tx,
                &BranchId::new(MAIN),
                CurrencyCode::parse(currency).unwrap(),
                side,
            )
            .await
            .expect("line_in")
            .unwrap_or_else(|| panic!("{currency} {side:?} is published"));
        tx.rollback().await.expect("roll back");
        stale |= got.stale;
    }
    stale
}

/// v2's keys keep their order and `freshness` comes after the last of them, so the kept v2 kiosk page reads the
/// answer exactly as before.
#[actix_web::test]
async fn rates_appends_freshness_after_every_v2_key() {
    let counter = Counter::open(&[]).await;

    let (json, text) = rates(&counter).await;
    let order = [
        "branchId",
        "branchName",
        "open",
        "rates",
        "paymentMethods",
        "simulation",
        "freshness",
    ];
    let mut last = 0;
    for key in order {
        let at = text
            .find(&format!("\"{key}\":"))
            .unwrap_or_else(|| panic!("the answer has `{key}`: {text}"));
        assert!(at >= last, "`{key}` comes after the keys before it: {text}");
        last = at;
    }
    let keys: Vec<&String> = json.as_object().expect("an object").keys().collect();
    assert_eq!(keys.len(), order.len(), "no other top-level key: {text}");
    let fresh = &json["freshness"];
    let mut fields: Vec<&String> = fresh
        .as_object()
        .expect("freshness is an object")
        .keys()
        .collect();
    fields.sort();
    assert_eq!(fields, ["publishedAt", "sourceDate", "stale"], "{fresh}");

    counter.finish().await;
}

/// `Counter::open`'s publication has no reference: fresh, no reference date, and its publish time.
#[actix_web::test]
async fn a_publication_without_a_reference_is_fresh_with_no_source_date() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();

    let fresh = freshness(&counter).await;
    assert_eq!(fresh["stale"], json!(false), "{fresh}");
    assert_eq!(fresh["sourceDate"], Value::Null, "{fresh}");
    assert_published_at(&fresh, published_at(&pool).await);

    counter.finish().await;
}

/// RF-1's rule: a reference past its age is stale, on a MANUAL line too.
#[actix_web::test]
async fn a_reference_past_its_age_makes_the_kiosk_answer_stale() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    give_every_line(&pool, 30, 24, "2026-10-06").await;

    let fresh = freshness(&counter).await;
    assert_eq!(fresh["stale"], json!(true), "{fresh}");
    assert_eq!(fresh["sourceDate"], json!("2026-10-06"), "{fresh}");
    assert_published_at(&fresh, published_at(&pool).await);

    counter.finish().await;
}

/// A reference within its age is fresh, and its date is shown.
#[actix_web::test]
async fn a_reference_within_its_age_is_fresh_with_its_date() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    give_every_line(&pool, 1, 24, "2026-10-09").await;

    let fresh = freshness(&counter).await;
    assert_eq!(fresh["stale"], json!(false), "{fresh}");
    assert_eq!(fresh["sourceDate"], json!("2026-10-09"), "{fresh}");

    counter.finish().await;
}

/// The freshness is the publication's (`Freshness::of_publication`): any stale line makes it stale, and the reference
/// date is the oldest one, the same answer the admin and staff screens give.
#[actix_web::test]
async fn the_kiosk_freshness_is_the_publications() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    give_every_line(&pool, 1, 24, "2026-10-09").await;
    add_line(&pool, "EUR", 30, 24, "2026-10-06").await;

    let fresh = freshness(&counter).await;
    assert_eq!(
        fresh["stale"],
        json!(true),
        "the old EUR reference makes it stale: {fresh}"
    );
    assert_eq!(
        fresh["sourceDate"],
        json!("2026-10-06"),
        "the oldest reference date: {fresh}"
    );

    counter.finish().await;
}

/// The customer's screen and the quote path agree: what refuses or caps a quote is what the badge shows.
#[actix_web::test]
async fn the_kiosk_and_the_quote_path_agree_on_stale() {
    for (hours, label) in [(30, "an old reference"), (1, "a fresh reference")] {
        let counter = Counter::open(&[]).await;
        let pool = counter.desk.db.pool.clone();
        give_every_line(&pool, hours, 24, "2026-10-08").await;

        let fresh = freshness(&counter).await;
        let quote = quote_path_stale(&pool, "USD").await;
        assert_eq!(
            fresh["stale"],
            json!(quote),
            "{label}: the quote path says stale = {quote}, the kiosk answer says {fresh}"
        );

        counter.finish().await;
    }
}

/// No current publication: nothing to be stale, no date, no publish time.
#[actix_web::test]
async fn a_branch_with_no_publication_answers_an_empty_freshness() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    sqlx::query("UPDATE branches SET current_publication_id = NULL WHERE id = $1")
        .bind(MAIN)
        .execute(&pool)
        .await
        .expect("take the publication away");

    let fresh = freshness(&counter).await;
    assert_eq!(
        fresh,
        json!({ "stale": false, "sourceDate": null, "publishedAt": null }),
        "{fresh}"
    );

    counter.finish().await;
}

/// The kiosk prints RF-1's one shape and never computes staleness itself.
#[test]
fn the_kiosk_answer_reads_freshness_through_the_one_rule() {
    let root = env!("CARGO_MANIFEST_DIR");
    let dto = std::fs::read_to_string(format!("{root}/src/presentation/dto/kiosk.rs"))
        .expect("dto/kiosk.rs");
    let handler = std::fs::read_to_string(format!("{root}/src/presentation/http/kiosk.rs"))
        .expect("http/kiosk.rs");
    assert!(
        dto.contains("RateFreshnessDto"),
        "dto/kiosk.rs prints the kiosk's freshness as RF-1's RateFreshnessDto"
    );
    for (name, source) in [("dto/kiosk.rs", &dto), ("http/kiosk.rs", &handler)] {
        for key in ["max_reference_age_hours", "fetched_at"] {
            assert!(
                !source.contains(key),
                "{name} reads `{key}`: staleness is computed only by xc-rate's Freshness"
            );
        }
    }
}
