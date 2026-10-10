//! Specification tests for V2-cash-valuation (#509; docs/design/xchange-v3-web.md
//! §12.20.4, §12.20.9): the compute action `shift_valuation`, which values a
//! shift's opening, stored expected balance and movements in baht at rates the
//! user typed, on exact decimals, and writes nothing.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! - `POST /api/v3/compute/shift_valuation`, found by the build in
//!   `http/v3/actions/` and registered by `configure_v3` (every call here goes
//!   through `bootstrap::configure`, nothing is registered by hand).
//! - Body `{shiftId, rates?}`; answer `{currencies, result?}`, money and rates
//!   as decimal strings (W7). The wire names are [required]; the Rust type names
//!   are not used here.
//! - v2's contract (`xchange-v2-source@4d0bdbb`, `src/devices-public/`):
//!   `shift-valuation.js:6-7` (which currencies), `:11-12` (`rates = {THB: 1}`),
//!   `shift-valuation-core.js:2-7` (`Math.round(n × r × 100) / 100` per currency
//!   and per movement, IN `+` / OUT `−` applied after rounding),
//!   `shift-valuation.js:13-14` (row value `current × rate`, rate `toFixed(6)`).
//!   v3 does the same on exact decimals through `xc_kernel::money::{typed_decimal,
//!   round_cents, fixed2, fixed6}` (CA7): a half cent goes up.
//!
//! Every shift is seeded through `ShiftService` (open, move, close, and a
//! transaction's delta through `apply_delta_in`) against a real PostgreSQL:
//! `TEST_DATABASE_URL` must name an existing database whose name ends in
//! `_test`. These tests fail without it; they do not skip. Each test works in a
//! schema of its own.

mod common;

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, TimeZone, Utc};
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta, fixed2, fixed6, round_cents};
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::shift::commands::close_shift::CloseShift;
use xc_shift::domain::shift::commands::move_stock::MoveStock;
use xc_shift::domain::shift::commands::open_shift::OpenShift;
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::domain::shift::value_objects::stock_delta::{Direction, StockDelta};
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http::v3::ACTIONS;
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::pages::{SessionReader, Sessions};

const URI: &str = "/api/v3/compute/shift_valuation";
const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
const VALUATION_RATE: &str = "กรอกเรทให้ครบทุกสกุลที่มียอด";
const NOT_FOUND: &str = "Not found";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

/// A request id `read_request_id` accepts: 16–60 ASCII letters, digits or `-`.
fn rid() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("rq-{}-{nanos}-{n}", std::process::id())
}

/// A fixed instant, `seconds` after 2026-10-09 03:00:00 UTC. Nothing here is
/// ordered by it: movements are read in insertion order (`seq`).
fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 9, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn cur(code: &str) -> CurrencyCode {
    CurrencyCode::parse(code).unwrap_or_else(|_| panic!("{code} is a register currency"))
}

fn money(text: &str) -> Money {
    Money::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn balances(pairs: &[(&str, &str)]) -> Balances {
    let pairs: Vec<(CurrencyCode, Money)> = pairs.iter().map(|(c, a)| (cur(c), money(a))).collect();
    Balances::of(&pairs)
}

fn d(text: &str) -> Decimal {
    Decimal::from_str_exact(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

/// `fixed2(round_cents(n × r))` — the kernel's own answer for one value, used
/// to check that each literal pinned below is what the production helpers give.
fn kernel_value(n: &str, r: &str) -> String {
    fixed2(round_cents(d(n) * d(r)).expect("fits"))
}

// ------------------------------------------------------------ database

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The shift valuation tests need a real PostgreSQL and do not skip. Point it \
                 at an existing database whose name ends in `_test`, e.g.\n\
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
        let schema = unique("v3valuation");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B02', 'B02')")
            .execute(&pool)
            .await
            .expect("insert the branches this file names into branches");
        seed_opener(&pool, "owner-1").await;
        Self { pool, schema }
    }

    fn service(&self) -> ShiftService<PgShiftRepository> {
        ShiftService::new(
            Arc::new(PgShiftRepository::new(self.pool.clone())),
            BranchTx::new(Transactor::new(self.pool.clone())),
        )
    }

    /// Opens a shift on `branch` with `opening`; answers its id.
    async fn open(&self, branch: &str, opening: &[(&str, &str)]) -> String {
        self.service()
            .open(
                &BranchId::new(branch),
                OpenShift {
                    request_id: rid(),
                    opening: balances(opening),
                },
                "owner-1",
                at(0),
            )
            .await
            .expect("open a shift")
            .id
    }

    /// One manual movement of the open shift `shift` (a `stock_movements` row).
    async fn movement(&self, branch: &str, shift: &str, code: &str, dir: Direction, amount: &str) {
        self.service()
            .move_stock(
                &BranchId::new(branch),
                MoveStock {
                    request_id: rid(),
                    shift_id: shift.to_owned(),
                    currency: cur(code),
                    direction: dir,
                    amount: money(amount),
                    reason: "ทดสอบ".to_owned(),
                },
                "owner-1",
                at(1),
            )
            .await
            .expect("move stock");
    }

    /// A transaction's effect on the open shift's expected balance, through
    /// `ShiftService::apply_delta_in` (no `stock_movements` row). `minor` is
    /// signed hundredths.
    async fn trade(&self, branch: &str, pairs: &[(&str, i128)]) {
        let pairs: Vec<(CurrencyCode, MoneyDelta)> = pairs
            .iter()
            .map(|(c, m)| (cur(c), MoneyDelta::from_minor(*m)))
            .collect();
        let delta = StockDelta::of(&pairs);
        let service = self.service();
        let b = BranchId::new(branch);
        let on = b.clone();
        let branch_tx = service.branch_tx().clone();
        branch_tx
            .run(&on, move |tx| {
                Box::pin(async move { service.apply_delta_in(tx, &b, &delta).await })
            })
            .await
            .expect("apply a transaction's delta");
    }

    /// Closes the open shift `shift`, claiming `expected` and counting `counted`.
    async fn close(
        &self,
        branch: &str,
        shift: &str,
        expected: Value,
        counted: &[(&str, &str)],
        note: &str,
    ) {
        self.service()
            .close(
                &BranchId::new(branch),
                CloseShift {
                    request_id: rid(),
                    shift_id: shift.to_owned(),
                    counted: balances(counted),
                    claimed_expected: expected,
                    note: note.to_owned(),
                },
                "owner-1",
                at(2),
            )
            .await
            .expect("close the shift");
    }

    /// Every `branch_versions` row, in branch order.
    async fn version_rows(&self) -> Vec<(String, i64)> {
        sqlx::query_as::<_, (String, i64)>(
            "SELECT branch_id, version FROM branch_versions ORDER BY branch_id",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read branch_versions")
    }

    /// Every row the action reads, as text, in a fixed order.
    async fn shift_rows(&self) -> Vec<String> {
        sqlx::query_scalar::<_, String>(
            "SELECT 's ' || id || ' ' || status || ' ' || coalesce(close_note, '') FROM shifts \
             UNION ALL \
             SELECT 'b ' || shift_id || ' ' || code || ' ' || opening::text || ' ' || expected::text \
                    || ' ' || coalesce(counted::text, '-') || ' ' || coalesce(difference::text, '-') \
               FROM shift_balances \
             UNION ALL \
             SELECT 'm ' || id || ' ' || shift_id || ' ' || code || ' ' || direction || ' ' || amount::text \
               FROM stock_movements \
             ORDER BY 1",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read the shift tables")
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

// ------------------------------------------------------------ callers

/// Who is signed in, from the request header `x-test-as`: `owner`,
/// `staff:<ROLE>:<branch>`, `owner+staff:<ROLE>:<branch>`, or absent.
struct HeaderSessions;

fn staff_of(spec: &str) -> Option<Actor> {
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = Role::parse(parts.next().expect("a role")).expect("a known role");
    let branch = parts.next().expect("a branch");
    Actor::staff("employee-7", role, BranchId::new(branch))
}

fn sessions_of(spec: &str) -> Sessions {
    if spec.is_empty() {
        return Sessions {
            owner: None,
            staff: None,
        };
    }
    if spec == "owner" {
        return Sessions {
            owner: Some(Actor::owner("owner-1")),
            staff: None,
        };
    }
    if let Some(staff) = spec.strip_prefix("owner+") {
        return Sessions {
            owner: Some(Actor::owner("owner-1")),
            staff: staff_of(staff),
        };
    }
    Sessions {
        owner: None,
        staff: staff_of(spec),
    }
}

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Sessions> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { sessions_of(&spec) })
    }
}

const STAFF_ROLES: [&str; 9] = [
    "FRONT",
    "BACKEND",
    "MANAGER",
    "RATES",
    "MONITOR",
    "FINANCE",
    "SYSTEM",
    "DOCUMENTS",
    "DOCUMENT_REVIEW",
];

fn every_caller() -> Vec<String> {
    let mut out = vec![String::new(), "owner".to_string()];
    for role in STAFF_ROLES {
        out.push(format!("staff:{role}:XC-B01"));
        out.push(format!("owner+staff:{role}:XC-B01"));
    }
    out
}

// ------------------------------------------------------------ requests

#[derive(Debug)]
struct Answer {
    status: StatusCode,
    cache_control: Option<String>,
    text: String,
    body: Value,
}

/// `POST /api/v3/compute/shift_valuation` as `who`, from the same origin, to
/// the whole application (`bootstrap::configure` — the generated `ACTIONS`).
async fn post(db: &TestDb, who: &str, body: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::post()
        .uri(URI)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"));
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    let resp = atest::call_service(&app, req.set_payload(body.to_string()).to_request()).await;
    let status = resp.status();
    let cache_control = resp
        .headers()
        .get(header::CACHE_CONTROL)
        .map(|v| v.to_str().expect("text").to_owned());
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer {
        status,
        cache_control,
        text,
        body,
    }
}

/// The owner, on branch XC-B01.
async fn owner(db: &TestDb, mut body: Value) -> Answer {
    body["branch"] = json!("XC-B01");
    post(db, "owner", body).await
}

fn is_answer(a: &Answer, want: Value, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: status; body {}", a.text);
    assert_eq!(a.body, want, "{what}: the whole answer");
    assert_eq!(a.cache_control.as_deref(), Some("no-store"), "{what}");
}

fn is_refused(a: &Answer, status: u16, code: &str, field: Option<&str>, message: &str, what: &str) {
    assert_eq!(a.status.as_u16(), status, "{what}: status; body {}", a.text);
    let want = match field {
        Some(f) => json!({ "error": { "code": code, "field": f, "message": message } }),
        None => json!({ "error": { "code": code, "message": message } }),
    };
    assert_eq!(a.body, want, "{what}: the refusal body");
}

fn is_valuation_rate(a: &Answer, what: &str) {
    is_refused(a, 400, "valuationRate", Some("rates"), VALUATION_RATE, what);
}

fn currency(code: &str, opening: &str, current: &str) -> Value {
    json!({ "currency": code, "opening": opening, "current": current, "needsRate": code != "THB" })
}

fn row(code: &str, rate: &str, value: &str) -> Value {
    json!({ "currency": code, "rate": rate, "value": value })
}

/// The worked example of §12.20.4: opening THB 10,000.00 + USD 100.00; a sale
/// of USD 100 for THB 3,900 cash (a transaction, no movement row); one movement
/// IN THB 500.00 → expected THB 6,600.00 + USD 200.00.
async fn worked_example(db: &TestDb) -> String {
    let shift = db.open("XC-B01", &[("THB", "10000"), ("USD", "100")]).await;
    db.trade("XC-B01", &[("USD", 10_000), ("THB", -390_000)])
        .await;
    db.movement("XC-B01", &shift, "THB", Direction::In, "500")
        .await;
    shift
}

fn worked_example_currencies() -> Value {
    json!([
        currency("THB", "10000.00", "6600.00"),
        currency("USD", "100.00", "200.00"),
    ])
}

// ------------------------------------------------------------ the route

/// #508's gap: an action file in `http/v3/actions/` is registered by
/// `configure_v3` through the generated `ACTIONS`, and reachable in the app.
#[actix_web::test]
async fn the_action_is_in_the_generated_table_and_reachable_through_the_whole_app() {
    let names: Vec<&str> = ACTIONS.iter().map(|a| a.name).collect();
    assert!(
        names.contains(&"shift_valuation"),
        "ACTIONS lists shift_valuation: {names:?}"
    );
    let db = TestDb::start().await;
    let shift = worked_example(&db).await;
    let a = owner(&db, json!({ "shiftId": shift })).await;
    is_answer(
        &a,
        json!({ "currencies": worked_example_currencies() }),
        "POST /api/v3/compute/shift_valuation",
    );
    db.finish().await;
}

// ------------------------------------------------------------ currencies only

/// Step 2: `rates` absent or `null` → `currencies` only, no `result` key.
#[actix_web::test]
async fn without_rates_the_answer_lists_the_currencies_only() {
    let db = TestDb::start().await;
    let shift = worked_example(&db).await;
    for (what, body) in [
        ("rates absent", json!({ "shiftId": shift })),
        ("rates null", json!({ "shiftId": shift, "rates": null })),
    ] {
        let a = owner(&db, body).await;
        is_answer(
            &a,
            json!({ "currencies": worked_example_currencies() }),
            what,
        );
        assert!(a.body.get("result").is_none(), "{what}: no result key");
    }
    db.finish().await;
}

// ------------------------------------------------------------ the figures

/// §12.20.4's worked example, exactly.
#[actix_web::test]
async fn the_worked_example_values_the_shift_at_the_typed_rate() {
    let db = TestDb::start().await;
    let shift = worked_example(&db).await;
    let a = owner(&db, json!({ "shiftId": shift, "rates": { "USD": "34.5" } })).await;
    is_answer(
        &a,
        json!({
            "currencies": worked_example_currencies(),
            "result": {
                "rows": [
                    row("THB", "1.000000", "6600.00"),
                    row("USD", "34.500000", "6900.00"),
                ],
                "opening": "13450.00",
                "current": "13500.00",
                "movements": "500.00",
                "difference": "50.00",
                "adjusted": "-450.00",
            }
        }),
        "worked example",
    );
    db.finish().await;
}

/// Steps 4–5 on real half-cent midpoints, where a half-up rule (v2's
/// `Math.round`, `round_cents`) and a half-even rule differ, and where rounding
/// each currency / each movement differs from rounding a sum or a net:
///
/// opening THB 100.00, USD 1.25, EUR 1.25, GBP 2.25; movements IN USD 1.25
/// four times and OUT GBP 1.00; a sale of all the EUR for THB 2.00 (a
/// transaction, no movement row) → expected THB 102.00, USD 6.25, EUR 0.00,
/// GBP 1.25; rates USD 0.5, EUR 0.1, GBP 0.125.
///
/// - opening = 100.00 + r(0.625)=0.63 + r(0.125)=0.13 + r(0.28125)=0.28
///   = 101.04 (half-even 101.02; rounding the sum 101.03)
/// - current = 102.00 + r(3.125)=3.13 + r(0.15625)=0.16 = 105.29
///   (half-even 105.28; rounding the sum 105.28)
/// - movements = 4 × r(0.625)=0.63 − r(0.125)=0.13 = 2.39 (one net per
///   currency 2.38; negating before rounding, −0.125 → −0.12, 2.40;
///   half-even 2.36)
/// - difference = 4.25; adjusted = 1.86
/// - rows THB, EUR, GBP, USD (A–Z, not the register's THB, USD, EUR, GBP);
///   EUR is listed by its opening alone (expected zero, no movement) and its
///   row is 0.00
#[actix_web::test]
async fn each_value_is_rounded_half_up_per_currency_and_per_movement_on_exact_decimals() {
    // the literals below are the kernel's own answers
    for (n, r, want) in [
        ("1.25", "0.5", "0.63"),
        ("1.25", "0.1", "0.13"),
        ("2.25", "0.125", "0.28"),
        ("6.25", "0.5", "3.13"),
        ("1.25", "0.125", "0.16"),
        ("1", "0.125", "0.13"),
        ("0", "0.1", "0.00"),
        ("102", "1", "102.00"),
    ] {
        assert_eq!(kernel_value(n, r), want, "round_cents({n} × {r})");
    }
    assert_eq!(fixed6(d("0.5")), "0.500000");
    assert_eq!(fixed6(d("0.1")), "0.100000");
    assert_eq!(fixed6(d("0.125")), "0.125000");

    let db = TestDb::start().await;
    let shift = db
        .open(
            "XC-B01",
            &[
                ("THB", "100"),
                ("USD", "1.25"),
                ("EUR", "1.25"),
                ("GBP", "2.25"),
            ],
        )
        .await;
    for _ in 0..4 {
        db.movement("XC-B01", &shift, "USD", Direction::In, "1.25")
            .await;
    }
    db.movement("XC-B01", &shift, "GBP", Direction::Out, "1")
        .await;
    db.trade("XC-B01", &[("EUR", -125), ("THB", 200)]).await;
    let a = owner(
        &db,
        json!({ "shiftId": shift, "rates": { "USD": "0.5", "EUR": "0.1", "GBP": "0.125" } }),
    )
    .await;
    is_answer(
        &a,
        json!({
            "currencies": [
                currency("THB", "100.00", "102.00"),
                currency("EUR", "1.25", "0.00"),
                currency("GBP", "2.25", "1.25"),
                currency("USD", "1.25", "6.25"),
            ],
            "result": {
                "rows": [
                    row("THB", "1.000000", "102.00"),
                    row("EUR", "0.100000", "0.00"),
                    row("GBP", "0.125000", "0.16"),
                    row("USD", "0.500000", "3.13"),
                ],
                "opening": "101.04",
                "current": "105.29",
                "movements": "2.39",
                "difference": "4.25",
                "adjusted": "1.86",
            }
        }),
        "midpoints",
    );
    db.finish().await;
}

/// Step 5 and V6: the rate is used exactly (not its six-place form) and shown
/// `toFixed(6)`, a half going up. USD 100,000.00 × 32.123456789 =
/// 3,212,345.6789 → 3212345.68 (the six-place rate would give 3212345.70);
/// EUR 1.00 × 36.1234565 → rate "36.123457" (half-even "36.123456").
#[actix_web::test]
async fn a_rate_with_many_places_is_used_exactly_and_shown_to_six_places() {
    assert_eq!(kernel_value("100000", "32.123456789"), "3212345.68");
    assert_eq!(kernel_value("100000", "32.123457"), "3212345.70");
    assert_eq!(fixed6(d("32.123456789")), "32.123457");
    assert_eq!(fixed6(d("36.1234565")), "36.123457");
    assert_eq!(kernel_value("1", "36.1234565"), "36.12");

    let db = TestDb::start().await;
    let shift = db.open("XC-B01", &[("USD", "100000"), ("EUR", "1")]).await;
    let a = owner(
        &db,
        json!({ "shiftId": shift, "rates": { "USD": "32.123456789", "EUR": "36.1234565" } }),
    )
    .await;
    is_answer(
        &a,
        json!({
            "currencies": [
                currency("EUR", "1.00", "1.00"),
                currency("USD", "100000.00", "100000.00"),
            ],
            "result": {
                "rows": [
                    row("EUR", "36.123457", "36.12"),
                    row("USD", "32.123457", "3212345.68"),
                ],
                "opening": "3212381.80",
                "current": "3212381.80",
                "movements": "0.00",
                "difference": "0.00",
                "adjusted": "0.00",
            }
        }),
        "many places",
    );
    db.finish().await;
}

/// The bounds `0.000001 ≤ r ≤ 1,000,000` are inclusive.
#[actix_web::test]
async fn the_rate_bounds_are_inclusive() {
    let db = TestDb::start().await;
    let shift = db.open("XC-B01", &[("USD", "1"), ("JPY", "2")]).await;
    let a = owner(
        &db,
        json!({ "shiftId": shift, "rates": { "USD": "1000000", "JPY": "0.000001" } }),
    )
    .await;
    is_answer(
        &a,
        json!({
            "currencies": [
                currency("JPY", "2.00", "2.00"),
                currency("USD", "1.00", "1.00"),
            ],
            "result": {
                "rows": [
                    row("JPY", "0.000001", "0.00"),
                    row("USD", "1000000.000000", "1000000.00"),
                ],
                "opening": "1000000.00",
                "current": "1000000.00",
                "movements": "0.00",
                "difference": "0.00",
                "adjusted": "0.00",
            }
        }),
        "at both bounds",
    );
    db.finish().await;
}

// ------------------------------------------------------------ which currencies

/// Step 1 and §12.20.9: a shift holding USD, AUD and EUR answers THB, AUD, EUR,
/// USD, where `CurrencyCode::all()` would give THB, USD, EUR, AUD. Each is
/// listed for a different reason: USD by its opening, AUD by its expected
/// balance only (a transaction), EUR by a movement.
#[actix_web::test]
async fn currencies_and_rows_are_thb_first_then_a_to_z_not_register_order() {
    let db = TestDb::start().await;
    let shift = db.open("XC-B01", &[("THB", "1000"), ("USD", "10")]).await;
    db.trade("XC-B01", &[("AUD", 2_000), ("THB", -40_000)])
        .await;
    db.movement("XC-B01", &shift, "EUR", Direction::In, "5")
        .await;
    let want_currencies = json!([
        currency("THB", "1000.00", "600.00"),
        currency("AUD", "0.00", "20.00"),
        currency("EUR", "0.00", "5.00"),
        currency("USD", "10.00", "10.00"),
    ]);
    let a = owner(&db, json!({ "shiftId": shift })).await;
    is_answer(
        &a,
        json!({ "currencies": want_currencies.clone() }),
        "listed",
    );
    let a = owner(
        &db,
        json!({ "shiftId": shift, "rates": { "USD": "30", "AUD": "20", "EUR": "40" } }),
    )
    .await;
    is_answer(
        &a,
        json!({
            "currencies": want_currencies,
            "result": {
                "rows": [
                    row("THB", "1.000000", "600.00"),
                    row("AUD", "20.000000", "400.00"),
                    row("EUR", "40.000000", "200.00"),
                    row("USD", "30.000000", "300.00"),
                ],
                "opening": "1300.00",
                "current": "1500.00",
                "movements": "200.00",
                "difference": "200.00",
                "adjusted": "0.00",
            }
        }),
        "valued",
    );
    db.finish().await;
}

/// §9: a currency moved in and out back to zero (`o_c = e_c = 0`) is listed,
/// needs its rate, and has a row of `"0.00"`. THB, all zero and unmoved, is not
/// listed. GBP before USD: A–Z, where the register has USD first.
#[actix_web::test]
async fn a_currency_moved_in_and_out_back_to_zero_is_listed_and_needs_its_rate() {
    let db = TestDb::start().await;
    let shift = db.open("XC-B01", &[("USD", "5")]).await;
    db.movement("XC-B01", &shift, "GBP", Direction::In, "10.01")
        .await;
    db.movement("XC-B01", &shift, "GBP", Direction::Out, "10.01")
        .await;
    let want_currencies = json!([
        currency("GBP", "0.00", "0.00"),
        currency("USD", "5.00", "5.00"),
    ]);
    let a = owner(&db, json!({ "shiftId": shift })).await;
    is_answer(
        &a,
        json!({ "currencies": want_currencies.clone() }),
        "listed",
    );
    let a = owner(&db, json!({ "shiftId": shift, "rates": { "USD": "30" } })).await;
    is_valuation_rate(&a, "GBP's rate is needed");
    let a = owner(
        &db,
        json!({ "shiftId": shift, "rates": { "USD": "30", "GBP": "40.5" } }),
    )
    .await;
    is_answer(
        &a,
        json!({
            "currencies": want_currencies,
            "result": {
                "rows": [
                    row("GBP", "40.500000", "0.00"),
                    row("USD", "30.000000", "150.00"),
                ],
                "opening": "150.00",
                "current": "150.00",
                "movements": "0.00",
                "difference": "0.00",
                "adjusted": "0.00",
            }
        }),
        "valued",
    );
    db.finish().await;
}

/// §9: `rates: {}` on a shift that needs none answers an empty result of zeros.
#[actix_web::test]
async fn a_shift_that_needs_no_rate_answers_zero_figures_for_empty_rates() {
    let db = TestDb::start().await;
    let shift = db.open("XC-B01", &[]).await;
    let a = owner(&db, json!({ "shiftId": shift })).await;
    is_answer(&a, json!({ "currencies": [] }), "listed");
    let a = owner(&db, json!({ "shiftId": shift, "rates": {} })).await;
    is_answer(
        &a,
        json!({
            "currencies": [],
            "result": {
                "rows": [],
                "opening": "0.00",
                "current": "0.00",
                "movements": "0.00",
                "difference": "0.00",
                "adjusted": "0.00",
            }
        }),
        "valued",
    );
    db.finish().await;
}

// ------------------------------------------------------------ open and closed shifts

/// §9: an open shift is valued on its expected balance at the time of each call.
#[actix_web::test]
async fn an_open_shift_is_valued_on_its_expected_balance_at_the_time_of_the_call() {
    let db = TestDb::start().await;
    let shift = db.open("XC-B01", &[("USD", "10")]).await;
    let rates = json!({ "USD": "30" });
    let a = owner(&db, json!({ "shiftId": shift, "rates": rates.clone() })).await;
    is_answer(
        &a,
        json!({
            "currencies": [currency("USD", "10.00", "10.00")],
            "result": {
                "rows": [row("USD", "30.000000", "300.00")],
                "opening": "300.00",
                "current": "300.00",
                "movements": "0.00",
                "difference": "0.00",
                "adjusted": "0.00",
            }
        }),
        "before the movement",
    );
    db.movement("XC-B01", &shift, "USD", Direction::Out, "4")
        .await;
    db.trade("XC-B01", &[("USD", -100), ("THB", 3_500)]).await;
    let a = owner(&db, json!({ "shiftId": shift, "rates": rates })).await;
    is_answer(
        &a,
        json!({
            "currencies": [
                currency("THB", "0.00", "35.00"),
                currency("USD", "10.00", "5.00"),
            ],
            "result": {
                "rows": [
                    row("THB", "1.000000", "35.00"),
                    row("USD", "30.000000", "150.00"),
                ],
                "opening": "300.00",
                "current": "185.00",
                "movements": "-120.00",
                "difference": "-115.00",
                "adjusted": "5.00",
            }
        }),
        "after a movement and a trade",
    );
    db.finish().await;
}

/// A closed shift is valued on its stored expected balance (not on what was
/// counted) and on its own movements only (not the next shift's).
#[actix_web::test]
async fn a_closed_shift_is_valued_on_its_stored_expected_balance_and_its_own_movements() {
    let db = TestDb::start().await;
    let first = db.open("XC-B01", &[("THB", "1000"), ("USD", "10")]).await;
    db.movement("XC-B01", &first, "USD", Direction::In, "2")
        .await;
    db.close(
        "XC-B01",
        &first,
        json!({ "THB": "1000.00", "USD": "12.00" }),
        &[("THB", "990"), ("USD", "11")],
        "นับขาด",
    )
    .await;
    let second = db.open("XC-B01", &[("THB", "990"), ("USD", "11")]).await;
    db.movement("XC-B01", &second, "USD", Direction::In, "50")
        .await;
    db.movement("XC-B01", &second, "CHF", Direction::In, "3")
        .await;

    let a = owner(&db, json!({ "shiftId": first, "rates": { "USD": "30" } })).await;
    is_answer(
        &a,
        json!({
            "currencies": [
                currency("THB", "1000.00", "1000.00"),
                currency("USD", "10.00", "12.00"),
            ],
            "result": {
                "rows": [
                    row("THB", "1.000000", "1000.00"),
                    row("USD", "30.000000", "360.00"),
                ],
                "opening": "1300.00",
                "current": "1360.00",
                "movements": "60.00",
                "difference": "60.00",
                "adjusted": "0.00",
            }
        }),
        "the closed shift",
    );
    db.finish().await;
}

// ------------------------------------------------------------ refusals

/// §8, §9: a needed rate absent, empty, not a typed decimal, `≤ 0`, below
/// `0.000001` or above `1,000,000` → `400 valuationRate` on `rates`.
#[actix_web::test]
async fn a_needed_rate_absent_ill_typed_or_out_of_range_is_refused() {
    let db = TestDb::start().await;
    let shift = db
        .open("XC-B01", &[("THB", "100"), ("USD", "10"), ("EUR", "10")])
        .await;
    let mut cases: Vec<(String, Value)> = vec![
        ("rates {}".into(), json!({})),
        ("USD absent".into(), json!({ "EUR": "40" })),
        ("EUR absent".into(), json!({ "USD": "30" })),
        ("only THB".into(), json!({ "THB": "1" })),
        ("lower-case key".into(), json!({ "usd": "30", "EUR": "40" })),
    ];
    for bad in [
        "",
        "abc",
        "1e3",
        "1E3",
        " 34",
        "34 ",
        "+34",
        "34,5",
        "34.",
        "0x10",
        "NaN",
        "Infinity",
        "0",
        "0.00",
        "-0",
        "-1",
        "-34.5",
        "0.0000009",
        "0.0000001",
        "1000000.000001",
        "1000001",
        "79228162514264337593543950336",
    ] {
        cases.push((format!("USD {bad:?}"), json!({ "USD": bad, "EUR": "40" })));
    }
    for (what, rates) in cases {
        let a = owner(&db, json!({ "shiftId": shift, "rates": rates })).await;
        is_valuation_rate(&a, &what);
    }
    db.finish().await;
}

/// Decided on #509 (lane controller, after claude/32ef39's verdict on PR 507,
/// issuecomment-6066723647): a whitespace-only rate is refused exactly like an
/// empty one. v2's `Number(' ') === 0` is an accident, not served behaviour.
#[actix_web::test]
async fn a_whitespace_only_rate_is_refused_like_an_empty_one() {
    let db = TestDb::start().await;
    let shift = db.open("XC-B01", &[("USD", "10")]).await;
    let empty = owner(&db, json!({ "shiftId": shift, "rates": { "USD": "" } })).await;
    is_valuation_rate(&empty, "empty");
    for blank in [" ", "   ", "\t", "\n", " \t "] {
        let a = owner(&db, json!({ "shiftId": shift, "rates": { "USD": blank } })).await;
        is_valuation_rate(&a, &format!("{blank:?}"));
        assert_eq!(a.body, empty.body, "{blank:?}: exactly like empty");
    }
    db.finish().await;
}

/// §8: `shiftId` absent or `""` → `400 invalidRequest` on `shiftId`, whether
/// or not rates are sent.
#[actix_web::test]
async fn a_missing_or_empty_shift_id_is_an_invalid_request_on_the_field_shift_id() {
    let db = TestDb::start().await;
    let _ = worked_example(&db).await;
    for (what, body) in [
        ("empty body", json!({})),
        ("shiftId empty", json!({ "shiftId": "" })),
        ("rates only", json!({ "rates": { "USD": "34.5" } })),
        (
            "shiftId empty, rates",
            json!({ "shiftId": "", "rates": { "USD": "34.5" } }),
        ),
    ] {
        let a = owner(&db, body).await;
        is_refused(
            &a,
            400,
            "invalidRequest",
            Some("shiftId"),
            BAD_REQUEST,
            what,
        );
    }
    db.finish().await;
}

/// §8: an unknown shift, or a shift of another branch, is `404 notFound` on
/// `shiftId` — with the policy's own sentence for that code.
#[actix_web::test]
async fn an_unknown_shift_or_a_shift_of_another_branch_is_not_found() {
    let not_found =
        V3Error::from(xc_auth::domain::policy::value_objects::policy_error::PolicyError::NotFound);
    assert_eq!(not_found.code, "notFound");

    let db = TestDb::start().await;
    let b01 = worked_example(&db).await;
    let b02 = db.open("XC-B02", &[("USD", "5")]).await;
    let calls = [
        (
            "unknown id",
            "owner",
            json!({ "shiftId": "SH-nope", "branch": "XC-B01" }),
        ),
        (
            "unknown id, rates",
            "owner",
            json!({ "shiftId": "SH-nope", "branch": "XC-B01", "rates": { "USD": "34.5" } }),
        ),
        (
            "owner on XC-B02 naming XC-B01's shift",
            "owner",
            json!({ "shiftId": b01, "branch": "XC-B02", "rates": { "USD": "34.5" } }),
        ),
        (
            "owner on MAIN naming XC-B01's shift",
            "owner",
            json!({ "shiftId": b01 }),
        ),
        (
            "owner on XC-B01 naming XC-B02's shift",
            "owner",
            json!({ "shiftId": b02, "branch": "XC-B01" }),
        ),
        (
            "staff of XC-B02 naming XC-B01's shift",
            "staff:FRONT:XC-B02",
            json!({ "shiftId": b01 }),
        ),
        (
            "staff of XC-B01 naming XC-B02's shift",
            "staff:FRONT:XC-B01",
            json!({ "shiftId": b02, "rates": { "USD": "30" } }),
        ),
    ];
    for (what, who, body) in calls {
        let a = post(&db, who, body).await;
        is_refused(&a, 404, "notFound", Some("shiftId"), NOT_FOUND, what);
    }
    // OR-4b (#767, D-4, §02 "Server ตรวจ … branch scope"): staff naming another branch are refused
    // that branch before the shift is looked for
    let a = post(
        &db,
        "staff:MANAGER:XC-B02",
        json!({ "shiftId": b01, "branch": "XC-B01", "rates": { "USD": "34.5" } }),
    )
    .await;
    is_refused(
        &a,
        403,
        "outOfScope",
        None,
        "สาขานี้อยู่นอกขอบเขตของบัญชีนี้",
        "staff of XC-B02 naming XC-B01's shift and branch",
    );

    // the same shifts, asked for on their own branch, answer
    let a = post(&db, "staff:FRONT:XC-B02", json!({ "shiftId": b02 })).await;
    is_answer(
        &a,
        json!({ "currencies": [currency("USD", "5.00", "5.00")] }),
        "staff of XC-B02, own shift",
    );
    let a = post(&db, "staff:FRONT:XC-B01", json!({ "shiftId": b01 })).await;
    is_answer(
        &a,
        json!({ "currencies": worked_example_currencies() }),
        "staff of XC-B01, own shift",
    );
    db.finish().await;
}

/// §8: `rates` that is not an object of strings is the seam's step 6 —
/// `400 invalidRequest` without a field.
#[actix_web::test]
async fn rates_that_are_not_an_object_of_strings_are_an_invalid_request_without_a_field() {
    let db = TestDb::start().await;
    let shift = worked_example(&db).await;
    for (what, rates) in [
        ("a number rate", json!({ "USD": 34.5 })),
        ("a null rate", json!({ "USD": null })),
        ("an object rate", json!({ "USD": { "v": "34.5" } })),
        ("a string", json!("34.5")),
        ("an array", json!(["34.5"])),
        ("true", json!(true)),
        ("a number", json!(34.5)),
    ] {
        let a = owner(&db, json!({ "shiftId": shift, "rates": rates })).await;
        is_refused(&a, 400, "invalidRequest", None, BAD_REQUEST, what);
    }
    db.finish().await;
}

// ------------------------------------------------------------ writes nothing

/// CA3: no call — answered, refused or not found — moves a branch version or
/// changes a shift row.
#[actix_web::test]
async fn a_call_never_moves_a_branch_version_or_any_shift_row() {
    let db = TestDb::start().await;
    let shift = worked_example(&db).await;
    let other = db.open("XC-B02", &[("USD", "5")]).await;
    let versions_before = db.version_rows().await;
    let rows_before = db.shift_rows().await;
    assert!(
        versions_before.iter().any(|(b, v)| b == "XC-B01" && *v > 0),
        "the seeding moved XC-B01's version: {versions_before:?}"
    );

    for _ in 0..2 {
        for (who, body, status) in [
            (
                "owner",
                json!({ "shiftId": shift, "branch": "XC-B01" }),
                200,
            ),
            (
                "owner",
                json!({ "shiftId": shift, "branch": "XC-B01", "rates": { "USD": "34.5" } }),
                200,
            ),
            (
                "staff:FRONT:XC-B01",
                json!({ "shiftId": shift, "rates": { "USD": "34.5" } }),
                200,
            ),
            (
                "staff:FRONT:XC-B02",
                json!({ "shiftId": other, "rates": {} }),
                400,
            ),
            (
                "owner",
                json!({ "shiftId": shift, "branch": "XC-B01", "rates": { "USD": "" } }),
                400,
            ),
            ("owner", json!({ "shiftId": "", "branch": "XC-B01" }), 400),
            (
                "owner",
                json!({ "shiftId": other, "branch": "XC-B01" }),
                404,
            ),
        ] {
            let a = post(&db, who, body).await;
            assert_eq!(a.status.as_u16(), status, "{who}: body {}", a.text);
        }
    }

    assert_eq!(
        db.version_rows().await,
        versions_before,
        "branch_versions after the calls: no version moved, no row added (CA3)"
    );
    assert_eq!(
        db.shift_rows().await,
        rows_before,
        "shifts, shift_balances and stock_movements after the calls"
    );
    db.finish().await;
}

// ------------------------------------------------------------ who may call

/// CA4: `INHERITS = (Rpc, "adminSnapshot")` — the endpoint refuses exactly whom
/// `Policy::allow` refuses for that pair, and answers everyone else on their
/// own branch.
#[actix_web::test]
async fn the_action_answers_exactly_whom_the_policy_allows_for_admin_snapshot() {
    let db = TestDb::start().await;
    let shift = worked_example(&db).await;
    let mut allowed = 0;
    let mut refused = 0;
    for who in every_caller() {
        let a = post(&db, &who, json!({ "shiftId": shift, "branch": "XC-B01" })).await;
        let s = sessions_of(&who);
        let actor = s.owner.or(s.staff);
        match Policy::allow(
            actor.as_ref(),
            EntryPath::Rpc,
            &Action::new("adminSnapshot"),
        ) {
            Ok(()) => {
                allowed += 1;
                is_answer(
                    &a,
                    json!({ "currencies": worked_example_currencies() }),
                    &format!("{who:?}: allowed"),
                );
            }
            Err(e) => {
                refused += 1;
                let status = e.status();
                let message = e.to_string();
                let code = V3Error::from(e).code;
                is_refused(&a, status, code, None, &message, &format!("{who:?}"));
            }
        }
    }
    assert_eq!(
        allowed, 19,
        "the owner and every staff role, alone or with the owner"
    );
    assert_eq!(refused, 1, "nobody signed in");
    db.finish().await;
}

// ------------------------------------------------------------ the generated types

/// §5.1, W7: the committed TypeScript carries the wire names, money and rates
/// as `Money` / `Rate`, `rates` and `result` optional, and no `branch` field
/// (#508's reserved key).
#[test]
fn the_generated_typescript_carries_the_wire_names() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../web/src/shared/api/generated/shift_valuation.ts");
    let ts =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    for needle in [
        "shiftId: string",
        "rates?: ",
        "currencies: Array<",
        "result?: ",
        "currency: string",
        "opening: Money",
        "current: Money",
        "needsRate: boolean",
        "rows: Array<",
        "movements: Money",
        "difference: Money",
        "adjusted: Money",
        "rate: Rate",
        "value: Money",
    ] {
        assert!(
            ts.contains(needle),
            "shift_valuation.ts has {needle:?}:\n{ts}"
        );
    }
    assert!(!ts.contains("branch"), "no field named branch:\n{ts}");
    assert!(
        !ts.contains("number"),
        "money and rates are strings (W7):\n{ts}"
    );
}

// ------------------------------------------------------------ implementer additions

/// A whitespace-only `shiftId` (" ") is `400 invalidRequest` on the field `shiftId`,
/// exactly like an empty one.
#[actix_web::test]
async fn a_whitespace_only_shift_id_is_an_invalid_request_on_the_field_shift_id() {
    let db = TestDb::start().await;
    let _ = worked_example(&db).await;
    for blank in [" ", "   ", "\t", "\n", " \t "] {
        for (what, body) in [
            (format!("shiftId {blank:?}"), json!({ "shiftId": blank })),
            (
                format!("shiftId {blank:?}, rates"),
                json!({ "shiftId": blank, "rates": { "USD": "34.5" } }),
            ),
        ] {
            let a = owner(&db, body).await;
            is_refused(
                &a,
                400,
                "invalidRequest",
                Some("shiftId"),
                BAD_REQUEST,
                &what,
            );
        }
    }
    db.finish().await;
}

/// M35: The THB rate is fixed at 1 and is never taken from `rates`.
/// Passing a THB rate in `rates` is ignored and does not alter THB's rate (fixed at "1.000000")
/// or its valued figures, and unknown keys like "XYZ" are likewise ignored.
#[actix_web::test]
async fn a_thb_rate_in_rates_is_ignored_and_fixed_at_one() {
    let db = TestDb::start().await;
    let shift = worked_example(&db).await;
    for (what, rates) in [
        ("thb 99.0", json!({ "USD": "34.5", "THB": "99.0" })),
        ("thb 0.5", json!({ "USD": "34.5", "THB": "0.5" })),
        (
            "thb 0 and extra key XYZ",
            json!({ "USD": "34.5", "THB": "0", "XYZ": "123" }),
        ),
    ] {
        let a = owner(&db, json!({ "shiftId": shift, "rates": rates })).await;
        is_answer(
            &a,
            json!({
                "currencies": worked_example_currencies(),
                "result": {
                    "rows": [
                        row("THB", "1.000000", "6600.00"),
                        row("USD", "34.500000", "6900.00"),
                    ],
                    "opening": "13450.00",
                    "current": "13500.00",
                    "movements": "500.00",
                    "difference": "50.00",
                    "adjusted": "-450.00",
                }
            }),
            what,
        );
    }
    db.finish().await;
}

/// CA-1 (#617): a shift's opener must name a user (`shifts.teller_id`, design #582 §8 M1 item 3). The user an
/// opener id resolves to: `users.legacy_employee_id = id`, or, for `"owner"`, the organisation-wide EXECUTIVE +
/// ADMIN user. Seeded once per id; no test's expectation depends on it. Added by the ticket author in CA-1's
/// up-front commit.
async fn seed_opener(pool: &PgPool, id: &str) {
    if id == "owner" {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users u WHERE u.legacy_employee_id IS NULL AND EXISTS \
             (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL))",
        )
        .fetch_one(pool)
        .await
        .expect("look for the owner's user");
        if !exists {
            sqlx::query(
                "WITH o AS (SELECT id FROM organizations WHERE code = 'XC'), \
                 u AS (INSERT INTO users (organization_id, name, status, created_by) \
                       SELECT id, 'owner', 'PENDING', 'test' FROM o RETURNING id, organization_id) \
                 INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
                 SELECT u.id, r.code, u.organization_id, NULL FROM u, (VALUES ('EXECUTIVE'), ('ADMIN')) AS r(code)",
            )
            .execute(pool)
            .await
            .expect("seed the owner's user");
        }
        return;
    }
    sqlx::query(
        "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
         SELECT id, $1, 'PENDING', 'test', $1 FROM organizations WHERE code = 'XC' \
         ON CONFLICT (legacy_employee_id) DO NOTHING",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("seed an opener's user");
}
