//! RF-4 #943 (design `docs/design/blueprint/rates.md` §4.10, row RF-4): the `admin_booking_settings` view — what the
//! Admin Bookings "create" dialog reads beside its counter rates — answers the freshness of the branch's current
//! publication, `data.freshness = { stale, sourceDate, publishedAt }` (`RateFreshnessDto`, RF-1), computed by the ONE
//! rule (`Freshness::of_side` for both sides of every line, then `Freshness::of_publication`), so the dialog and the
//! `published_rates` screen never disagree.
//!
//! Written before the implementation by claude/dell-claude-4 on the lane controller's order (ticket author's work).
//! Do not edit, delete or weaken these tests; add your own in another file.
//!
//! What is pinned (ticket §8 and §9; one test per row):
//! - AUTO sides within their age: `stale: false`, `sourceDate` = the OLDEST reference date (not the first line's),
//!   `publishedAt` = the publication's instant.
//! - Any side past its age is `stale: true`: on an AUTO line, and on a MANUAL line (RF-1's rule: the mode does not
//!   matter), and on a line the dialog does not list (a non-`ALL` denomination group: the publication folds every
//!   line, as `published_rates` does).
//! - MANUAL with no reference: `stale: false`, `sourceDate: null`.
//! - No publication for the branch: `{ stale: false, sourceDate: null, publishedAt: null }`, `counterRates` empty.
//! - On every case with a publication, the view's `stale` and `sourceDate` equal `published_rates'` for the branch.
//! - The counter rates the dialog lists are unchanged by the new key (one entry per `ALL` line, sorted by code).
//! - The view's source calls `Freshness::` and reads no `fetched_at` / `max_reference_age_hours` itself (done
//!   criterion 4).
//! - `generated/admin_bookings.ts` declares `freshness: RateFreshnessDto` on the settings type.
//!
//! The publications are seeded by SQL with a crafted `source_snapshot` (the keys the publisher writes), as in RF-1's
//! `rate_freshness_test.rs` and RF-3's `v3_kiosk_currencies_freshness_test.rs`. The clock is the wall clock:
//! references are 30 h or 1 h old against a 24 h limit.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips. Each test
//! works in a schema of its own.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration as StdDuration, SystemTime, UNIX_EPOCH};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const MUST_FINISH: StdDuration = StdDuration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const MAIN: &str = "MAIN";
/// The publication's time; microseconds on purpose, so a format that drops them still names the same second.
const PUBLISHED_AT: &str = "2026-10-10T03:15:00.123456Z";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn time(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("{text}: {e}"))
        .with_timezone(&Utc)
}

// ------------------------------------------------------------ database

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    /// A schema of its own with `api/migrations/` applied (which seeds `MAIN`).
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The admin booking settings freshness tests need a real PostgreSQL and do not skip. \
                 Point it at an existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`."
        );
        let schema = unique("v3bookfresh");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");
        Self { pool, schema }
    }

    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

/// One line of a seeded publication; the snapshot keys are the ones the publisher writes.
struct Line {
    currency: &'static str,
    buy_mode: &'static str,
    sell_mode: &'static str,
    fetched_hours_ago: Option<i64>,
    max_hours: Option<i64>,
    source_date: Option<&'static str>,
}

struct LineWithGroup {
    currency: &'static str,
    denomination_group: &'static str,
    buy_mode: &'static str,
    sell_mode: &'static str,
    fetched_hours_ago: Option<i64>,
    max_hours: Option<i64>,
    source_date: Option<&'static str>,
}

/// One publication on MAIN at `PUBLISHED_AT` holding `lines`, made current. Bypasses the draft flow on purpose: the
/// snapshot is the input under test.
async fn publish(pool: &PgPool, lines: &[Line]) {
    let with_groups: Vec<_> = lines
        .iter()
        .map(|l| LineWithGroup {
            currency: l.currency,
            denomination_group: "ALL",
            buy_mode: l.buy_mode,
            sell_mode: l.sell_mode,
            fetched_hours_ago: l.fetched_hours_ago,
            max_hours: l.max_hours,
            source_date: l.source_date,
        })
        .collect();
    publish_with_groups(pool, &with_groups).await;
}

async fn publish_with_groups(pool: &PgPool, lines: &[LineWithGroup]) {
    let at = time(PUBLISHED_AT);
    let request: String = sqlx::query_scalar(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
         proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES (gen_random_uuid(), 'RATE_PUBLICATION', $1, 'rf4-seed', 'h', 'p', '{}'::jsonb, 'test', $2, \
         'APPROVED', gen_random_uuid()::text) RETURNING id::text",
    )
    .bind(MAIN)
    .bind(at)
    .fetch_one(pool)
    .await
    .expect("an approved request");
    let publication: String = sqlx::query_scalar(
        "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
         VALUES (gen_random_uuid(), $1, \
                 (SELECT coalesce(max(version), 0) + 1 FROM rate_publications WHERE branch_id = $1), \
                 $2, $3::uuid, 'test') RETURNING id::text",
    )
    .bind(MAIN)
    .bind(at)
    .bind(&request)
    .fetch_one(pool)
    .await
    .expect("a publication");
    for l in lines {
        let mut snapshot =
            json!({ "buy_mode": l.buy_mode, "sell_mode": l.sell_mode, "partner": null });
        if let Some(h) = l.fetched_hours_ago {
            snapshot["fetched_at"] = json!((Utc::now() - Duration::hours(h)).to_rfc3339());
        }
        if let Some(m) = l.max_hours {
            snapshot["max_reference_age_hours"] = json!(m);
        }
        if let Some(d) = l.source_date {
            snapshot["source_date"] = json!(d);
        }
        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, unit_size, \
             buy_rate, sell_rate, source_snapshot) \
             VALUES (gen_random_uuid(), $1::uuid, $2, $3, 1, 32.10000000, 32.50000000, $4)",
        )
        .bind(&publication)
        .bind(l.currency)
        .bind(l.denomination_group)
        .bind(&snapshot)
        .execute(pool)
        .await
        .expect("a line");
    }
    sqlx::query("UPDATE branches SET current_publication_id = $1::uuid WHERE id = $2")
        .bind(&publication)
        .bind(MAIN)
        .execute(pool)
        .await
        .expect("make it current");
}

// ------------------------------------------------------------ callers and requests

/// The owner when the header `x-test-as: owner` is present; nobody otherwise.
struct HeaderSessions;

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let owner = req
            .headers()
            .get(AS)
            .is_some_and(|v| v.as_bytes() == b"owner");
        Box::pin(async move {
            Sessions {
                owner: owner.then(|| Actor::owner("owner-1")),
                staff: None,
            }
        })
    }
}

/// `data` of `GET /api/v3/views/<view>?branch=MAIN` as the owner; must be 200 and changed.
async fn data(db: &TestDb, view: &str) -> Value {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let uri = format!("/api/v3/views/{view}?branch={MAIN}");
    let req = atest::TestRequest::get()
        .uri(&uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, "owner"))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert_eq!(status, StatusCode::OK, "GET {uri}: {text:.400}");
    let body: Value = serde_json::from_slice(&bytes).expect("a JSON body");
    assert_eq!(body["changed"], json!(true), "GET {uri}: {text:.400}");
    body["data"].clone()
}

const SETTINGS: &str = "admin_booking_settings";

/// `data` of `admin_booking_settings` and its `freshness`, which has exactly the three keys of `RateFreshnessDto`.
async fn settings(db: &TestDb) -> (Value, Value) {
    let data = data(db, SETTINGS).await;
    let f = data
        .get("freshness")
        .unwrap_or_else(|| panic!("{SETTINGS} data has `freshness`; keys: {:?}", keys(&data)))
        .clone();
    let mut got = keys(&f);
    got.sort_unstable();
    assert_eq!(
        got,
        vec!["publishedAt", "sourceDate", "stale"],
        "freshness is a RateFreshnessDto: {f}"
    );
    (data, f)
}

fn keys(v: &Value) -> Vec<String> {
    v.as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

/// `publishedAt` names the publication's instant (to the second; sub-second digits may be dropped or kept).
fn assert_published_at(f: &Value) {
    let text = f["publishedAt"]
        .as_str()
        .unwrap_or_else(|| panic!("publishedAt is a string: {f}"));
    let got = DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("publishedAt {text:?} is RFC 3339: {e}"))
        .with_timezone(&Utc);
    let want = time(PUBLISHED_AT);
    assert_eq!(
        got.timestamp(),
        want.timestamp(),
        "publishedAt {text} is the publication's {PUBLISHED_AT}"
    );
}

/// The dialog's view agrees with `published_rates` on the same branch: one rule.
async fn assert_agrees_with_published_rates(db: &TestDb, f: &Value) {
    let screen = data(db, "published_rates").await;
    assert_eq!(
        f["stale"], screen["stale"],
        "stale: {SETTINGS} {f} vs published_rates {screen}"
    );
    assert_eq!(
        f["sourceDate"], screen["sourceDate"],
        "sourceDate: {SETTINGS} {f} vs published_rates {screen}"
    );
}

/// The currencies the dialog lists, in order.
fn listed(data: &Value) -> Vec<String> {
    data["counterRates"]
        .as_array()
        .unwrap_or_else(|| panic!("counterRates is a list: {data}"))
        .iter()
        .map(|r| r["currency"].as_str().unwrap_or_default().to_owned())
        .collect()
}

fn line(
    currency: &'static str,
    mode: &'static str,
    fetched_hours_ago: Option<i64>,
    source_date: Option<&'static str>,
) -> Line {
    Line {
        currency,
        buy_mode: mode,
        sell_mode: mode,
        fetched_hours_ago,
        max_hours: Some(24),
        source_date,
    }
}

// ------------------------------------------------------------ §8, one test per row

/// §8 row 1: AUTO sides within their age. `sourceDate` is the OLDEST date; the older one is on USD, which sorts
/// after EUR, so a "first line's date" reading answers the wrong day.
#[actix_web::test]
async fn auto_sides_within_their_age_are_fresh_with_the_oldest_date_and_the_publication_time() {
    let db = TestDb::start().await;
    publish(
        &db.pool,
        &[
            line("EUR", "AUTO", Some(1), Some("2026-10-09")),
            line("USD", "AUTO", Some(2), Some("2026-10-08")),
        ],
    )
    .await;

    let (data, f) = settings(&db).await;
    assert_eq!(f["stale"], json!(false), "{f}");
    assert_eq!(
        f["sourceDate"],
        json!("2026-10-08"),
        "the oldest reference: {f}"
    );
    assert_published_at(&f);
    assert_eq!(
        listed(&data),
        vec!["EUR", "USD"],
        "the dialog's list is unchanged: {data}"
    );
    assert_agrees_with_published_rates(&db, &f).await;
    db.finish().await;
}

/// §8 row 2: an AUTO side past its age (30 h against 24 h) makes the publication stale.
#[actix_web::test]
async fn an_auto_side_past_its_age_is_stale_in_the_create_dialog() {
    let db = TestDb::start().await;
    publish(
        &db.pool,
        &[
            line("EUR", "AUTO", Some(1), Some("2026-10-09")),
            line("USD", "AUTO", Some(30), Some("2026-10-07")),
        ],
    )
    .await;

    let (_, f) = settings(&db).await;
    assert_eq!(f["stale"], json!(true), "{f}");
    assert_eq!(f["sourceDate"], json!("2026-10-07"), "{f}");
    assert_published_at(&f);
    assert_agrees_with_published_rates(&db, &f).await;
    db.finish().await;
}

/// §8 row 2, "MANUAL included": the only old reference sits on a MANUAL line; the AUTO line is fresh.
#[actix_web::test]
async fn a_publication_whose_only_old_reference_is_on_a_manual_side_is_stale_in_the_create_dialog()
{
    let db = TestDb::start().await;
    publish(
        &db.pool,
        &[
            line("EUR", "MANUAL", Some(30), Some("2026-10-06")),
            line("USD", "AUTO", Some(1), Some("2026-10-09")),
        ],
    )
    .await;

    let (_, f) = settings(&db).await;
    assert_eq!(f["stale"], json!(true), "{f}");
    assert_eq!(
        f["sourceDate"],
        json!("2026-10-06"),
        "the oldest reference: {f}"
    );
    assert_published_at(&f);
    assert_agrees_with_published_rates(&db, &f).await;
    db.finish().await;
}

/// §8 row 3: a MANUAL line with no reference is fresh and has no source date; the publication time still shows.
#[actix_web::test]
async fn a_manual_line_with_no_reference_is_fresh_with_no_source_date() {
    let db = TestDb::start().await;
    publish(&db.pool, &[line("USD", "MANUAL", None, None)]).await;

    let (data, f) = settings(&db).await;
    assert_eq!(f["stale"], json!(false), "{f}");
    assert_eq!(f["sourceDate"], json!(null), "{f}");
    assert_published_at(&f);
    assert_eq!(listed(&data), vec!["USD"], "{data}");
    assert_agrees_with_published_rates(&db, &f).await;
    db.finish().await;
}

/// §8 row 4: no publication for the branch — nothing to be stale, no dates, and no counter rates, as today.
#[actix_web::test]
async fn with_no_publication_the_freshness_is_empty_and_no_counter_rate_is_offered() {
    let db = TestDb::start().await;
    let (data, f) = settings(&db).await;
    assert_eq!(
        f,
        json!({ "stale": false, "sourceDate": null, "publishedAt": null }),
        "no publication"
    );
    assert_eq!(data["counterRates"], json!([]), "{data}");
    db.finish().await;
}

/// §9 "both sides of every line": the only old reference is on a non-`ALL` line, which the dialog does not list.
/// The publication is still stale, as `published_rates` says.
#[actix_web::test]
async fn an_old_reference_on_a_line_the_dialog_does_not_list_still_makes_it_stale() {
    let db = TestDb::start().await;
    publish_with_groups(
        &db.pool,
        &[
            LineWithGroup {
                currency: "USD",
                denomination_group: "ALL",
                buy_mode: "AUTO",
                sell_mode: "AUTO",
                fetched_hours_ago: Some(1),
                max_hours: Some(24),
                source_date: Some("2026-10-09"),
            },
            LineWithGroup {
                currency: "USD",
                denomination_group: "50-100",
                buy_mode: "MANUAL",
                sell_mode: "MANUAL",
                fetched_hours_ago: Some(30),
                max_hours: Some(24),
                source_date: Some("2026-10-06"),
            },
        ],
    )
    .await;

    let (data, f) = settings(&db).await;
    assert_eq!(
        f["stale"],
        json!(true),
        "the publication folds every line: {f}"
    );
    assert_eq!(f["sourceDate"], json!("2026-10-06"), "{f}");
    assert_published_at(&f);
    assert_eq!(
        listed(&data),
        vec!["USD"],
        "the dialog lists the ALL line only: {data}"
    );
    assert_agrees_with_published_rates(&db, &f).await;
    db.finish().await;
}

// ------------------------------------------------------------ one rule, in the source; the generated type

/// Done criterion 4 and §4.10 ("no reader computes staleness any other way"): the view calls `Freshness::` and
/// reads no snapshot age key itself.
#[test]
fn the_admin_bookings_view_file_calls_the_one_rule() {
    let path = format!(
        "{}/src/presentation/http/v3/views/admin_bookings.rs",
        env!("CARGO_MANIFEST_DIR")
    );
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    assert!(
        src.contains("Freshness::of_publication"),
        "{path} calls Freshness::of_publication"
    );
    assert!(
        src.contains("Side::Buy") && src.contains("Side::Sell"),
        "{path} folds both sides of each line"
    );
    assert!(
        !src.contains("max_reference_age") && !src.contains("fetched_at"),
        "{path} reads no snapshot age key itself"
    );
}

/// §3 item 2: `generated/admin_bookings.ts` (the committed file the screen imports) declares `freshness:
/// RateFreshnessDto` on `AdminBookingSettingsData`.
#[test]
fn the_generated_settings_type_carries_freshness() {
    let path = format!(
        "{}/../web/src/shared/api/generated/admin_bookings.ts",
        env!("CARGO_MANIFEST_DIR")
    );
    let ts = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let decl = ts
        .lines()
        .find(|l| l.starts_with("export type AdminBookingSettingsData = "))
        .unwrap_or_else(|| panic!("AdminBookingSettingsData in {path}"));
    assert!(
        decl.contains("freshness: RateFreshnessDto,"),
        "the settings type carries `freshness: RateFreshnessDto`: {decl}"
    );
    assert!(
        ts.lines().any(|l| l.starts_with("import type {")
            && l.contains("RateFreshnessDto")
            && l.contains("\"./common\"")),
        "{path} imports RateFreshnessDto from common.ts"
    );
}
