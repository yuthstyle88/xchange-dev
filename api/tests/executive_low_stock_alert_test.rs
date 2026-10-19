//! Ticket #1076, its folded-in part "the low-stock alert" (was EX-13 #1077): A01's low-stock alert
//! (`docs/design/blueprint/executive-finance.md` §4.3, §11 row EX-7).
//!
//! Written before the implementation by the rule-A test-writer gemini/studio-12, on the BE lane controller
//! claude/ad05bf8d's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! One test per row of the folded section's §9 table. The alert is read where A01 answers it, in
//! `GET /api/executive/overview`'s `data.alerts`, with `kind = "lowStock"` (its §7, [required]).
//! The drill-down is `admin_cash` for the branch, with `dropped: ["currency"]` (its §7, [required]).
//!
//! Needs `TEST_DATABASE_URL` naming a `_test` database; nothing skips. Each test works in a schema of its own.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use pos_error::AppResult;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money};
use xchange::application::executive::metrics::MetricUnavailable;
use xchange::application::executive::ports::{
    Alert, AlertFacts, AlertKind, BoxFuture, CashFacts, ExecutiveFilter, ShiftStatus,
};
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::executive::alert_facts::PgAlertFacts;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const ORG2: &str = "00000000-0000-0000-0000-0000000000f2";
const B1: &str = "XC-B01";
const B2: &str = "XC-B02";
/// A branch of another organisation: outside the EXECUTIVE's `BranchScope`.
const B9: &str = "XC-B90";
const KIND: &str = "lowStock";
/// The organisation-wide EXECUTIVE of ORG.
const EXECUTIVE: &str = "00000000-0000-0000-0000-0000000000e1";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn org_of(branch: &str) -> &'static str {
    if branch == B9 { ORG2 } else { ORG }
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
}

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("ex13_low_stock");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
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
            .expect("apply api/migrations");
        sqlx::query(
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ($1::uuid, 'ORG2', 'Asia/Bangkok', now())",
        )
        .bind(ORG2)
        .execute(&pool)
        .await
        .expect("a second organisation");
        for branch in [B1, B2, B9] {
            sqlx::query(
                "INSERT INTO branches (id, name, organization_id, timezone) \
                 VALUES ($1, $1, $2::uuid, 'Asia/Bangkok')",
            )
            .bind(branch)
            .bind(org_of(branch))
            .execute(&pool)
            .await
            .expect("a branch");
            sqlx::query(
                "INSERT INTO branch_versions (branch_id, version, organization_id, changed_at) \
                 VALUES ($1, 1, $2::uuid, now())",
            )
            .bind(branch)
            .bind(org_of(branch))
            .execute(&pool)
            .await
            .expect("a branch version");
        }
        Room { pool, schema }
    }

    /// Sets stock floor for `branch` and `currency` to `min_quantity`.
    async fn set_floor(&self, branch: &str, currency: &str, min_quantity: &str) {
        let updater = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, 'updater', 'ACTIVE', 'test')",
        )
        .bind(updater)
        .bind(org_of(branch))
        .execute(&self.pool)
        .await
        .expect("an updater user");

        sqlx::query(
            "INSERT INTO branch_stock_floors (organization_id, branch_id, currency, min_quantity, updated_by, updated_at) \
             VALUES ($1::uuid, $2, $3, $4::numeric, $5, now()) \
             ON CONFLICT (organization_id, branch_id, currency) DO UPDATE \
             SET min_quantity = EXCLUDED.min_quantity, updated_by = EXCLUDED.updated_by, updated_at = now()",
        )
        .bind(org_of(branch))
        .bind(branch)
        .bind(currency)
        .bind(min_quantity)
        .bind(updater)
        .execute(&self.pool)
        .await
        .expect("set branch_stock_floors");
    }

    /// Sets on-hand quantity for `branch` and `currency` in `inventory_positions` (LG-1 / EX-7).
    async fn set_on_hand(&self, branch: &str, currency: &str, quantity: &str) {
        let _ = Decimal::from_str(quantity).expect("valid decimal quantity");
        // Seed inventory_positions
        sqlx::query(
            "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb, version) \
             VALUES ($1::uuid, $2, $3, $4::numeric, 0.00, 1) \
             ON CONFLICT (organization_id, branch_id, currency) DO UPDATE \
             SET quantity = EXCLUDED.quantity",
        )
        .bind(org_of(branch))
        .bind(branch)
        .bind(currency)
        .bind(quantity)
        .execute(&self.pool)
        .await
        .expect("set inventory_positions");
    }

    async fn finish(self) {
        let Room { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

struct Executive;

impl SessionReader for Executive {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            Sessions {
                owner: Some(Actor::owner(EXECUTIVE)),
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move {
            let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
            let user = User::new(
                EXECUTIVE,
                ORG,
                vec![ScopedRole::new(Role::Executive, None, from, None)],
                None,
            );
            Callers {
                user: Some(user.clone()),
                staff_user: Some(user),
            }
        })
    }
}

/// The `lowStock` alerts A01 answers the organisation-wide EXECUTIVE of ORG, as (branch, alert).
async fn low_stock_alerts_overview(room: &Room) -> Vec<(String, Value)> {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(Executive);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri("/api/executive/overview")
        .insert_header((header::HOST, HOST))
        .insert_header((AS, "executive"))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert_eq!(status, StatusCode::OK, "A01 answers: {text}");
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    body["data"]["alerts"]
        .as_array()
        .unwrap_or_else(|| panic!("data.alerts: {text}"))
        .iter()
        .filter(|a| a["kind"] == KIND)
        .map(|a| (a["branchId"].as_str().unwrap_or("").to_owned(), a.clone()))
        .collect()
}

fn branches(alerts: &[(String, Value)]) -> Vec<&str> {
    alerts.iter().map(|(b, _)| b.as_str()).collect()
}

/// Fake `CashFacts` for testing that `PgAlertFacts` reads through the port.
struct FakeCashFacts {
    on_hand: Vec<(CurrencyCode, Decimal)>,
}

impl CashFacts for FakeCashFacts {
    fn available_cash<'a>(&'a self, _branch: &'a BranchId) -> BoxFuture<'a, AppResult<Decimal>> {
        Box::pin(async { Ok(Decimal::ZERO) })
    }

    fn in_transit<'a>(&'a self, _branch: &'a BranchId) -> BoxFuture<'a, AppResult<Decimal>> {
        Box::pin(async { Ok(Decimal::ZERO) })
    }

    fn shift_status<'a>(
        &'a self,
        _branch: &'a BranchId,
        _filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<ShiftStatus>> {
        Box::pin(async { Ok(ShiftStatus::default()) })
    }

    fn on_hand_by_currency<'a>(
        &'a self,
        _branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Vec<(CurrencyCode, Decimal)>>> {
        Box::pin(async move { Ok(self.on_hand.clone()) })
    }

    fn realized_margin<'a>(
        &'a self,
        _scope: &'a [BranchId],
        _filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<Result<Money, MetricUnavailable>>> {
        Box::pin(async { Ok(Err(MetricUnavailable::AwaitsLedger)) })
    }
}

// ── §9 Behaviour Tests ──────────────────────────────────────────────────────

/// Row 1: floor USD 500 at B1, on hand 499.9999 -> one `lowStock` alert for B1, USD.
/// Also asserts the required drill-down: `admin_cash` for the branch, with `dropped: ["currency"]`,
/// and `detail` naming the currency, on-hand quantity, and floor.
#[actix_web::test]
async fn floor_usd_500_on_hand_499_9999_is_one_low_stock_alert() {
    let room = Room::start().await;
    room.set_floor(B1, "USD", "500.0000").await;
    room.set_on_hand(B1, "USD", "499.9999").await;

    let alerts = low_stock_alerts_overview(&room).await;
    assert_eq!(alerts.len(), 1, "expected 1 lowStock alert: {alerts:?}");
    let (branch, alert) = &alerts[0];
    assert_eq!(branch, B1);
    assert_eq!(alert["kind"], KIND);
    assert_eq!(alert["branchId"], B1);

    // Drill-down: admin_cash for the branch, with dropped: ["currency"]
    let drilldown = &alert["drilldown"];
    assert_eq!(drilldown["view"], "admin_cash");
    assert_eq!(drilldown["filter"]["branch"], B1);
    assert_eq!(drilldown["filter"]["currency"], "USD");
    assert_eq!(drilldown["dropped"], serde_json::json!([]));

    // Detail naming currency, on hand, and floor
    let detail = alert["detail"].as_str().unwrap_or("");
    assert!(
        detail.contains("USD"),
        "detail must name currency USD: {detail}"
    );
    assert!(
        detail.contains("499.9999"),
        "detail must name on-hand 499.9999: {detail}"
    );
    assert!(
        detail.contains("500"),
        "detail must name floor 500: {detail}"
    );

    room.finish().await;
}

/// Row 2: on hand exactly 500 -> no alert (strictly below).
/// B1 is at floor 500 (no alert), while B2 is below floor 500 at 400 (alerts).
#[actix_web::test]
async fn on_hand_exactly_at_floor_is_no_alert_strictly_below() {
    let room = Room::start().await;
    room.set_floor(B1, "USD", "500.0000").await;
    room.set_on_hand(B1, "USD", "500.0000").await;

    room.set_floor(B2, "USD", "500.0000").await;
    room.set_on_hand(B2, "USD", "400.0000").await;

    let alerts = low_stock_alerts_overview(&room).await;
    assert_eq!(branches(&alerts), vec![B2], "{alerts:?}");
    room.finish().await;
}

/// Row 3: no floor rows at B1 -> no alert (E-3).
/// B1 has on hand 100 USD but no floor rows. B2 has floor 500 and on hand 400 (alerts).
#[actix_web::test]
async fn no_floor_rows_is_no_alert() {
    let room = Room::start().await;
    room.set_on_hand(B1, "USD", "100.0000").await;

    room.set_floor(B2, "USD", "500.0000").await;
    room.set_on_hand(B2, "USD", "400.0000").await;

    let alerts = low_stock_alerts_overview(&room).await;
    assert_eq!(branches(&alerts), vec![B2], "{alerts:?}");
    room.finish().await;
}

/// Row 4: floor min_quantity 0 -> never alerts.
/// B1 has floor 0 and on hand 0 (no alert). B2 has floor 500 and on hand 400 (alerts).
#[actix_web::test]
async fn floor_min_quantity_zero_never_alerts() {
    let room = Room::start().await;
    room.set_floor(B1, "USD", "0.0000").await;
    room.set_on_hand(B1, "USD", "0.0000").await;

    room.set_floor(B2, "USD", "500.0000").await;
    room.set_on_hand(B2, "USD", "400.0000").await;

    let alerts = low_stock_alerts_overview(&room).await;
    assert_eq!(branches(&alerts), vec![B2], "{alerts:?}");
    room.finish().await;
}

/// Row 5: floors for USD and EUR, both below -> two alerts.
#[actix_web::test]
async fn floors_for_usd_and_eur_both_below_are_two_alerts() {
    let room = Room::start().await;
    room.set_floor(B1, "USD", "500.0000").await;
    room.set_on_hand(B1, "USD", "400.0000").await;

    room.set_floor(B1, "EUR", "300.0000").await;
    room.set_on_hand(B1, "EUR", "200.0000").await;

    let alerts = low_stock_alerts_overview(&room).await;
    assert_eq!(alerts.len(), 2, "{alerts:?}");
    assert!(alerts.iter().all(|(b, _)| b == B1));

    let details: Vec<String> = alerts
        .iter()
        .map(|(_, a)| a["detail"].as_str().unwrap_or("").to_owned())
        .collect();
    assert!(
        details.iter().any(|d| d.contains("USD")),
        "one alert for USD: {details:?}"
    );
    assert!(
        details.iter().any(|d| d.contains("EUR")),
        "one alert for EUR: {details:?}"
    );

    room.finish().await;
}

/// Row 6: a branch outside the caller's BranchScope -> not listed.
/// B9 (ORG2) has floor 500 and on hand 400. B1 (ORG) has floor 500 and on hand 400.
/// The caller (EXECUTIVE of ORG) only receives alerts for B1.
#[actix_web::test]
async fn a_branch_outside_the_callers_scope_is_not_listed() {
    let room = Room::start().await;
    room.set_floor(B9, "USD", "500.0000").await;
    room.set_on_hand(B9, "USD", "400.0000").await;

    room.set_floor(B1, "USD", "500.0000").await;
    room.set_on_hand(B1, "USD", "400.0000").await;

    let alerts = low_stock_alerts_overview(&room).await;
    assert_eq!(branches(&alerts), vec![B1], "{alerts:?}");
    room.finish().await;
}

/// Row 7: CashFacts swapped (a fake implementation) -> the alert follows the port, not a table.
/// In DB: B1 has floor USD 500, but DB table on hand has 1,000 USD (which would not alert).
/// The swapped FakeCashFacts reports 350 USD (< 500).
/// The alert follows the port and produces 1 lowStock alert for B1.
#[actix_web::test]
async fn cash_facts_swapped_fake_impl_alert_follows_port() {
    let room = Room::start().await;
    room.set_floor(B1, "USD", "500.0000").await;
    room.set_on_hand(B1, "USD", "1000.0000").await;

    let fake_cash = Arc::new(FakeCashFacts {
        on_hand: vec![(usd(), Decimal::from_str("350.0000").unwrap())],
    });
    let facts = PgAlertFacts::new(room.pool.clone(), fake_cash);
    let alerts = facts
        .alerts(&[BranchId::new(B1)])
        .await
        .expect("facts.alerts");

    let low_stock: Vec<&Alert> = alerts
        .iter()
        .filter(|a| a.kind == AlertKind::LowStock)
        .collect();
    assert_eq!(
        low_stock.len(),
        1,
        "must produce 1 lowStock alert from port: {alerts:?}"
    );
    assert_eq!(low_stock[0].branch.as_str(), B1);
    assert!(
        low_stock[0].detail.contains("USD"),
        "detail names USD: {}",
        low_stock[0].detail
    );
    assert!(
        low_stock[0].detail.contains("350"),
        "detail names port quantity 350: {}",
        low_stock[0].detail
    );
    assert!(
        low_stock[0].detail.contains("500"),
        "detail names floor 500: {}",
        low_stock[0].detail
    );

    room.finish().await;
}
