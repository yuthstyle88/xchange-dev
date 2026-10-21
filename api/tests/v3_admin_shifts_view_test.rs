//! LG-6.r2 (#1041): the `admin_shifts` view reads `ShiftBalances` (§7, §12.6).
//! Up-front tests for the behavior table (§9).

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use actix_web::http::Method;
use chrono::{Duration, Utc};
use pos_error::AppResult;
use rpc_common::{App, Call, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta};
use xc_shift::domain::shift::repositories::shift_balances::{ShiftBalance, ShiftBalances, ShiftId};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

const ORIGIN: &str = "http://desk.test";
const MAIN: &str = "MAIN";

#[derive(Clone, Default)]
struct FakeShiftBalances {
    balances: Arc<Mutex<HashMap<ShiftId, Vec<ShiftBalance>>>>,
    calls: Arc<Mutex<Vec<ShiftId>>>,
}

impl FakeShiftBalances {
    fn new() -> Self {
        Self::default()
    }

    fn set(&self, shift: impl Into<ShiftId>, balances: Vec<ShiftBalance>) {
        self.balances.lock().unwrap().insert(shift.into(), balances);
    }

    fn calls_for(&self, shift: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.as_str() == shift)
            .count()
    }
}

impl ShiftBalances for FakeShiftBalances {
    fn shift_balances<'a>(
        &'a self,
        shift: &'a ShiftId,
    ) -> BoxFuture<'a, AppResult<Vec<ShiftBalance>>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(shift.clone());
            let list = self
                .balances
                .lock()
                .unwrap()
                .get(shift)
                .cloned()
                .unwrap_or_default();
            Ok(list)
        })
    }
}

fn cur(code: &str) -> CurrencyCode {
    CurrencyCode::parse(code).expect("valid currency")
}

fn money(amount: &str) -> Money {
    Money::parse(amount).expect("valid money")
}

fn delta(minor: i128) -> MoneyDelta {
    MoneyDelta::from_minor(minor)
}

fn shift_balance(
    code: &str,
    opening: &str,
    expected_minor: i128,
    counted: Option<&str>,
    difference_minor: Option<i128>,
) -> ShiftBalance {
    ShiftBalance {
        currency: cur(code),
        opening: money(opening),
        expected: delta(expected_minor),
        moved_in: Money::ZERO,
        moved_out: Money::ZERO,
        counted: counted.map(money),
        difference: difference_minor.map(delta),
    }
}

struct TestContext {
    desk: Desk,
    app: App,
    owner_cookie: String,
    fake: Arc<FakeShiftBalances>,
}

impl TestContext {
    async fn setup() -> Self {
        let desk = Desk::open().await;
        let fake = Arc::new(FakeShiftBalances::new());
        let state = desk.state().with_shift_balances(fake.clone());
        let clock: Arc<dyn xchange::presentation::http::rpc::RpcClock> = desk.clock.clone();
        let bank: Arc<dyn xc_rate::domain::rate::entities::rate_source::RateSource> =
            desk.bank.clone();
        let app = App::new(
            actix_web::test::init_service(
                actix_web::App::new()
                    .configure(move |cfg| xchange::bootstrap::configure(cfg, state))
                    .app_data(actix_web::web::Data::from(clock))
                    .app_data(actix_web::web::Data::new(bank)),
            )
            .await,
            |t| t.to_request(),
        );
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let owner_cookie = cookies["OWNER"].clone();
        Self {
            desk,
            app,
            owner_cookie,
            fake,
        }
    }

    async fn get_admin_shifts(&self, query: &str) -> Value {
        let path = if query.is_empty() {
            "/api/v3/views/admin_shifts".to_string()
        } else {
            format!("/api/v3/views/admin_shifts?{query}")
        };
        let call = Call {
            method: Method::GET,
            host: "desk.test".to_string(),
            path: path.clone(),
            headers: [
                ("origin".to_string(), ORIGIN.to_string()),
                ("x-xchange-request".to_string(), "1".to_string()),
                ("cookie".to_string(), self.owner_cookie.clone()),
            ]
            .into_iter()
            .collect(),
            body: vec![],
        };
        let answer = self.app.send(call).await;
        assert_eq!(answer.status, 200, "GET {path}: {}", answer.text);
        answer.json()["data"].clone()
    }
}

/// §9 Row 1: a closed shift with a verified count & non-zero difference
/// -> its difference item is listed, THB first.
#[actix_web::test]
async fn test_closed_shift_with_verified_count_lists_difference_thb_first() {
    let ctx = TestContext::setup().await;
    let t0 = Utc::now();

    let shift_id = "S-CLOSED-DIFF";
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, closed_at, closed_by, close_note, open_request_id, close_request_id, teller_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, (SELECT id FROM users LIMIT 1))",
    )
    .bind(shift_id)
    .bind(MAIN)
    .bind("CLOSED")
    .bind(t0)
    .bind("owner")
    .bind(Some(t0 + Duration::hours(8)))
    .bind(Some("owner"))
    .bind("THB ขาด 50 · EUR เกิน 2 · USD เกิน 10")
    .bind("req-open-diff")
    .bind(Some("req-close-diff"))
    .execute(&ctx.desk.db.pool)
    .await
    .expect("insert closed shift");

    // Provide balances from ShiftBalances port:
    // USD has non-zero diff (+10.00)
    // THB has non-zero diff (-50.00)
    // EUR has non-zero diff (+2.00) (sorts before THB alphabetically, pinning THB-first ordering)
    // JPY has exact count (difference 0.00), absent from difference and note, present in balances
    ctx.fake.set(
        shift_id,
        vec![
            shift_balance("USD", "100.00", 10000, Some("110.00"), Some(1000)),
            shift_balance("THB", "1000.00", 100000, Some("950.00"), Some(-5000)),
            shift_balance("EUR", "50.00", 5000, Some("52.00"), Some(200)),
            shift_balance("JPY", "1000.00", 100000, Some("1000.00"), Some(0)),
        ],
    );

    let rows = ctx.get_admin_shifts("branch=MAIN").await;
    let row = rows
        .as_array()
        .and_then(|arr| arr.iter().find(|r| r["id"] == json!(shift_id)))
        .unwrap_or_else(|| panic!("shift {shift_id} not found in {rows}"));

    assert_eq!(row["status"], json!("CLOSED"), "{row}");
    assert_eq!(
        row["note"],
        json!("THB ขาด 50 · EUR เกิน 2 · USD เกิน 10"),
        "{row}"
    );

    // Difference items: non-zero difference, THB first then A-Z!
    // EUR sorts alphabetically before THB ('E' < 'T'), pinning that THB comes first.
    // JPY has exact count (difference 0.00), so it is absent from difference and note.
    assert_eq!(
        row["difference"],
        json!([
            { "code": "THB", "amount": "-50.00" },
            { "code": "EUR", "amount": "2.00" },
            { "code": "USD", "amount": "10.00" },
        ]),
        "difference items must list THB first, then A-Z: {row}"
    );

    // Detail balance rows: THB first, then EUR, then JPY, then USD
    assert_eq!(
        row["balances"],
        json!([
            { "code": "THB", "expected": "1000.00", "counted": "950.00", "difference": "-50.00" },
            { "code": "EUR", "expected": "50.00", "counted": "52.00", "difference": "2.00" },
            { "code": "JPY", "expected": "1000.00", "counted": "1000.00", "difference": "0.00" },
            { "code": "USD", "expected": "100.00", "counted": "110.00", "difference": "10.00" },
        ]),
        "balances must be ordered THB first, then A-Z: {row}"
    );

    assert_eq!(
        ctx.fake.calls_for(shift_id),
        1,
        "ShiftBalances::shift_balances must be called for {shift_id}"
    );
}

/// §9 Row 2: an open shift
/// -> no difference items; opening/expected shown.
#[actix_web::test]
async fn test_open_shift_shows_opening_and_expected_without_difference() {
    let ctx = TestContext::setup().await;
    let t0 = Utc::now();

    let shift_id = "S-OPEN-1";
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         VALUES ($1, $2, $3, $4, $5, $6, (SELECT id FROM users LIMIT 1))",
    )
    .bind(shift_id)
    .bind(MAIN)
    .bind("OPEN")
    .bind(t0)
    .bind("owner")
    .bind("req-open-open1")
    .execute(&ctx.desk.db.pool)
    .await
    .expect("insert open shift");

    ctx.fake.set(
        shift_id,
        vec![
            shift_balance("THB", "5000.00", 650000, None, None),
            shift_balance("JPY", "10000.00", 1000000, None, None),
        ],
    );

    let rows = ctx.get_admin_shifts("branch=MAIN").await;
    let row = rows
        .as_array()
        .and_then(|arr| arr.iter().find(|r| r["id"] == json!(shift_id)))
        .unwrap_or_else(|| panic!("shift {shift_id} not found in {rows}"));

    assert_eq!(row["status"], json!("OPEN"), "{row}");
    assert_eq!(row["note"], json!(""), "open shift has empty note: {row}");
    assert_eq!(
        row["difference"],
        Value::Null,
        "open shift has no difference items: {row}"
    );

    assert_eq!(
        row["balances"],
        json!([
            { "code": "THB", "expected": "6500.00", "counted": null, "difference": null },
            { "code": "JPY", "expected": "10000.00", "counted": null, "difference": null },
        ]),
        "open shift's running expected balances: {row}"
    );

    assert_eq!(
        ctx.fake.calls_for(shift_id),
        1,
        "ShiftBalances::shift_balances must be called for {shift_id}"
    );
}

/// §9 Row 3: a shift with no posting
/// -> the same empty rows as today.
#[actix_web::test]
async fn test_shift_with_no_posting_shows_same_empty_rows_as_today() {
    let ctx = TestContext::setup().await;
    let t0 = Utc::now();

    let shift_id = "S-NO-POSTING";
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         VALUES ($1, $2, $3, $4, $5, $6, (SELECT id FROM users LIMIT 1))",
    )
    .bind(shift_id)
    .bind(MAIN)
    .bind("OPEN")
    .bind(t0)
    .bind("owner")
    .bind("req-open-nopost")
    .execute(&ctx.desk.db.pool)
    .await
    .expect("insert shift with no posting");

    // Fake returns empty vec (no postings in ledger for this shift)
    ctx.fake.set(shift_id, vec![]);

    let rows = ctx.get_admin_shifts("branch=MAIN").await;
    let row = rows
        .as_array()
        .and_then(|arr| arr.iter().find(|r| r["id"] == json!(shift_id)))
        .unwrap_or_else(|| panic!("shift {shift_id} not found in {rows}"));

    assert_eq!(row["status"], json!("OPEN"), "{row}");
    assert_eq!(row["difference"], Value::Null, "no difference items: {row}");

    // Must show empty rows as today (default THB with 0.00 expected, NOT the stale 9999.00 from table)
    assert_eq!(
        row["balances"],
        json!([
            { "code": "THB", "expected": "0.00", "counted": null, "difference": null },
        ]),
        "empty rows as today: {row}"
    );

    assert_eq!(
        ctx.fake.calls_for(shift_id),
        1,
        "ShiftBalances::shift_balances must be called for {shift_id}"
    );
}
