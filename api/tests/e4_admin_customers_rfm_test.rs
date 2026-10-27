//! E-4 (feat/be-gaps-b; design `docs/design/blueprint/executive-finance.md` §3 row `rfm`, E-4 "rolling 12 months",
//! and the E-4 decision recorded there): every `admin_customers` row also carries R, F and THB value over the 12
//! rolling months before the read: `rfmLastAt`, `rfmCompleted`, `rfmVolume`.
//!
//! Written before the implementation by the sub-controller (claude/2ef7d5a6). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! [required] by these tests:
//! - The window is `transactions.recorded_at >= now() - interval '12 months'` (the database clock at the read), the
//!   same lookback `PgCustomerMetrics` uses for segments (`infrastructure/marketing/pg_customer_metrics.rs:72`).
//! - The same rows count as for `completed`/`volume`/`lastAt` today: LIVE, `status = 'COMPLETED'`, at the branch the
//!   view is read at; `rfmVolume` is Σ `transaction_lines.thb_amount` as a 2-decimal string, `"0.00"` when none;
//!   `rfmLastAt` is the newest such `recorded_at` in the window (RFC 3339, milliseconds, `Z`) or `null`.
//! - `completed`, `volume`, `lastAt` stay all-time, unchanged; the order and paging of the view are unchanged.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

struct Owner;

impl SessionReader for Owner {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let signed_in = req.headers().get(AS).is_some();
        Box::pin(async move {
            let user = signed_in.then(|| test_callers::owner("owner"));
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

struct Db {
    pool: PgPool,
}

impl Db {
    async fn start() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip."
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("e4rfm");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(16)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        Migrator::new(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("migrations")
                .as_path(),
        )
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("apply api/migrations");
        let db = Db { pool };
        db.exec("INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN'), ('B2', 'B2') ON CONFLICT (id) DO NOTHING")
            .await;
        for branch in ["MAIN", "B2"] {
            db.exec(&format!(
                "WITH u AS (INSERT INTO users (organization_id, name, status, created_by) \
                     VALUES ('{ORG}'::uuid, 'Teller {branch}', 'ACTIVE', 'test') RETURNING id) \
                 INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
                 SELECT 'SH-{branch}', '{branch}', 'OPEN', now(), 'test', 'req-open-{branch}', u.id FROM u"
            ))
            .await;
        }
        db
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn customer(&self, name: &str) -> String {
        let id = unique("CX");
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('{id}', '{ORG}', '{id}', '{name}', 'LIVE', now() - interval '3 years')"
        ))
        .await;
        id
    }

    /// One exchange of `customer` at `branch`, recorded at `at`, worth `thb` THB.
    async fn exchange(
        &self,
        customer: &str,
        branch: &str,
        at: DateTime<Utc>,
        thb: &str,
        mode: &str,
        status: &str,
    ) {
        let id = uuid::Uuid::new_v4().to_string();
        let when = format!(
            "'{}'::timestamptz",
            at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        );
        let completed = if status == "COMPLETED" {
            when.clone()
        } else {
            "NULL".to_owned()
        };
        self.exec(&format!(
            "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
                customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, \
                recorded_at, recorded_by, status, completed_at, organization_id) \
             VALUES ('{id}', 'DOC-{id}', '{branch}', '{mode}', 'SH-{branch}', 'sub-{id}', 'hash-{id}', \
                '{customer}', 'Customer', 'THA', 'teller-1', '', '', ({when})::date, \
                {when}, {when}, 'teller-1', '{status}', {completed}, '{ORG}')"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
                quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
             VALUES (gen_random_uuid(), '{ORG}', '{id}', 'BUY', 'USD', 100, 1, 33.5, {thb}, 0, {thb}, 1, \
                {when}, {when})"
        ))
        .await;
    }

    async fn rows(&self) -> Vec<Value> {
        let state = AppState::new(Arc::new(common::settings(&[])), self.pool.clone());
        let reader: Arc<dyn SessionReader> = Arc::new(Owner);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let req = atest::TestRequest::get()
            .uri("/api/v3/views/admin_customers?branch=MAIN&limit=100")
            .insert_header((header::HOST, HOST))
            .insert_header((AS, "OWNER"));
        let resp = atest::call_service(&app, req.to_request()).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = atest::read_body_json(resp).await;
        body["data"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| panic!("data array: {body}"))
    }

    async fn row_of(&self, customer: &str) -> Value {
        self.rows()
            .await
            .into_iter()
            .find(|r| r["id"] == customer)
            .unwrap_or_else(|| panic!("{customer} is listed"))
    }
}

/// Now, to the millisecond (what the view prints), less `days` days.
fn days_ago(days: i64) -> DateTime<Utc> {
    let now = Utc::now();
    let ms = now.timestamp_millis() - days * 86_400_000;
    DateTime::from_timestamp_millis(ms).expect("a valid instant")
}

fn printed(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// 12 calendar months before now, shifted by `hours`.
fn twelve_months_ago_plus_hours(hours: i64) -> DateTime<Utc> {
    let base = Utc::now()
        .checked_sub_months(chrono::Months::new(12))
        .expect("12 months ago");
    base + chrono::Duration::hours(hours)
}

#[actix_web::test]
async fn e4_1_rfm_counts_only_the_12_rolling_months_while_completed_stays_all_time() {
    let db = Db::start().await;
    let cx = db.customer("Recent and old").await;
    let month_ago = days_ago(30);
    db.exchange(&cx, "MAIN", days_ago(400), "1000.00", "LIVE", "COMPLETED")
        .await;
    db.exchange(&cx, "MAIN", month_ago, "3350.25", "LIVE", "COMPLETED")
        .await;

    let row = db.row_of(&cx).await;
    assert_eq!(row["completed"], 2, "all-time, unchanged: {row}");
    assert_eq!(row["volume"], "4350.25", "all-time, unchanged: {row}");
    assert_eq!(row["rfmCompleted"], 1, "{row}");
    assert_eq!(row["rfmVolume"], "3350.25", "{row}");
    let recent = printed(month_ago);
    assert_eq!(row["rfmLastAt"], recent.as_str(), "{row}");
    assert_eq!(row["lastAt"], recent.as_str(), "{row}");
}

#[actix_web::test]
async fn e4_2_a_customer_with_nothing_in_the_window_answers_zero_and_null() {
    let db = Db::start().await;
    let cx = db.customer("Only old").await;
    db.exchange(&cx, "MAIN", days_ago(400), "1000.00", "LIVE", "COMPLETED")
        .await;
    let never = db.customer("Never").await;

    let row = db.row_of(&cx).await;
    assert_eq!(row["completed"], 1, "{row}");
    assert_eq!(row["rfmCompleted"], 0, "{row}");
    assert_eq!(row["rfmVolume"], "0.00", "{row}");
    assert_eq!(row["rfmLastAt"], Value::Null, "{row}");

    let row = db.row_of(&never).await;
    assert_eq!(row["rfmCompleted"], 0, "{row}");
    assert_eq!(row["rfmVolume"], "0.00", "{row}");
    assert_eq!(row["rfmLastAt"], Value::Null, "{row}");
}

#[actix_web::test]
async fn e4_3_reversed_demo_and_other_branch_exchanges_never_count() {
    let db = Db::start().await;
    let cx = db.customer("Mixed").await;
    let month_ago = days_ago(30);
    db.exchange(&cx, "MAIN", month_ago, "500.00", "LIVE", "COMPLETED")
        .await;
    db.exchange(&cx, "MAIN", days_ago(60), "700.00", "LIVE", "REVERSED")
        .await;
    db.exchange(&cx, "MAIN", days_ago(90), "900.00", "DEMO", "COMPLETED")
        .await;
    db.exchange(&cx, "B2", days_ago(120), "1100.00", "LIVE", "COMPLETED")
        .await;

    let row = db.row_of(&cx).await;
    assert_eq!(
        row["rfmCompleted"], 1,
        "only the LIVE COMPLETED exchange at MAIN: {row}"
    );
    assert_eq!(row["rfmVolume"], "500.00", "{row}");
    assert_eq!(row["rfmLastAt"], printed(month_ago).as_str(), "{row}");
}

#[actix_web::test]
async fn e4_4_the_window_starts_exactly_12_months_before_the_read() {
    let db = Db::start().await;
    let cx = db.customer("Edge").await;
    db.exchange(
        &cx,
        "MAIN",
        twelve_months_ago_plus_hours(1),
        "10.00",
        "LIVE",
        "COMPLETED",
    )
    .await;
    db.exchange(
        &cx,
        "MAIN",
        twelve_months_ago_plus_hours(-1),
        "20.00",
        "LIVE",
        "COMPLETED",
    )
    .await;

    let row = db.row_of(&cx).await;
    assert_eq!(row["completed"], 2, "{row}");
    assert_eq!(
        row["rfmCompleted"], 1,
        "an hour inside counts, an hour outside does not: {row}"
    );
    assert_eq!(row["rfmVolume"], "10.00", "{row}");
}

#[actix_web::test]
async fn e4_5_a_row_has_exactly_the_eleven_fields() {
    let db = Db::start().await;
    let cx = db.customer("Shape").await;
    let row = db.row_of(&cx).await;
    let mut keys: Vec<&str> = row
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "completed",
            "created",
            "id",
            "lastAt",
            "member",
            "name",
            "nationality",
            "rfmCompleted",
            "rfmLastAt",
            "rfmVolume",
            "volume",
        ],
        "{row}"
    );
}
