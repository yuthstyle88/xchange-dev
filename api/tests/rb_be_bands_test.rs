//! RB-BE (rate board v2, amount bands): the public rates route
//! `GET /api/public/orgs/{org}/branches/{code}/rates` adds `bandHeadings` and `line.bands`.
//! Written before the implementation by the ticket author (claude/dell-claude-3) on the order of
//! the BE lane controller claude/a90ab16c. Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! Design: `docs/design/blueprint/rate-display.md` §9.3 (merged in PR 3558), with the FE lane
//! controller's ruling on PR 3558 (issuecomment-6267993195):
//! - `bandHeadings` comes from **the publication's policy version** (`rate_policy_value_tiers`),
//!   never from the lines;
//! - lines are bucketed by that version's tiers (`value_tier` `T<n>` under heading `T<n>`);
//! - lines that disagree with the headings never make the route answer `500`.
//!
//! The publication's policy version is the first non-null `source_snapshot->>'policy_version_id'`
//! of its lines (the draft publish writes it on every line, `publish_service.rs:431`).
//!
//! Every read goes over HTTP through the whole route table (or through `published_rates_data`, the
//! loader the route and the v3 view share), and every check reads JSON, so on `main` these fail on
//! a missing key, not on a compile error. Seeds: `quote_common`, plus raw rows for the tiers.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

mod common;
mod quote_common;

use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, test as atest};
use quote_common::{Line, TestDb};
use serde_json::{Value, json};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http::v3::published_rates::published_rates_data;

/// The seeded organisation of every migrated database (`20261011100000_organizations.sql`).
const XC: &str = "XC";

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
}

/// One GET of the public rates route of `MAIN` through the whole route table.
async fn rates(db: &TestDb) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let app =
        atest::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state)))
            .await;
    let req = atest::TestRequest::get()
        .uri(&format!("/api/public/orgs/{XC}/branches/MAIN/rates"))
        .insert_header((header::HOST, "xc.test"))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let etag = resp
        .headers()
        .get(header::ETAG)
        .map(|v| v.to_str().expect("an ASCII header").to_owned());
    let body = atest::read_body(resp).await.to_vec();
    Answer { status, etag, body }
}

/// Turns value tiers on for `policy` with the tier tops `uppers` (tier 4's is `None`, open above).
async fn tiers_on(db: &TestDb, policy: &str, uppers: [Option<&str>; 4]) {
    db.exec(&format!(
        "UPDATE rate_policy_versions SET value_tiers_on = true WHERE id = '{policy}'"
    ))
    .await;
    for (i, upper) in uppers.iter().enumerate() {
        let upper = upper.map_or("NULL".to_owned(), |u| u.to_owned());
        db.exec(&format!(
            "INSERT INTO rate_policy_value_tiers \
                 (policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct) \
             VALUES ('{policy}', {no}, {upper}, 0, 0)",
            no = i + 1
        ))
        .await;
    }
}

/// One published row of `code` in tier `tier` with its own stored bounds, under `policy`.
/// `lower = None` makes it the line's first-tier row (the row `lines` already shows).
#[allow(clippy::too_many_arguments)]
async fn tier_row(
    db: &TestDb,
    publication: &str,
    code: &str,
    unit: &str,
    tier: &str,
    lower: Option<&str>,
    upper: Option<&str>,
    buy: &str,
    sell: &str,
    policy: Option<&str>,
) {
    let snapshot = match policy {
        Some(p) => json!({ "currency": code, "buy_mode": "MANUAL", "sell_mode": "MANUAL",
                           "reason": "test", "policy_version_id": p, "value_tier": tier }),
        None => json!({ "currency": code, "buy_mode": "MANUAL", "sell_mode": "MANUAL",
                        "reason": "test", "value_tier": tier }),
    };
    let lower = lower.map_or("NULL".to_owned(), |v| v.to_owned());
    let upper = upper.map_or("NULL".to_owned(), |v| v.to_owned());
    db.exec(&format!(
        "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
             denomination_group, unit_size, buy_rate, sell_rate, source_snapshot, value_tier, \
             tier_lower_exclusive_thb, tier_upper_inclusive_thb) \
         SELECT gen_random_uuid(), organization_id, '{publication}', '{code}', 'ALL', {unit}, {buy}, \
             {sell}, '{snapshot}'::jsonb, '{tier}', {lower}, {upper} \
         FROM rate_publications WHERE id = '{publication}'"
    ))
    .await;
}

/// The four tier tops of the owner's board (V3): ≤ 10,000 / ≤ 50,000 / ≤ 200,000 / above.
const TOPS: [Option<&str>; 4] = [Some("10000"), Some("50000"), Some("200000"), None];

/// USD in four tiers whose stored bounds match `TOPS`, under `policy`.
async fn usd_four_tiers(db: &TestDb, publication: &str, policy: &str) {
    for (tier, lo, hi, buy, sell) in [
        ("T1", None, Some("10000"), "35.10", "35.50"),
        ("T2", Some("10000"), Some("50000"), "35.15", "35.45"),
        ("T3", Some("50000"), Some("200000"), "35.20", "35.40"),
        ("T4", Some("200000"), None, "35.25", "35.35"),
    ] {
        tier_row(
            db,
            publication,
            "USD",
            "1",
            tier,
            lo,
            hi,
            buy,
            sell,
            Some(policy),
        )
        .await;
    }
}

fn line<'a>(body: &'a Value, code: &str) -> &'a Value {
    body["lines"]
        .as_array()
        .unwrap_or_else(|| panic!("lines is an array: {body}"))
        .iter()
        .find(|l| l["currency"] == code)
        .unwrap_or_else(|| panic!("a {code} line: {body}"))
}

fn tiers_of(bands: &Value) -> Vec<String> {
    bands
        .as_array()
        .unwrap_or_else(|| panic!("bands is an array: {bands}"))
        .iter()
        .map(|b| b["tier"].as_str().expect("a tier").to_owned())
        .collect()
}

fn headings_of_tops() -> Value {
    json!([
        { "tier": "T1", "lowerExclusiveThb": null,       "upperInclusiveThb": "10000.00" },
        { "tier": "T2", "lowerExclusiveThb": "10000.00",  "upperInclusiveThb": "50000.00" },
        { "tier": "T3", "lowerExclusiveThb": "50000.00",  "upperInclusiveThb": "200000.00" },
        { "tier": "T4", "lowerExclusiveThb": "200000.00", "upperInclusiveThb": null },
    ])
}

// Row 1 (kept): every existing field keeps its value, `lines` stays one per (currency, group), the
// ETag is unchanged. Green on `main` too; it must stay green.
#[tokio::test]
async fn existing_fields_lines_and_etag_are_unchanged_on_a_tiered_publication() {
    let db = TestDb::start().await;
    let policy = db.policy("MAIN", None, None).await;
    tiers_on(&db, &policy, TOPS).await;
    let published = db
        .publish("MAIN", &[Line::manual("JPY", "100", "23.40", "24.10")])
        .await;
    usd_four_tiers(&db, &published.id, &policy).await;

    let answer = rates(&db).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        answer.etag.as_deref(),
        Some(format!("\"p{}-s0\"", published.id).as_str()),
        "the tag stays \"p<publication id>-s<0|1>\""
    );
    let body = answer.json();
    assert_eq!(
        body["lines"].as_array().map(Vec::len),
        Some(2),
        "one line per currency: {body}"
    );
    let usd = line(&body, "USD");
    assert_eq!(
        usd["buyRate"], "35.10000000",
        "the line is the first-tier row, as today"
    );
    assert_eq!(usd["sellRate"], "35.50000000");
    assert_eq!(usd["unitSize"], "1");
    assert_eq!(usd["group"], "ALL");
    let jpy = line(&body, "JPY");
    assert_eq!(jpy["buyRate"], "23.40000000");
    assert_eq!(jpy["unitSize"], "100");
    assert_eq!(body["publication"]["id"], published.id.as_str());
    assert_eq!(body["stale"], false);
    db.finish().await;
}

// Row 2: four bands. `bandHeadings` from the policy version's tiers; each tiered line lists its
// four rows in tier order as `{:.8}` strings; an `ALL` line has `bands: []`.
#[tokio::test]
async fn four_bands_come_with_their_headings_from_the_policy_version() {
    let db = TestDb::start().await;
    let policy = db.policy("MAIN", None, None).await;
    tiers_on(&db, &policy, TOPS).await;
    let published = db
        .publish("MAIN", &[Line::manual("JPY", "100", "23.40", "24.10")])
        .await;
    usd_four_tiers(&db, &published.id, &policy).await;

    let body = rates(&db).await.json();
    assert_eq!(body["bandHeadings"], headings_of_tops(), "{body}");
    let usd = line(&body, "USD");
    assert_eq!(
        usd["bands"],
        json!([
            { "tier": "T1", "buyRate": "35.10000000", "sellRate": "35.50000000" },
            { "tier": "T2", "buyRate": "35.15000000", "sellRate": "35.45000000" },
            { "tier": "T3", "buyRate": "35.20000000", "sellRate": "35.40000000" },
            { "tier": "T4", "buyRate": "35.25000000", "sellRate": "35.35000000" },
        ]),
        "{body}"
    );
    assert_eq!(
        line(&body, "JPY")["bands"],
        json!([]),
        "an ALL line has no bands: {body}"
    );
    db.finish().await;
}

// Row 3: missing band rows. A tiered line lists only the tiers it has rows for; headings still
// come from the policy version (all four); a band rate that is NULL stays null.
#[tokio::test]
async fn missing_band_rows_are_left_out_and_headings_stay_the_policy_versions() {
    let db = TestDb::start().await;
    let policy = db.policy("MAIN", None, None).await;
    tiers_on(&db, &policy, TOPS).await;
    let published = db.publish("MAIN", &[]).await;
    tier_row(
        &db,
        &published.id,
        "USD",
        "1",
        "T1",
        None,
        Some("10000"),
        "35.10",
        "35.50",
        Some(&policy),
    )
    .await;
    tier_row(
        &db,
        &published.id,
        "USD",
        "1",
        "T2",
        Some("10000"),
        Some("50000"),
        "35.15",
        "35.45",
        Some(&policy),
    )
    .await;
    tier_row(
        &db,
        &published.id,
        "EUR",
        "1",
        "T1",
        None,
        Some("10000"),
        "38.00",
        "38.60",
        Some(&policy),
    )
    .await;
    db.exec(&format!(
        "UPDATE published_rate_lines SET buy_rate = NULL \
         WHERE publication_id = '{}' AND currency_code = 'EUR' AND value_tier = 'T1'",
        published.id
    ))
    .await;

    let answer = rates(&db).await;
    assert_eq!(answer.status, StatusCode::OK);
    let body = answer.json();
    assert_eq!(body["bandHeadings"], headings_of_tops(), "{body}");
    assert_eq!(
        tiers_of(&line(&body, "USD")["bands"]),
        ["T1", "T2"],
        "{body}"
    );
    assert_eq!(
        line(&body, "EUR")["bands"],
        json!([{ "tier": "T1", "buyRate": null, "sellRate": "38.60000000" }]),
        "{body}"
    );
    db.finish().await;
}

// Row 4: lines that disagree with the headings. A row whose stored bounds differ from the policy
// version's is still bucketed by its tier code under that version's heading, and the route answers
// 200 with the version's bounds, never 500 (ruling on PR 3558).
#[tokio::test]
async fn lines_that_disagree_with_the_headings_are_bucketed_by_the_policy_version_never_500() {
    let db = TestDb::start().await;
    let policy = db.policy("MAIN", None, None).await;
    tiers_on(&db, &policy, TOPS).await;
    let published = db.publish("MAIN", &[]).await;
    usd_four_tiers(&db, &published.id, &policy).await;
    // EUR's rows were stored with other bounds (T1 ≤ 20,000, T2 ≤ 60,000, ...).
    for (tier, lo, hi, buy, sell) in [
        ("T1", None, Some("20000"), "38.00", "38.60"),
        ("T2", Some("20000"), Some("60000"), "38.05", "38.55"),
        ("T3", Some("60000"), Some("300000"), "38.10", "38.50"),
        ("T4", Some("300000"), None, "38.15", "38.45"),
    ] {
        tier_row(
            &db,
            &published.id,
            "EUR",
            "1",
            tier,
            lo,
            hi,
            buy,
            sell,
            Some(&policy),
        )
        .await;
    }

    let answer = rates(&db).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "a disagreement is never a 500"
    );
    let body = answer.json();
    assert_eq!(
        body["bandHeadings"],
        headings_of_tops(),
        "the version's bounds, not EUR's: {body}"
    );
    let eur = &line(&body, "EUR")["bands"];
    assert_eq!(tiers_of(eur), ["T1", "T2", "T3", "T4"], "{body}");
    assert_eq!(
        eur[1]["buyRate"], "38.05000000",
        "EUR's T2 row under heading T2: {body}"
    );
    db.finish().await;
}

// Row 5: an old publication whose policy version has no tiers (tiers off, or no tier rows), and a
// publication whose lines name no policy version at all: no headings, and every line `bands: []`.
#[tokio::test]
async fn a_policy_version_without_tiers_gives_no_headings_and_no_bands() {
    let db = TestDb::start().await;
    let policy = db.policy("MAIN", None, None).await; // value_tiers_on stays false, no tier rows
    let published = db.publish("MAIN", &[]).await;
    usd_four_tiers(&db, &published.id, &policy).await;

    let answer = rates(&db).await;
    assert_eq!(answer.status, StatusCode::OK);
    let body = answer.json();
    assert_eq!(body["bandHeadings"], json!([]), "{body}");
    assert_eq!(
        line(&body, "USD")["bands"],
        json!([]),
        "no heading to bucket under: {body}"
    );

    // A publication of MANUAL lines only: no policy_version_id anywhere.
    db.publish("MAIN", &[Line::manual("JPY", "100", "23.40", "24.10")])
        .await;
    let body = rates(&db).await.json();
    assert_eq!(body["bandHeadings"], json!([]), "{body}");
    assert_eq!(line(&body, "JPY")["bands"], json!([]), "{body}");
    db.finish().await;
}

// The one loader: the route's body is `published_rates_data`'s, so the v3 `published_rates` view
// and the staff view carry the same two fields.
#[tokio::test]
async fn the_shared_loader_carries_the_bands_for_every_reader() {
    let db = TestDb::start().await;
    let policy = db.policy("MAIN", None, None).await;
    tiers_on(&db, &policy, TOPS).await;
    let published = db.publish("MAIN", &[]).await;
    usd_four_tiers(&db, &published.id, &policy).await;

    let shared = published_rates_data(&db.pool, &BranchId::new("MAIN"))
        .await
        .expect("the shared loader");
    let shared = serde_json::to_value(&shared).expect("serialise");
    assert_eq!(shared["bandHeadings"], headings_of_tops(), "{shared}");
    assert_eq!(
        tiers_of(&line(&shared, "USD")["bands"]),
        ["T1", "T2", "T3", "T4"],
        "{shared}"
    );
    assert_eq!(
        rates(&db).await.json(),
        shared,
        "the route's body is the loader's"
    );
    db.finish().await;
}
