//! Ticket #603 (RE-6): the `kiosk_currencies` view reads the branch's current rate publication,
//! not the old `rates` table, and compares it with KBank per 1.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §3.3 (a rate on the wire is the 8-place string the
//! database holds; HALF_UP), §4.1 (`published_rate_lines`, a NULL side is unavailable,
//! `source_snapshot.partner`), §4.8 (every reader reads the current publication), §9 RE-6.
//!
//! What is pinned, per currency of the view (`GET /api/v3/views/kiosk_currencies`):
//! - `buy` / `sell` are the current publication line's `buy_rate` / `sell_rate` exactly as stored:
//!   the 8-place strings (`"23.50000000"` for JPY per 100), `null` for a NULL side.
//! - `unitSize` [required, new]: a decimal string with no trailing zeros. The line's `unit_size`
//!   when the currency has a line (`"100"` for JPY, `"10"` for a CNY line stored per 10 although
//!   the register says 1), else the currency's register unit (`"1"` for USD, `"100"` for JPY,
//!   `"1000"` for KRW).
//! - `updated` is that publication's `published_at`, RFC3339 with milliseconds and `Z`
//!   (`"2026-10-06T05:00:01.123Z"` for `05:00:01.123456`), as today's format; `""` when the
//!   currency has no line.
//! - `partnerName` / `partnerCost` are `source_snapshot.partner.name` / `.cost` (the cost's text
//!   unchanged), or `""` / `null` when `partner` is `null` or absent.
//! - A currency with no line, and every currency before the branch's first publication: `buy`,
//!   `sell`, `partnerCost` `null`, `partnerName` and `updated` `""`.
//! - The old `rates` table is never read: rows in it change nothing, and the view answers with the
//!   table renamed away.
//! - KBank's file is "THB per 1": the shop side of each comparison is the published rate ÷ the
//!   line's `unit_size` (JPY 23.4 per 100 compares as 0.234). The shop figure and the difference
//!   (shop − bank, from the exact per-1 value, rounded once) keep five places, rounded HALF_UP
//!   (`MidpointAwayFromZero`): a midpoint case below differs from banker's rounding and from
//!   truncation, on both signs.
//! - A publish through the real routes moves the version the view answers (`since`).
//!
//! The publications are seeded straight into the tables (`publication_seed`), except in the last
//! test, which publishes through `POST /api/rate-drafts`, `…/submit` and `…/publish`.
//!
//! Assumption (ticket author's ruling on #603, 2026-10-09): the view may read the current
//! publication's lines with a query of its own, because `PublishedRates::current`
//! (`xc-rate/src/infrastructure/publication/published_rates.rs:27-56`) answers the publication's
//! header only. Nothing here depends on how the lines are read.
//!
//! Callers are given by the request header `x-test-as` through a `SessionReader`, as in
//! `v3_rate_engine_routes_test.rs`; who may read the view is pinned in
//! `v3_kiosk_currencies_view_test.rs`, and this file only reads as the owner.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips. Each test works in a schema of its own.
//!
//! Signatures/fields this test assumes: `KioskCurrency` gains `unit_size: String`
//! (`#[serde(rename_all = "camelCase")]` ⇒ `unitSize`, exported to TypeScript as
//! `unitSize: string`); every other field of `dto/v3/kiosk_currencies.rs` keeps its name and type;
//! the RE-3 command bodies of `v3_rate_engine_routes_test.rs`.

mod common;
mod publication_seed;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use publication_seed::{Line, Partner, both, publish};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http::v3::VIEWS;
use xchange::presentation::pages::{SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const VIEW: &str = "kiosk_currencies";
const MAIN: &str = "MAIN";
const B9: &str = "B9";

/// The keys of one currency: today's eleven plus `unitSize`.
const CURRENCY_KEYS: [&str; 12] = [
    "code",
    "name",
    "nameEn",
    "countries",
    "buy",
    "sell",
    "unitSize",
    "updated",
    "partnerName",
    "partnerCost",
    "enabled",
    "kbank",
];

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
    /// A schema of its own with `api/migrations/` applied (which seeds `MAIN`), plus `B9`.
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The kiosk currencies publication tests need a real PostgreSQL and do not skip. \
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
        let schema = unique("v3kioskpub");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('B9', 'Nine')")
            .execute(&pool)
            .await
            .expect("insert B9");
        Self { pool, schema }
    }

    /// An old counter rate in `rates`, which the view must not read.
    async fn old_rate(&self, branch: &str, code: &str, buy: &str, sell: &str, partner: &str) {
        let cost: Option<&str> = if partner.is_empty() {
            None
        } else {
            Some("1.5")
        };
        sqlx::query(
            "INSERT INTO rates (branch_id, code, buy, sell, partner_name, partner_cost, \
             updated_at, updated_by, request_id) \
             VALUES ($1, $2, $3::numeric, $4::numeric, $5, $6::numeric, $7, 'test', $8)",
        )
        .bind(branch)
        .bind(code)
        .bind(buy)
        .bind(sell)
        .bind(partner)
        .bind(cost)
        .bind(time("2026-10-06T04:00:00.000Z"))
        .bind(unique("req"))
        .execute(&self.pool)
        .await
        .expect("insert an old counter rate");
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

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(db: &TestDb, method: Method, uri: &str, body: Option<Value>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, "owner"));
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", unique("key")))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// The view as the owner (`query` e.g. `branch=B9`, `since=3`); must be 200.
async fn view(db: &TestDb, query: &str) -> Value {
    let uri = if query.is_empty() {
        format!("/api/v3/views/{VIEW}")
    } else {
        format!("/api/v3/views/{VIEW}?{query}")
    };
    let a = call(db, Method::GET, &uri, None).await;
    assert_eq!(a.status, StatusCode::OK, "GET {uri}: {:.400}", a.text);
    a.body
}

/// `data` of the view read with no `since`.
async fn data(db: &TestDb, query: &str) -> Value {
    let body = view(db, query).await;
    assert_eq!(body["changed"], json!(true), "{query}: {body:.400}");
    body["data"].clone()
}

fn currency<'a>(data: &'a Value, code: &str) -> &'a Value {
    data["currencies"]
        .as_array()
        .unwrap_or_else(|| panic!("currencies is an array: {data}"))
        .iter()
        .find(|c| c["code"] == json!(code))
        .unwrap_or_else(|| panic!("{code} in the view's currencies"))
}

/// The rate fields of one currency: `[buy, sell, unitSize, updated, partnerName, partnerCost]`.
fn rate_fields(c: &Value) -> Value {
    json!([
        c["buy"],
        c["sell"],
        c["unitSize"],
        c["updated"],
        c["partnerName"],
        c["partnerCost"]
    ])
}

/// A currency with no published line, at the register unit `unit`.
fn unpublished(unit: &str) -> Value {
    json!([null, null, unit, "", "", null])
}

/// A comparison row (`None` = `null`).
fn row(label: &str, values: [Option<&str>; 6]) -> Value {
    let [
        bank_buy,
        shop_buy,
        buy_diff,
        bank_sell,
        shop_sell,
        sell_diff,
    ] = values;
    json!({
        "label": label,
        "bankBuy": bank_buy, "shopBuy": shop_buy, "buyDifference": buy_diff,
        "bankSell": bank_sell, "shopSell": shop_sell, "sellDifference": sell_diff,
    })
}

const T1: &str = "2026-10-06T05:00:01.123456Z";
const T1_MS: &str = "2026-10-06T05:00:01.123Z";
const T2: &str = "2026-10-06T06:30:00.000999Z";
const T2_MS: &str = "2026-10-06T06:30:00.000Z";

// ------------------------------------------------------------ before any publication

/// Before the branch's first publication every currency has null rates, whatever `rates` holds;
/// `unitSize` is the register's unit.
#[actix_web::test]
async fn before_the_first_publication_every_currency_has_null_rates() {
    let db = TestDb::start().await;
    db.old_rate(MAIN, "USD", "32.1", "33", "SuperRich").await;
    db.old_rate(MAIN, "JPY", "0.2105", "0.2205", "").await;

    let data = data(&db, "").await;
    let currencies = data["currencies"].as_array().expect("currencies");
    assert_eq!(currencies.len(), 152, "the table's 152 currencies");
    for c in currencies {
        let code = c["code"].as_str().expect("a code");
        let mut keys: Vec<&str> = c
            .as_object()
            .expect("a currency is an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        let mut want = CURRENCY_KEYS.to_vec();
        want.sort_unstable();
        assert_eq!(keys, want, "{code}: the keys");
        let unit = match code {
            "JPY" => "100",
            "KRW" => "1000",
            _ => "1",
        };
        assert_eq!(rate_fields(c), unpublished(unit), "{code}");
        for r in c["kbank"].as_array().expect("kbank") {
            assert_eq!(r["shopBuy"], Value::Null, "{code}: {r}");
            assert_eq!(r["shopSell"], Value::Null, "{code}: {r}");
            assert_eq!(r["buyDifference"], Value::Null, "{code}: {r}");
            assert_eq!(r["sellDifference"], Value::Null, "{code}: {r}");
        }
    }
    db.finish().await;
}

// ------------------------------------------------------------ the current publication

/// The current publication's lines, exactly as stored, with the line's unit, the publication's
/// time and the partner from the snapshot; a currency with no line answers nulls.
#[actix_web::test]
async fn the_view_answers_the_current_publications_lines_as_stored() {
    let db = TestDb::start().await;
    // rows in the old table that would answer otherwise
    db.old_rate(MAIN, "USD", "30", "31", "Old Partner").await;
    db.old_rate(MAIN, "AFN", "0.45", "0.5", "").await;
    publish(
        &db.pool,
        MAIN,
        time(T1),
        &[
            both("JPY", "23.4", "23.5"),
            Line {
                code: "USD".to_owned(),
                unit_size: None,
                buy: Some("32.1".to_owned()),
                sell: Some("32.45".to_owned()),
                partner: Partner::Quote("SuperRich".to_owned(), "31.95000000".to_owned()),
            },
            Line {
                code: "EUR".to_owned(),
                unit_size: None,
                buy: Some("37.12345678".to_owned()),
                sell: None,
                partner: Partner::Absent,
            },
            Line {
                code: "CNY".to_owned(),
                unit_size: Some("10".to_owned()),
                buy: Some("45.5".to_owned()),
                sell: Some("50.5".to_owned()),
                partner: Partner::Null,
            },
        ],
    )
    .await;

    let data = data(&db, "").await;
    assert_eq!(
        rate_fields(currency(&data, "JPY")),
        json!(["23.40000000", "23.50000000", "100", T1_MS, "", null]),
        "JPY per 100, as stored"
    );
    assert_eq!(
        rate_fields(currency(&data, "USD")),
        json!([
            "32.10000000",
            "32.45000000",
            "1",
            T1_MS,
            "SuperRich",
            "31.95000000"
        ]),
        "USD with the snapshot's partner"
    );
    assert_eq!(
        rate_fields(currency(&data, "EUR")),
        json!(["37.12345678", null, "1", T1_MS, "", null]),
        "EUR: sell unavailable, no partner key"
    );
    assert_eq!(
        rate_fields(currency(&data, "CNY")),
        json!(["45.50000000", "50.50000000", "10", T1_MS, "", null]),
        "CNY: the line's unit, not the register's"
    );
    assert_eq!(
        rate_fields(currency(&data, "AFN")),
        unpublished("1"),
        "AFN: no line"
    );
    assert_eq!(
        rate_fields(currency(&data, "KRW")),
        unpublished("1000"),
        "KRW: no line"
    );
    for c in data["currencies"].as_array().expect("currencies") {
        for key in ["buy", "sell", "unitSize", "partnerCost"] {
            assert!(
                c[key].is_string() || c[key].is_null(),
                "{}: {key} is a string or null, never a number: {}",
                c["code"],
                c[key]
            );
        }
    }
    db.finish().await;
}

/// Only the branch's current publication is read: a newer one replaces an older one's lines,
/// times and partners, and a currency only in the older one answers nulls.
#[actix_web::test]
async fn only_the_current_publication_is_read() {
    let db = TestDb::start().await;
    publish(
        &db.pool,
        MAIN,
        time(T1),
        &[
            Line {
                code: "USD".to_owned(),
                unit_size: None,
                buy: Some("32.1".to_owned()),
                sell: Some("32.45".to_owned()),
                partner: Partner::Quote("SuperRich".to_owned(), "31.95000000".to_owned()),
            },
            both("EUR", "37", "38.1"),
        ],
    )
    .await;
    publish(&db.pool, MAIN, time(T2), &[both("USD", "32.2", "32.5")]).await;

    let data = data(&db, "branch=MAIN").await;
    assert_eq!(
        rate_fields(currency(&data, "USD")),
        json!(["32.20000000", "32.50000000", "1", T2_MS, "", null]),
        "version 2's line, time and (no) partner"
    );
    assert_eq!(
        rate_fields(currency(&data, "EUR")),
        unpublished("1"),
        "EUR is only in version 1"
    );
    db.finish().await;
}

/// Each branch reads its own publication.
#[actix_web::test]
async fn each_branch_reads_its_own_publication() {
    let db = TestDb::start().await;
    publish(&db.pool, MAIN, time(T1), &[both("USD", "32.1", "32.45")]).await;
    publish(&db.pool, B9, time(T2), &[both("SGD", "25.9", "26.6")]).await;

    let main = data(&db, "branch=MAIN").await;
    assert_eq!(
        rate_fields(currency(&main, "USD")),
        json!(["32.10000000", "32.45000000", "1", T1_MS, "", null])
    );
    assert_eq!(rate_fields(currency(&main, "SGD")), unpublished("1"));

    let b9 = data(&db, "branch=B9").await;
    assert_eq!(rate_fields(currency(&b9, "USD")), unpublished("1"));
    assert_eq!(
        rate_fields(currency(&b9, "SGD")),
        json!(["25.90000000", "26.60000000", "1", T2_MS, "", null])
    );
    db.finish().await;
}

/// The old `rates` table is never read: with it renamed away the view still answers, from the
/// publication.
#[actix_web::test]
async fn the_view_answers_with_the_rates_table_gone() {
    let db = TestDb::start().await;
    publish(&db.pool, MAIN, time(T1), &[both("USD", "32.1", "32.45")]).await;
    sqlx::query("ALTER TABLE rates RENAME TO hidden_rates")
        .execute(&db.pool)
        .await
        .expect("rename rates");
    let data = data(&db, "").await;
    assert_eq!(
        rate_fields(currency(&data, "USD")),
        json!(["32.10000000", "32.45000000", "1", T1_MS, "", null])
    );
    db.finish().await;
}

// ------------------------------------------------------------ KBank per 1

/// JPY at 23.4 / 23.5 per 100 compares with KBank's per-1 figures as 0.234 / 0.235; a CNY line
/// per 10 is divided by its own unit.
#[actix_web::test]
async fn kbank_compares_the_published_rate_per_one() {
    let db = TestDb::start().await;
    publish(
        &db.pool,
        MAIN,
        time(T1),
        &[
            both("JPY", "23.4", "23.5"),
            Line {
                code: "CNY".to_owned(),
                unit_size: Some("10".to_owned()),
                buy: Some("45.5".to_owned()),
                sell: Some("50.5".to_owned()),
                partner: Partner::Null,
            },
        ],
    )
    .await;
    let data = data(&db, "").await;
    let s = Some;
    assert_eq!(
        currency(&data, "JPY")["kbank"],
        json!([row(
            "JPY",
            [
                s("0.20715"),
                s("0.23400"),
                s("0.02685"),
                s("0.21844"),
                s("0.23500"),
                s("0.01656")
            ]
        )]),
        "JPY: 23.4 / 100 = 0.234 against 0.20715"
    );
    assert_eq!(
        currency(&data, "CNY")["kbank"],
        json!([row(
            "CNY",
            [
                s("4.66643"),
                s("4.55000"),
                s("-0.11643"),
                s("5.16131"),
                s("5.05000"),
                s("-0.11131")
            ]
        )]),
        "CNY: 45.5 / 10 = 4.55 against 4.66643"
    );
    db.finish().await;
}

/// Five places, HALF_UP (away from zero), where banker's rounding and truncation would differ:
/// - KRW per 1000: 21.585 → 0.021585 → `0.02159` (banker's `0.02158`); its difference
///   0.000065 → `0.00007` (banker's `0.00006`); 25.645 → 0.025645 → `0.02565` (banker's
///   `0.02564`); difference 0.000005 → `0.00001` (banker's `0.00000`).
/// - USD per 1: 32.379185 → `32.37919` (banker's `32.37918`); against `USD 1` 32.37919 the
///   difference is −0.000005 → `-0.00001`; against `USD 5-20` 32.58661 it is −0.207425 →
///   `-0.20743` (banker's `-0.20742`). The difference comes from the exact per-1 value, not from
///   the rounded shop figure (which would give `0.00000` against `USD 1`).
#[actix_web::test]
async fn five_place_figures_round_half_up_at_the_midpoint() {
    let db = TestDb::start().await;
    publish(
        &db.pool,
        MAIN,
        time(T1),
        &[
            both("KRW", "21.585", "25.645"),
            both("USD", "32.379185", "32.5"),
        ],
    )
    .await;
    let data = data(&db, "").await;
    assert_eq!(
        rate_fields(currency(&data, "KRW")),
        json!(["21.58500000", "25.64500000", "1000", T1_MS, "", null])
    );
    let s = Some;
    assert_eq!(
        currency(&data, "KRW")["kbank"],
        json!([row(
            "KRW",
            [
                s("0.02152"),
                s("0.02159"),
                s("0.00007"),
                s("0.02564"),
                s("0.02565"),
                s("0.00001")
            ]
        )]),
        "KRW per 1000"
    );
    assert_eq!(
        currency(&data, "USD")["kbank"],
        json!([
            row(
                "USD 1",
                [
                    s("32.37919"),
                    s("32.37919"),
                    s("-0.00001"),
                    s("33.81656"),
                    s("32.50000"),
                    s("-1.31656"),
                ],
            ),
            row(
                "USD 5-20",
                [
                    s("32.58661"),
                    s("32.37919"),
                    s("-0.20743"),
                    s("33.81656"),
                    s("32.50000"),
                    s("-1.31656"),
                ],
            ),
            row(
                "USD 50-100",
                [
                    s("33.10516"),
                    s("32.37919"),
                    s("-0.72598"),
                    s("33.85113"),
                    s("32.50000"),
                    s("-1.35113"),
                ],
            ),
        ]),
        "USD per 1"
    );
    db.finish().await;
}

/// A side that is unavailable leaves its shop figure and difference `null`, the other side shows.
#[actix_web::test]
async fn an_unavailable_side_has_no_shop_figure() {
    let db = TestDb::start().await;
    publish(
        &db.pool,
        MAIN,
        time(T1),
        &[Line {
            code: "JPY".to_owned(),
            unit_size: None,
            buy: None,
            sell: Some("23.5".to_owned()),
            partner: Partner::Null,
        }],
    )
    .await;
    let data = data(&db, "").await;
    let s = Some;
    assert_eq!(
        currency(&data, "JPY")["kbank"],
        json!([row(
            "JPY",
            [
                s("0.20715"),
                None,
                None,
                s("0.21844"),
                s("0.23500"),
                s("0.01656")
            ]
        )])
    );
    db.finish().await;
}

// ------------------------------------------------------------ only what changed

/// A publish through the real routes (OWNER: draft → submit → publish) moves the version the
/// view answers, and the same `since` then reports `changed: true` with the published rate.
#[actix_web::test]
async fn a_publish_through_the_routes_moves_the_version_the_view_answers() {
    let db = TestDb::start().await;
    let before = view(&db, "").await;
    let v = before["version"].as_i64().expect("a numeric version");
    assert_eq!(
        rate_fields(currency(&before["data"], "USD")),
        unpublished("1")
    );

    let line = json!({
        "currency": "USD", "buyMode": "MANUAL", "sellMode": "MANUAL",
        "manualBuy": "32.1", "manualSell": "32.45", "reason": "set by hand for the test",
    });
    let a = call(
        &db,
        Method::POST,
        "/api/rate-drafts",
        Some(json!({ "lines": [line] })),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "create a draft: {}", a.text);
    let id = a.body["id"].as_str().expect("the draft's id").to_owned();
    let a = call(
        &db,
        Method::POST,
        &format!("/api/rate-drafts/{id}/submit"),
        Some(json!({})),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "submit: {}", a.text);
    let a = call(
        &db,
        Method::POST,
        &format!("/api/rate-drafts/{id}/publish"),
        Some(json!({ "expectedVersion": null })),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "publish: {}", a.text);

    let after = view(&db, &format!("since={v}")).await;
    let now = after["version"].as_i64().expect("a numeric version");
    assert!(now > v, "the publish moved MAIN's version: {v} → {now}");
    assert_eq!(after["changed"], json!(true), "{after:.300}");
    let usd = currency(&after["data"], "USD");
    assert_eq!(usd["buy"], json!("32.10000000"), "{usd}");
    assert_eq!(usd["sell"], json!("32.45000000"), "{usd}");
    assert_eq!(usd["unitSize"], json!("1"), "{usd}");
    db.finish().await;
}

// ------------------------------------------------------------ the generated types

/// The generated TypeScript carries `unitSize: string` on `KioskCurrency`.
#[test]
fn the_generated_types_carry_unit_size() {
    let view = VIEWS
        .iter()
        .find(|v| v.name == VIEW)
        .expect("kiosk_currencies is in the generated view table");
    let dir = std::env::temp_dir().join(unique("xc-v3-kiosk-currencies-unit"));
    std::fs::create_dir_all(&dir).expect("create a scratch directory");
    (view.export_types)(&dir).expect("export kiosk_currencies");
    let ts = std::fs::read_to_string(dir.join(format!("{VIEW}.ts"))).expect("read the export");
    let _ = std::fs::remove_dir_all(&dir);
    let start = ts
        .find("export type KioskCurrency = ")
        .unwrap_or_else(|| panic!("KioskCurrency in:\n{ts}"));
    let body = &ts[start..];
    let end = body.find('}').expect("the type's closing brace");
    assert!(
        body[..end].contains("unitSize: string"),
        "KioskCurrency has `unitSize: string`:\n{}",
        &body[..end]
    );
}
