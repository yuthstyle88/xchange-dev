//! RF-3 (design `docs/design/blueprint/rates.md` §4.10, PR 843): the `kiosk_currencies` view answers the freshness of
//! the branch's current publication, `data.freshness = { stale, sourceDate, publishedAt }` (`RateFreshnessDto`, RF-1),
//! computed by the ONE rule (`Freshness::of_side` + `Freshness::of_publication`), so the kiosk page and the
//! `published_rates` screen never disagree.
//!
//! Written before the implementation by the ticket author (claude/dell-claude-3). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! What is pinned:
//! - A publication whose only old reference sits on a MANUAL side is `stale: true` (§4.10: a side is stale when its
//!   snapshot has `fetched_at` and `max_reference_age_hours` and the age is past; the mode does not matter).
//! - `sourceDate` is the OLDEST `source_date` among the publication's lines (`%Y-%m-%d`), not the first line's.
//! - `publishedAt` is an RFC 3339 time equal to the publication's `published_at` (the format is the implementer's;
//!   the instant is pinned).
//! - Fresh lines (an AUTO line within its age, a MANUAL line with no reference) are `stale: false`.
//! - Before the branch's first publication: `{ stale: false, sourceDate: null, publishedAt: null }`.
//! - On every case the view's `stale` and `sourceDate` equal `published_rates`' for the same branch.
//! - The view's source calls `Freshness::` and does not read `max_reference_age_hours` itself.
//!
//! The publications are seeded by SQL with a crafted `source_snapshot` (the keys the publisher writes), as in RF-1's
//! `rate_freshness_test.rs`. The clock is the wall clock: references are 30 h or 1 h old against a 24 h limit.
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
                 The kiosk currencies freshness tests need a real PostgreSQL and do not skip. \
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
        let schema = unique("v3kioskfresh");
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
         VALUES (gen_random_uuid(), 'RATE_PUBLICATION', $1, 'rf3-seed', 'h', 'p', '{}'::jsonb, 'test', $2, \
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

/// `data.freshness` of the kiosk currencies view, with exactly the three keys of `RateFreshnessDto`.
async fn freshness(db: &TestDb) -> Value {
    let data = data(db, "kiosk_currencies").await;
    let f = data
        .get("freshness")
        .unwrap_or_else(|| {
            panic!(
                "kiosk_currencies data has `freshness`; keys: {:?}",
                keys(&data)
            )
        })
        .clone();
    let mut got = keys(&f);
    got.sort_unstable();
    assert_eq!(
        got,
        vec!["publishedAt", "sourceDate", "stale"],
        "freshness is a RateFreshnessDto: {f}"
    );
    f
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

/// The kiosk view agrees with `published_rates` on the same branch: one rule.
async fn assert_agrees_with_published_rates(db: &TestDb, f: &Value) {
    let screen = data(db, "published_rates").await;
    assert_eq!(
        f["stale"], screen["stale"],
        "stale: kiosk {f} vs published_rates {screen}"
    );
    assert_eq!(
        f["sourceDate"], screen["sourceDate"],
        "sourceDate: kiosk {f} vs published_rates {screen}"
    );
}

// ------------------------------------------------------------ the cases

/// §4.10: the only old reference sits on a MANUAL line (EUR, 30 h against 24 h); the AUTO line (USD) is fresh.
/// The publication is stale, and its `sourceDate` is the MANUAL line's older date.
#[actix_web::test]
async fn a_publication_whose_only_old_reference_is_on_a_manual_side_is_stale_on_the_kiosk_page() {
    let db = TestDb::start().await;
    publish(
        &db.pool,
        &[
            Line {
                currency: "EUR",
                buy_mode: "MANUAL",
                sell_mode: "MANUAL",
                fetched_hours_ago: Some(30),
                max_hours: Some(24),
                source_date: Some("2026-10-06"),
            },
            Line {
                currency: "USD",
                buy_mode: "AUTO",
                sell_mode: "AUTO",
                fetched_hours_ago: Some(1),
                max_hours: Some(24),
                source_date: Some("2026-10-09"),
            },
        ],
    )
    .await;

    let f = freshness(&db).await;
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

/// A fresh AUTO line and a MANUAL line with no reference: fresh; `sourceDate` is the one date there is. USD sorts
/// after EUR, so a "first line's date" reading would answer `null`.
#[actix_web::test]
async fn fresh_lines_are_fresh_on_the_kiosk_page() {
    let db = TestDb::start().await;
    publish(
        &db.pool,
        &[
            Line {
                currency: "EUR",
                buy_mode: "MANUAL",
                sell_mode: "MANUAL",
                fetched_hours_ago: None,
                max_hours: Some(24),
                source_date: None,
            },
            Line {
                currency: "USD",
                buy_mode: "AUTO",
                sell_mode: "AUTO",
                fetched_hours_ago: Some(1),
                max_hours: Some(24),
                source_date: Some("2026-10-09"),
            },
        ],
    )
    .await;

    let f = freshness(&db).await;
    assert_eq!(f["stale"], json!(false), "{f}");
    assert_eq!(f["sourceDate"], json!("2026-10-09"), "{f}");
    assert_published_at(&f);
    assert_agrees_with_published_rates(&db, &f).await;
    db.finish().await;
}

/// Before the branch's first publication there is nothing to be stale and no time to show.
#[actix_web::test]
async fn before_the_first_publication_the_kiosk_freshness_is_empty() {
    let db = TestDb::start().await;
    let f = freshness(&db).await;
    assert_eq!(
        f,
        json!({ "stale": false, "sourceDate": null, "publishedAt": null }),
        "no publication"
    );
    db.finish().await;
}

/// §4.10: a line with buy_mode AUTO (fresh) and sell_mode MANUAL with an old reference is stale on the kiosk page.
#[actix_web::test]
async fn a_line_with_auto_buy_and_manual_sell_with_an_old_reference_is_stale_on_the_kiosk_page() {
    let db = TestDb::start().await;
    publish(
        &db.pool,
        &[Line {
            currency: "USD",
            buy_mode: "AUTO",
            sell_mode: "MANUAL",
            fetched_hours_ago: Some(30),
            max_hours: Some(24),
            source_date: Some("2026-10-06"),
        }],
    )
    .await;

    let f = freshness(&db).await;
    assert_eq!(f["stale"], json!(true), "{f}");
    assert_eq!(f["sourceDate"], json!("2026-10-06"), "{f}");
    assert_published_at(&f);
    assert_agrees_with_published_rates(&db, &f).await;
    db.finish().await;
}

/// §4.10: a publication whose only old reference is on a non-ALL line (the ALL line of the same currency is fresh)
/// is stale on the kiosk page. Even though kiosk_currencies only renders cards for 'ALL' lines, the publication's
/// freshness folds every line in published_rate_lines.
#[actix_web::test]
async fn a_publication_whose_only_old_reference_is_on_a_non_all_line_is_stale_on_the_kiosk_page() {
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

    let f = freshness(&db).await;
    assert_eq!(
        f["stale"],
        json!(true),
        "the publication folds non-ALL lines: {f}"
    );
    assert_eq!(f["sourceDate"], json!("2026-10-06"), "{f}");
    assert_published_at(&f);
    assert_agrees_with_published_rates(&db, &f).await;
    db.finish().await;
}

// ------------------------------------------------------------ one rule, in the source

/// The view computes freshness through `Freshness::` and reads no age key of its own (§4.10: "No reader computes
/// staleness any other way").
#[test]
fn the_kiosk_currencies_view_calls_the_one_rule() {
    let path = format!(
        "{}/src/presentation/http/v3/views/kiosk_currencies.rs",
        env!("CARGO_MANIFEST_DIR")
    );
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    assert!(src.contains("Freshness::"), "{path} calls Freshness::");
    assert!(
        !src.contains("max_reference_age_hours") && !src.contains("fetched_at"),
        "{path} reads no snapshot age key itself"
    );
}

/// §4.10: The kiosk currencies view folds both Buy and Sell sides of every published line into publication freshness.
#[test]
fn the_kiosk_currencies_view_folds_both_buy_and_sell_sides() {
    let path = format!(
        "{}/src/presentation/http/v3/views/kiosk_currencies.rs",
        env!("CARGO_MANIFEST_DIR")
    );
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    assert!(
        src.contains("Freshness::of_side(&source_snapshot, Side::Buy, now)")
            && src.contains("Freshness::of_side(&source_snapshot, Side::Sell, now)"),
        "{path} folds both Buy and Sell sides of each published line"
    );
}
