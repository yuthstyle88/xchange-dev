//! Ticket #1040 (LG-6.r1, `docs/design/blueprint/ledger-inventory.md` §4.2, §4.2.1, §10, as
//! amended by design PR 1037): admin_cash view reads `ShiftBalances` over the ledger.
//! Written before the implementation by the test-writer (gemini/pool-1).
//!
//! §9 Edge cases:
//! - Row 1: no open shift -> the same empty answer as today (with lastCounted from ShiftBalances).
//! - Row 2: the last closed shift has a verified count -> lastCounted = its counted figures.
//! - Row 3: a drawer->safe move of 500 THB on the open shift -> the outgoing stat includes 500.
//! - Row 4: an exchange only -> thbIn/thbOut unchanged by it (it is in cash received/paid).

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use std::future::Future;
use std::pin::Pin;

use actix_web::http::Method;
use pos_error::AppResult;
use rpc_common::{App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, QuantityDelta};
use xc_shift::domain::shift::repositories::shift_balances::{ShiftBalance, ShiftBalances, ShiftId};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

const ORIGIN: &str = "http://desk.test";
const CASH: &str = "admin_cash";
const MAIN: &str = "MAIN";

const BUY_RATE: &str = "30";
const SELL_RATE: &str = "40";

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn request_id() -> String {
    format!("00000000-0000-4000-8000-{:012x}", next())
}

fn get(path: &str) -> Call {
    let mut call = Call::post(ORIGIN, path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

#[derive(Clone, Default)]
pub struct SeededShiftBalances {
    balances: Arc<Mutex<BTreeMap<ShiftId, Vec<ShiftBalance>>>>,
}

impl SeededShiftBalances {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, shift: &str, rows: Vec<ShiftBalance>) {
        self.balances
            .lock()
            .unwrap()
            .insert(shift.to_string(), rows);
    }
}

impl ShiftBalances for SeededShiftBalances {
    fn shift_balances<'a>(
        &'a self,
        shift: &'a ShiftId,
    ) -> BoxFuture<'a, AppResult<Vec<ShiftBalance>>> {
        let rows = self
            .balances
            .lock()
            .unwrap()
            .get(shift)
            .cloned()
            .unwrap_or_default();
        Box::pin(async move { Ok(rows) })
    }
}

fn shift_balance(
    currency: &str,
    opening: &str,
    expected: &str,
    moved_in: &str,
    moved_out: &str,
    counted: Option<&str>,
    difference: Option<&str>,
) -> ShiftBalance {
    let fa = |s: &str| {
        ForeignAmount::from_decimal(rust_decimal::Decimal::new(
            Money::parse(s).expect("valid money").try_minor().unwrap(),
            2,
        ))
    };
    ShiftBalance {
        currency: CurrencyCode::parse(currency).expect("valid currency"),
        opening: fa(opening),
        expected: QuantityDelta::from_decimal(rust_decimal::Decimal::from_i128_with_scale(
            Money::parse(expected).expect("valid expected").minor_i128(),
            2,
        ))
        .expect("valid expected"),
        moved_in: fa(moved_in),
        moved_out: fa(moved_out),
        counted: counted.map(fa),
        difference: difference.map(|d| {
            let m = Money::parse(d.trim_start_matches('-')).expect("valid difference");
            let minor = if d.starts_with('-') {
                -m.minor_i128()
            } else {
                m.minor_i128()
            };
            QuantityDelta::from_decimal(rust_decimal::Decimal::from_i128_with_scale(minor, 2))
                .expect("valid difference")
        }),
    }
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    sb: SeededShiftBalances,
}

impl Room {
    async fn open() -> Self {
        let sb = SeededShiftBalances::new();
        let desk = Desk::open().await.with_shift_balances(Arc::new(sb.clone()));
        snapshot_common::seed_opener(&desk.db.pool, "owner").await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        rates_re4e_publish::set_counter_rate(&desk.db.pool, MAIN, "USD", BUY_RATE, SELL_RATE).await;
        Self {
            desk,
            app,
            cookies,
            sb,
        }
    }

    async fn cash(&self, query: &str) -> Value {
        let path = if query.is_empty() {
            format!("/api/v3/views/{CASH}")
        } else {
            format!("/api/v3/views/{CASH}?{query}")
        };
        let call = get(&path).cookie(&self.cookies["OWNER"]);
        let answer = self.app.send(call).await;
        assert_eq!(answer.status, 200, "GET {path}: {}", answer.text);
        let body = answer.json();
        assert_eq!(body["changed"], json!(true), "GET {path}: {body}");
        body.get("data")
            .cloned()
            .unwrap_or_else(|| panic!("data in {body}"))
    }
}

async fn seed_closed_shift(pool: &PgPool, branch: &str, shift_id: &str) {
    let req = request_id();
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, closed_at, closed_by, open_request_id, close_request_id, teller_id) \
         VALUES ($1, $2, 'CLOSED', now() - interval '1 hour', 'owner', now(), 'owner', $3, $4, (SELECT id FROM users WHERE name = 'owner' LIMIT 1))",
    )
    .bind(shift_id)
    .bind(branch)
    .bind(format!("open-{req}"))
    .bind(format!("close-{req}"))
    .execute(pool)
    .await
    .expect("seed closed shift");
}

async fn seed_open_shift(pool: &PgPool, branch: &str, shift_id: &str) {
    let req = request_id();
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         VALUES ($1, $2, 'OPEN', now(), 'owner', $3, (SELECT id FROM users WHERE name = 'owner' LIMIT 1))",
    )
    .bind(shift_id)
    .bind(branch)
    .bind(format!("open-{req}"))
    .execute(pool)
    .await
    .expect("seed open shift");
}

async fn seed_completed_cash_sale(
    pool: &PgPool,
    shift_id: &str,
    currency: &str,
    quantity: &str,
    rate: &str,
    settle_thb: &str,
) {
    let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
        .fetch_one(pool)
        .await
        .expect("branch organization_id");

    let req = request_id();
    let tx_id = Uuid::new_v4();
    let line_id = Uuid::new_v4();
    let payment_id = Uuid::new_v4();
    let receipt_key = format!("REC-{req}");

    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ('CU-1', $1, 'CU-1', 'Somchai', 'LIVE', now()) ON CONFLICT DO NOTHING",
    )
    .bind(org_id)
    .execute(pool)
    .await
    .expect("insert customer");

    sqlx::query(
        "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
         VALUES ($1, 'MAIN', 'owner', 'application/pdf', 'sha', now()) ON CONFLICT DO NOTHING",
    )
    .bind(&receipt_key)
    .execute(pool)
    .await
    .expect("insert file");

    let mut tx = pool.begin().await.expect("begin tx");
    sqlx::query(
        "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
         customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, \
         recorded_at, recorded_by, status, completed_at) \
         VALUES ($1, $2, $3, 'MAIN', 'LIVE', $4, $5, $6, 'CU-1', 'Somchai', 'THA', 'owner', '', '', \
         CURRENT_DATE, now(), now(), 'owner', 'COMPLETED', now())",
    )
    .bind(tx_id)
    .bind(org_id)
    .bind(format!("TR-{req}"))
    .bind(shift_id)
    .bind(format!("sub-{req}"))
    .bind(format!("hash-{req}"))
    .execute(&mut *tx)
    .await
    .expect("insert transactions");

    sqlx::query(
        "INSERT INTO transaction_files (transaction_id, kind, file_key) VALUES ($1, 'RECEIPT', $2)",
    )
    .bind(tx_id)
    .bind(&receipt_key)
    .execute(&mut *tx)
    .await
    .expect("insert transaction_files");

    sqlx::query(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
         quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
         VALUES ($1, $2, $3, 'SELL', $4, $5::numeric, 1, $6::numeric, $7::numeric, 0, $7::numeric, 1, now(), now())",
    )
    .bind(line_id)
    .bind(org_id)
    .bind(tx_id)
    .bind(currency)
    .bind(quantity)
    .bind(rate)
    .bind(settle_thb)
    .execute(&mut *tx)
    .await
    .expect("insert transaction_lines");

    sqlx::query(
        "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, method, \
         amount_thb, bank_fee_thb, paid_at, created_at, updated_at) \
         VALUES ($1, $2, $3, 1, 'Cash', $4::numeric, 0, now(), now(), now())",
    )
    .bind(payment_id)
    .bind(org_id)
    .bind(tx_id)
    .bind(settle_thb)
    .execute(&mut *tx)
    .await
    .expect("insert transaction_payments");

    tx.commit().await.expect("commit tx");
}

// ── §9 Edge case 1 ────────────────────────────────────────────────────────────

#[actix_web::test]
async fn case_1_no_open_shift_answers_the_same_empty_answer_as_today_with_last_counted_from_shift_balances()
 {
    let room = Room::open().await;
    seed_closed_shift(&room.desk.db.pool, MAIN, "S-CLOSED-1").await;
    room.sb.set(
        "S-CLOSED-1",
        vec![
            shift_balance(
                "THB",
                "50000.00",
                "50000.00",
                "0.00",
                "0.00",
                Some("50000.00"),
                Some("0.00"),
            ),
            shift_balance(
                "USD",
                "1000.00",
                "1000.00",
                "0.00",
                "0.00",
                Some("1000.00"),
                Some("0.00"),
            ),
            shift_balance("EUR", "0.00", "0.00", "0.00", "0.00", None, None),
        ],
    );

    let data = room.cash("").await;

    // The same empty answer as today for shift, stock, stats, reference
    assert_eq!(
        data["openShifts"],
        json!([]),
        "no open shift has openShifts: [] (CA-10q)"
    );
    assert_eq!(data["stock"], json!([]), "no open shift has empty stock");
    assert_eq!(data["stats"], json!(null), "no open shift has stats: null");
    assert_eq!(
        data["reference"],
        json!({
            "withReference": 0,
            "withoutReference": 0,
            "spread": "0.00",
            "percent": "0.00"
        }),
        "no open shift has empty reference summary"
    );
    assert_eq!(
        data["currencies"]
            .as_array()
            .expect("currencies list")
            .len(),
        153,
        "currencies list is the register (153 codes)"
    );

    // lastCounted comes from the last closed shift's ShiftBalances (counted figures, THB first then A-Z)
    assert_eq!(
        data["lastCounted"],
        json!([
            { "code": "THB", "amount": "50000.00" },
            { "code": "USD", "amount": "1000.00" }
        ]),
        "lastCounted reflects verified count figures from ShiftBalances"
    );
}

// ── §9 Edge case 2 ────────────────────────────────────────────────────────────

#[actix_web::test]
async fn case_2_the_last_closed_shift_has_a_verified_count_reflected_in_last_counted() {
    let room = Room::open().await;
    seed_closed_shift(&room.desk.db.pool, MAIN, "S-CLOSED-2").await;
    seed_open_shift(&room.desk.db.pool, MAIN, "S-OPEN-2").await;

    room.sb.set(
        "S-CLOSED-2",
        vec![
            shift_balance(
                "THB",
                "50000.00",
                "50000.00",
                "0.00",
                "0.00",
                Some("12345.67"),
                Some("0.00"),
            ),
            shift_balance(
                "USD",
                "1000.00",
                "1000.00",
                "0.00",
                "0.00",
                Some("200.00"),
                Some("0.00"),
            ),
            shift_balance("JPY", "0.00", "0.00", "0.00", "0.00", Some("0.00"), None),
            shift_balance("EUR", "0.00", "0.00", "0.00", "0.00", None, None),
        ],
    );
    room.sb.set(
        "S-OPEN-2",
        vec![shift_balance(
            "THB",
            "100000.00",
            "100000.00",
            "0.00",
            "0.00",
            None,
            None,
        )],
    );

    let data = room.cash("").await;

    assert_eq!(data["openShifts"][0]["shiftId"], "S-OPEN-2");
    assert_eq!(
        data["lastCounted"],
        json!([
            { "code": "THB", "amount": "12345.67" },
            { "code": "USD", "amount": "200.00" }
        ]),
        "lastCounted has verified counted figures of the last closed shift from ShiftBalances"
    );
}

// ── §9 Edge case 3 ────────────────────────────────────────────────────────────

#[actix_web::test]
async fn case_3_a_drawer_to_safe_move_of_500_thb_on_the_open_shift_includes_in_outgoing_stat() {
    let room = Room::open().await;
    seed_open_shift(&room.desk.db.pool, MAIN, "S-OPEN-3").await;

    // A drawer->safe move of 500 THB on the open shift:
    // opening = 100000.00, expected = 99500.00, moved_in = 0.00, moved_out = 500.00
    room.sb.set(
        "S-OPEN-3",
        vec![shift_balance(
            "THB",
            "100000.00",
            "99500.00",
            "0.00",
            "500.00",
            None,
            None,
        )],
    );

    let data = room.cash("").await;

    assert_eq!(data["openShifts"][0]["shiftId"], "S-OPEN-3");

    // Outgoing stat includes the 500 THB drawer->safe move (thb_out = moved_out)
    let stats = &data["stats"];
    assert_eq!(stats["opening"], "100000.00");
    assert_eq!(stats["incoming"], "0.00");
    assert_eq!(
        stats["outgoing"], "500.00",
        "outgoing stat includes 500 THB from moved_out"
    );
    assert_eq!(
        stats["remaining"], "99500.00",
        "remaining reflects 100000 - 500 = 99500"
    );

    // Stock row for THB
    let stock = data["stock"].as_array().expect("stock array");
    let thb_row = stock
        .iter()
        .find(|r| r["code"] == "THB")
        .expect("THB stock row");
    assert_eq!(thb_row["opening"], "100000.00");
    assert_eq!(thb_row["net"], "-500.00");
    assert_eq!(thb_row["expected"], "99500.00");
}

// ── §9 Edge case 4 ────────────────────────────────────────────────────────────

#[actix_web::test]
async fn case_4_an_exchange_only_leaves_thb_in_and_thb_out_unchanged() {
    let room = Room::open().await;
    seed_open_shift(&room.desk.db.pool, MAIN, "S-OPEN-4").await;

    // Completed cash exchange: sell 100 USD at 40 -> customer pays 4,000 THB cash
    seed_completed_cash_sale(
        &room.desk.db.pool,
        "S-OPEN-4",
        "USD",
        "100",
        "40",
        "4000.00",
    )
    .await;

    // In ShiftBalances, EXCHANGE postings do not change moved_in or moved_out:
    // moved_in = 0.00, moved_out = 0.00
    // opening THB = 100000.00, expected THB = 104000.00
    // opening USD = 1000.00, expected USD = 900.00
    room.sb.set(
        "S-OPEN-4",
        vec![
            shift_balance("THB", "100000.00", "104000.00", "0.00", "0.00", None, None),
            shift_balance("USD", "1000.00", "900.00", "0.00", "0.00", None, None),
        ],
    );

    let data = room.cash("").await;

    assert_eq!(data["openShifts"][0]["shiftId"], "S-OPEN-4");

    // thbIn/thbOut are unchanged by the exchange (both 0.00), so incoming is 4000.00 from cash_received alone
    let stats = &data["stats"];
    assert_eq!(stats["opening"], "100000.00");
    assert_eq!(
        stats["incoming"], "4000.00",
        "incoming has cash_received alone, not double-counted by thb_in"
    );
    assert_eq!(stats["outgoing"], "0.00");
    assert_eq!(stats["remaining"], "104000.00");

    // Stock rows for THB and USD read from ShiftBalances
    let stock = data["stock"].as_array().expect("stock array");
    let thb_row = stock
        .iter()
        .find(|r| r["code"] == "THB")
        .expect("THB stock row");
    assert_eq!(thb_row["opening"], "100000.00");
    assert_eq!(thb_row["net"], "4000.00");
    assert_eq!(thb_row["expected"], "104000.00");

    let usd_row = stock
        .iter()
        .find(|r| r["code"] == "USD")
        .expect("USD stock row");
    assert_eq!(usd_row["opening"], "1000.00");
    assert_eq!(usd_row["net"], "-100.00");
    assert_eq!(usd_row["expected"], "900.00");
}
