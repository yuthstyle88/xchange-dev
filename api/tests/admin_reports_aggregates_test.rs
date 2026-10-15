//! Up-front tests for the ticket "admin_reports: grouped SQL instead of per-line Rust re-sums"
//! (`api/src/presentation/http/v3/views/admin_reports.rs`). Written before the implementation by
//! the ticket author for the BE lane controller claude/7ca80c. **Do not edit, delete or weaken
//! this file**; add tests in a file of your own.
//!
//! Controller's rulings (2026-10-15) pinned here:
//! - (a) an exchange's channel is the method of its first payment (`payment_no = 1`);
//! - (b) every count in the report counts **exchanges** (transactions), not lines or payments;
//! - (c) the bank fee is taken once per exchange, from `payment_no = 1`;
//! - the money totals equal the sum of each exchange's own figures, each taken once.
//!
//! The store (`PgTransactionStore::insert_draft_in`) refuses a second line, and nothing writes a
//! second payment yet, but the schema allows both (`UNIQUE (transaction_id, line_no)` and
//! `UNIQUE (transaction_id, payment_no)`). So each exchange is saved through the store with its
//! first line and first payment, and any further line or payment is added with a plain SQL insert
//! into the child table.
//!
//! | Test | On `origin/main` `1eba5943` |
//! |---|---|
//! | `two_lines_count_the_bank_fee_and_the_exchange_once` | red: bank fee `30.00`, counts 2 |
//! | `a_second_payment_does_not_repeat_the_lines` | red: received `2000.00`, PromptPay count 1 |
//! | `totals_equal_each_exchange_summed_once` | red: the line figures and bank fees are multiplied |
//! | `an_employees_currencies_keep_first_appearance_order` | green: a pin on today's order |
//! | `employees_sort_in_byte_order` | green: a pin on today's order |
//! | `equal_reference_lines_sum_exactly` | green: a pin on the per-line reference value |
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing skips.

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
mod rpc_common;
mod shift_setup_common;
mod snapshot_common;

use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::{Value, json};
use uuid::Uuid;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::repository::{StatusMove, TransactionStore};
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};

const ORIGIN: &str = "http://desk.test";
#[allow(dead_code)]
const RPC: &str = "/api/rpc";
const REPORTS: &str = "admin_reports";
const MAIN: &str = "MAIN";
const DAY: &str = "2026-10-06";
const PERIOD: &str = "branch=MAIN&from=2026-10-06&to=2026-10-06";

const RATES: [(&str, &str, &str); 4] = [
    ("USD", "30", "40"),
    ("EUR", "35", "38"),
    ("JPY", "0.2", "0.25"),
    ("GBP", "40", "45"),
];

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn get(path: &str) -> Call {
    let mut call = Call::post(ORIGIN, path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap_or_else(|e| panic!("a decimal {s:?}: {e}"))
}

fn money(d: Decimal) -> Money {
    let mut cents = d.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    cents.rescale(2);
    Money::parse(&cents.to_string()).unwrap_or_else(|e| panic!("money {cents}: {e:?}"))
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    t0: DateTime<Utc>,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let t0 = snapshot_common::time("2026-10-06T05:00:00.000Z");
        desk.clock.set(t0);
        let room = Room {
            desk,
            app,
            cookies,
            t0,
        };
        for (code, buy, sell) in RATES {
            rates_re4e_publish::set_counter_rate(&room.desk.db.pool, MAIN, code, buy, sell).await;
        }
        let t = room.tick();
        shift_setup_common::open_shift_by_count(
            &room.desk.db.pool,
            MAIN,
            json!({ "THB": 1000000, "USD": 100000, "EUR": 100000, "JPY": 10000000 }),
            t,
        )
        .await;
        room
    }

    fn tick(&self) -> DateTime<Utc> {
        let t = self.t0 + Duration::seconds(next() as i64);
        self.desk.clock.set(t);
        t
    }

    async fn report(&self) -> Value {
        let path = format!("/api/v3/views/{REPORTS}?{PERIOD}");
        let call = get(&path).cookie(&self.cookies["OWNER"]);
        let answer: Answer = self.app.send(call).await;
        assert_eq!(answer.status, 200, "GET {path}: {:.400}", answer.text);
        let body = answer.json();
        body.get("data")
            .cloned()
            .unwrap_or_else(|| panic!("GET {path}: data in {body}"))
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// One line of a seeded exchange. `thb = quantity × rate` (unit size 1), the settlement is
/// `thb − fee` for a BUY and `thb + fee` for a SELL, as the desk writes them.
struct L {
    side: Side,
    currency: &'static str,
    quantity: &'static str,
    rate: &'static str,
    reference: Option<&'static str>,
    fee: &'static str,
}

impl L {
    fn thb(&self) -> Decimal {
        dec(self.quantity) * dec(self.rate)
    }

    fn settle(&self) -> Decimal {
        let thb = money(self.thb()).to_string();
        match self.side {
            Side::Buy => dec(&thb) - dec(self.fee),
            Side::Sell => dec(&thb) + dec(self.fee),
        }
    }

    fn line(&self, line_no: i32) -> TransactionLine {
        TransactionLine {
            id: Uuid::new_v4(),
            quote_line_id: None,
            shop_side: self.side,
            currency: CurrencyCode::parse(self.currency).expect("a currency"),
            quantity: ForeignAmount::from_decimal(dec(self.quantity)),
            unit_size: UnitSize::parse("1").expect("unit 1"),
            rate_snapshot: Rate::parse(self.rate).expect("a rate"),
            reference_rate: self
                .reference
                .map(|r| Rate::parse(r).expect("a reference rate")),
            thb_amount: money(self.thb()),
            fee_thb: money(dec(self.fee)),
            settle_thb: money(self.settle()),
            cost_basis_thb: None,
            realized_margin_thb: None,
            line_no,
        }
    }
}

/// One payment of a seeded exchange.
struct P {
    method: PaymentMethod,
    amount: &'static str,
    bank_fee: &'static str,
}

/// Saves one COMPLETED LIVE exchange at MAIN for `staff`: line 1 and payment 1 through
/// `PgTransactionStore` (DRAFT → CONFIRMED → COMPLETED), then every further line (`line_no` 2..)
/// and payment (`payment_no` 2..) with an SQL insert into `transaction_lines` /
/// `transaction_payments`.
async fn seed(room: &Room, staff: &str, lines: &[L], payments: &[P]) -> TransactionId {
    assert!(!lines.is_empty() && !payments.is_empty());
    let pool = &room.desk.db.pool;
    let t = room.tick();
    let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
        .fetch_one(pool)
        .await
        .expect("read MAIN organization_id");
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ('CU-AGG', $1, 'CU-AGG', 'Somchai', 'LIVE', now()) ON CONFLICT DO NOTHING",
    )
    .bind(org_id)
    .execute(pool)
    .await
    .expect("seed the customer");
    sqlx::query(
        "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) VALUES \
         ('PASSPORT-AGG', 'MAIN', 'owner', 'application/pdf', 'sha-p', now()), \
         ('RECEIPT-AGG', 'MAIN', 'owner', 'application/pdf', 'sha-r', now()) \
         ON CONFLICT DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("seed the files");
    let shift_id: String = sqlx::query_scalar(
        "SELECT id FROM shifts WHERE branch_id = 'MAIN' AND closed_at IS NULL \
         ORDER BY opened_at DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("read the open shift");

    let id = TransactionId(Uuid::new_v4());
    let first = &payments[0];
    let payment = TransactionPayment {
        id: Uuid::new_v4(),
        payment_no: 1,
        method: first.method,
        reference: format!("REF-{}", id.0),
        account: "Cash drawer 1".to_owned(),
        slip_file_key: None,
        amount_thb: money(dec(first.amount)),
        bank_fee_thb: money(dec(first.bank_fee)),
        paid_at: t,
    };
    let entity = ExchangeTransaction {
        id,
        organization_id: org_id,
        branch_id: BranchId::new(MAIN),
        mode: Mode::Live,
        status: TransactionStatus::Draft,
        document_ref: None,
        quote_id: None,
        booking_id: None,
        shift_id: Some(shift_id),
        submission_id: format!("sub-agg-{}", id.0),
        payload_hash: "hash".to_owned(),
        customer_id: "CU-AGG".to_owned(),
        customer_name: "Somchai".to_owned(),
        nationality: "THA".to_owned(),
        staff_id: staff.to_owned(),
        booking_no: "WALK-IN".to_owned(),
        notes: String::new(),
        value_date: NaiveDate::parse_from_str(DAY, "%Y-%m-%d").expect("a day"),
        recorded_at: t,
        recorded_by: "owner".to_owned(),
        completed_at: None,
        cancelled_at: None,
        cancel_reason: None,
        approval_request_id: None,
        passport_file_key: "PASSPORT-AGG".to_owned(),
        kiosk_request_id: None,
        staff_draft_id: None,
        count_confirmed: false,
        lines: vec![lines[0].line(1)],
        payment,
        receipts: vec![],
    };

    let store = PgTransactionStore::new(pool.clone());
    let mut db = pool.begin().await.expect("begin");
    store
        .insert_draft_in(&mut db, &entity)
        .await
        .expect("insert the draft");
    store
        .set_status_in(
            &mut db,
            id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Confirmed,
                reason: None,
                approval_request_id: None,
            },
            t,
        )
        .await
        .expect("confirm");
    store
        .complete_in(&mut db, id, "RECEIPT-AGG", t)
        .await
        .expect("complete");
    db.commit().await.expect("commit");

    for (i, l) in lines.iter().enumerate().skip(1) {
        let line = l.line(i as i32 + 1);
        sqlx::query(
            "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, \
                currency, quantity, unit_size, rate_snapshot, reference_rate, thb_amount, fee_thb, \
                settle_thb, line_no, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6::numeric, 1, $7::numeric, $8::numeric, $9::numeric, \
                $10::numeric, $11::numeric, $12, $13, $13)",
        )
        .bind(line.id)
        .bind(org_id)
        .bind(id.0)
        .bind(match l.side {
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        })
        .bind(l.currency)
        .bind(l.quantity)
        .bind(l.rate)
        .bind(l.reference)
        .bind(line.thb_amount.to_string())
        .bind(line.fee_thb.to_string())
        .bind(line.settle_thb.to_string())
        .bind(line.line_no)
        .bind(t)
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("insert line {}: {e}", line.line_no));
    }
    for (i, p) in payments.iter().enumerate().skip(1) {
        sqlx::query(
            "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, \
                method, reference, account, amount_thb, bank_fee_thb, paid_at, created_at, \
                updated_at) \
             VALUES ($1, $2, $3, $4, $5, '', 'Cash drawer 1', $6::numeric, $7::numeric, $8, $8, $8)",
        )
        .bind(Uuid::new_v4())
        .bind(org_id)
        .bind(id.0)
        .bind(i as i32 + 1)
        .bind(p.method.as_str())
        .bind(p.amount)
        .bind(p.bank_fee)
        .bind(t)
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("insert payment {}: {e}", i + 1));
    }
    id
}

fn channel<'a>(list: &'a Value, method: &str) -> &'a Value {
    list.as_array()
        .unwrap_or_else(|| panic!("a channel list: {list}"))
        .iter()
        .find(|c| c["method"] == method)
        .unwrap_or_else(|| panic!("channel {method} in {list}"))
}

fn employee<'a>(data: &'a Value, staff: &str) -> &'a Value {
    data["byEmployee"]
        .as_array()
        .unwrap_or_else(|| panic!("byEmployee: {data}"))
        .iter()
        .find(|e| e["staff"] == staff)
        .unwrap_or_else(|| panic!("employee {staff} in {}", data["byEmployee"]))
}

fn currency<'a>(data: &'a Value, code: &str) -> &'a Value {
    data["byCurrency"]
        .as_array()
        .unwrap_or_else(|| panic!("byCurrency: {data}"))
        .iter()
        .find(|c| c["currency"] == code)
        .unwrap_or_else(|| panic!("currency {code} in {}", data["byCurrency"]))
}

fn money_of(v: &Value) -> Decimal {
    dec(v.as_str().unwrap_or_else(|| panic!("a money string: {v}")))
}

/// One exchange's own figures, read from its child tables by the test.
#[derive(sqlx::FromRow)]
struct Own {
    staff: String,
    method: String,
    received: String,
    paid: String,
    fees: String,
    bank_fee: String,
    lines: i64,
}

fn total(own: &[Own], keep: impl Fn(&Own) -> bool, figure: impl Fn(&Own) -> Decimal) -> Decimal {
    own.iter().filter(|o| keep(o)).map(figure).sum()
}

fn fixed2(d: Decimal) -> String {
    let mut d = d;
    d.rescale(2);
    d.to_string()
}

// ── Row 1: two lines, one payment ────────────────────────────────────────────

/// Behaviour row 1 (rulings b, c): one COMPLETED exchange with two BUY lines (USD settle
/// 3,000.00 fee 10.00; EUR settle 2,000.00 fee 5.00) and one Cash payment with bank fee 15.00.
/// The bank fee is the exchange's, taken once; every count counts the exchange once.
/// Red on main: the join repeats `p.bank_fee_thb` on each line (`bankFees` = 30.00) and counts
/// each line as an exchange (count 2).
#[actix_web::test]
async fn two_lines_count_the_bank_fee_and_the_exchange_once() {
    let room = Room::open().await;
    seed(
        &room,
        "dao",
        &[
            L {
                side: Side::Buy,
                currency: "USD",
                quantity: "100",
                rate: "30.10",
                reference: None,
                fee: "10",
            },
            L {
                side: Side::Buy,
                currency: "EUR",
                quantity: "50",
                rate: "40.10",
                reference: None,
                fee: "5",
            },
        ],
        &[P {
            method: PaymentMethod::Cash,
            amount: "5000",
            bank_fee: "15",
        }],
    )
    .await;

    let data = room.report().await;

    assert_eq!(
        data["stats"],
        json!({ "received": "0.00", "paid": "5000.00", "fees": "15.00", "bankFees": "15.00" }),
        "stats: the bank fee once per exchange, the line figures once per line"
    );
    let cash = channel(&data["byChannel"], "Cash");
    assert_eq!(
        cash,
        &json!({ "method": "Cash", "count": 1, "received": "0.00", "paid": "5000.00", "net": "-5000.00" }),
        "byChannel Cash counts the exchange once"
    );
    let dao = employee(&data, "dao");
    assert_eq!(dao["count"], json!(1), "employee count = exchanges: {dao}");
    assert_eq!(
        dao["completed"],
        json!(1),
        "employee completed = exchanges: {dao}"
    );
    assert_eq!(dao["paid"], json!("5000.00"), "{dao}");
    assert_eq!(dao["fees"], json!("15.00"), "{dao}");
    assert_eq!(
        dao["bankFees"],
        json!("15.00"),
        "employee bank fee once: {dao}"
    );
    assert_eq!(
        channel(&dao["methods"], "Cash"),
        &json!({ "method": "Cash", "count": 1, "received": "0.00", "paid": "5000.00", "net": "-5000.00" }),
        "employee's Cash method counts the exchange once"
    );
    assert_eq!(
        data["employeeTotals"],
        json!({ "people": 1, "completed": 1, "received": "0.00", "paid": "5000.00", "net": "-5000.00" }),
        "employeeTotals counts the exchange once"
    );
    assert_eq!(
        currency(&data, "USD"),
        &json!({ "currency": "USD", "buyAmount": "100.00", "sellAmount": "0.00", "principal": "3010.00", "fees": "10.00" })
    );
    assert_eq!(
        currency(&data, "EUR"),
        &json!({ "currency": "EUR", "buyAmount": "50.00", "sellAmount": "0.00", "principal": "2005.00", "fees": "5.00" })
    );

    room.finish().await;
}

// ── Row 2: one line, two payments ────────────────────────────────────────────

/// Behaviour row 2 (rulings a, b, c): one COMPLETED exchange with one SELL line (USD 24.50 at
/// 40, fee 20.00, settle 1,000.00) and two payments: 1 = Cash 600.00 with bank fee 7.00,
/// 2 = PromptPay 400.00 with bank fee 3.00. The exchange's channel is payment 1's method, the
/// line is counted once, and the bank fee is payment 1's.
/// Red on main: the join without `payment_no = 1` repeats the line under each payment
/// (`received` = 2000.00, PromptPay count 1, bank fees 10.00).
#[actix_web::test]
async fn a_second_payment_does_not_repeat_the_lines() {
    let room = Room::open().await;
    seed(
        &room,
        "dao",
        &[L {
            side: Side::Sell,
            currency: "USD",
            quantity: "24.50",
            rate: "40",
            reference: None,
            fee: "20",
        }],
        &[
            P {
                method: PaymentMethod::Cash,
                amount: "600",
                bank_fee: "7",
            },
            P {
                method: PaymentMethod::PromptPay,
                amount: "400",
                bank_fee: "3",
            },
        ],
    )
    .await;

    let data = room.report().await;

    assert_eq!(
        data["stats"],
        json!({ "received": "1000.00", "paid": "0.00", "fees": "20.00", "bankFees": "7.00" }),
        "stats: the line once, the bank fee of payment 1 only"
    );
    assert_eq!(
        channel(&data["byChannel"], "Cash"),
        &json!({ "method": "Cash", "count": 1, "received": "1000.00", "paid": "0.00", "net": "1000.00" }),
        "the exchange's channel is payment 1's method"
    );
    assert_eq!(
        channel(&data["byChannel"], "PromptPay"),
        &json!({ "method": "PromptPay", "count": 0, "received": "0.00", "paid": "0.00", "net": "0.00" }),
        "a second payment's method is not a second exchange"
    );
    assert_eq!(
        data["byCurrency"],
        json!([{ "currency": "USD", "buyAmount": "0.00", "sellAmount": "24.50", "principal": "980.00", "fees": "20.00" }]),
        "byCurrency takes the line once"
    );
    assert_eq!(
        data["reference"],
        json!({ "referenced": 0, "unreferenced": 1, "value": "0.00", "spread": "0.00", "percent": null }),
        "reference takes the line once"
    );
    let dao = employee(&data, "dao");
    assert_eq!(dao["count"], json!(1), "{dao}");
    assert_eq!(dao["completed"], json!(1), "{dao}");
    assert_eq!(dao["received"], json!("1000.00"), "{dao}");
    assert_eq!(dao["fees"], json!("20.00"), "{dao}");
    assert_eq!(dao["bankFees"], json!("7.00"), "{dao}");
    assert_eq!(channel(&dao["methods"], "Cash")["count"], json!(1), "{dao}");
    assert_eq!(
        channel(&dao["methods"], "PromptPay"),
        &json!({ "method": "PromptPay", "count": 0, "received": "0.00", "paid": "0.00", "net": "0.00" }),
        "{dao}"
    );
    assert_eq!(
        dao["currencies"],
        json!([{ "currency": "USD", "received": "0.00", "paid": "24.50" }]),
        "the employee's currency figures take the line once"
    );
    assert_eq!(
        data["employeeTotals"],
        json!({ "people": 1, "completed": 1, "received": "1000.00", "paid": "0.00", "net": "1000.00" })
    );

    room.finish().await;
}

// ── Totals = each exchange's own figures, taken once ─────────────────────────

/// Ruling: the money totals equal the sum of each exchange's own figures, each taken once, and
/// every count counts exchanges. Four exchanges for two employees: a plain one, a two-line one,
/// a two-payment one (`SCB Transfer` first, so it folds into `Bank Transfer`), and one with two
/// lines and two payments. The expected figures are read from the child tables in this test,
/// one exchange at a time.
/// Red on main: the per-row join multiplies the line figures and the bank fees.
#[actix_web::test]
async fn totals_equal_each_exchange_summed_once() {
    let room = Room::open().await;
    let usd_buy = || L {
        side: Side::Buy,
        currency: "USD",
        quantity: "100",
        rate: "30",
        reference: Some("31"),
        fee: "10",
    };
    let eur_sell = || L {
        side: Side::Sell,
        currency: "EUR",
        quantity: "40",
        rate: "38",
        reference: None,
        fee: "12.50",
    };
    let cash = |bank_fee| P {
        method: PaymentMethod::Cash,
        amount: "1",
        bank_fee,
    };
    seed(&room, "dao", &[usd_buy()], &[cash("1.25")]).await;
    seed(&room, "dao", &[usd_buy(), eur_sell()], &[cash("2.50")]).await;
    seed(
        &room,
        "som",
        &[eur_sell()],
        &[
            P {
                method: PaymentMethod::ScbTransfer,
                amount: "1",
                bank_fee: "4",
            },
            cash("8"),
        ],
    )
    .await;
    seed(
        &room,
        "som",
        &[eur_sell(), usd_buy()],
        &[
            P {
                method: PaymentMethod::PromptPay,
                amount: "1",
                bank_fee: "16",
            },
            P {
                method: PaymentMethod::CreditCard,
                amount: "1",
                bank_fee: "32",
            },
        ],
    )
    .await;

    // Each exchange's own figures, summed once per exchange.
    let pool = &room.desk.db.pool;
    let own: Vec<Own> = sqlx::query_as(
        "SELECT t.staff_id AS staff, \
                (SELECT method FROM transaction_payments \
                   WHERE transaction_id = t.id AND payment_no = 1) AS method, \
                (SELECT COALESCE(sum(settle_thb) FILTER (WHERE shop_side = 'SELL'), 0)::text \
                   FROM transaction_lines WHERE transaction_id = t.id) AS received, \
                (SELECT COALESCE(sum(settle_thb) FILTER (WHERE shop_side = 'BUY'), 0)::text \
                   FROM transaction_lines WHERE transaction_id = t.id) AS paid, \
                (SELECT sum(fee_thb)::text FROM transaction_lines \
                   WHERE transaction_id = t.id) AS fees, \
                (SELECT bank_fee_thb::text FROM transaction_payments \
                   WHERE transaction_id = t.id AND payment_no = 1) AS bank_fee, \
                (SELECT count(*) FROM transaction_lines WHERE transaction_id = t.id) AS lines \
           FROM transactions t \
          WHERE t.branch_id = 'MAIN' AND t.status = 'COMPLETED' AND t.mode = 'LIVE'",
    )
    .fetch_all(pool)
    .await
    .expect("read each exchange's own figures");
    assert_eq!(own.len(), 4, "four exchanges seeded");

    let data = room.report().await;

    assert_eq!(
        data["stats"],
        json!({
            "received": fixed2(total(&own, |_| true, |o| dec(&o.received))),
            "paid": fixed2(total(&own, |_| true, |o| dec(&o.paid))),
            "fees": fixed2(total(&own, |_| true, |o| dec(&o.fees))),
            "bankFees": fixed2(total(&own, |_| true, |o| dec(&o.bank_fee))),
        }),
        "stats = each exchange's own figures, once (bank fee of payment 1)"
    );

    for method in [
        "Cash",
        "Bank Transfer",
        "Credit Card",
        "Alipay",
        "PromptPay",
    ] {
        let mine = |o: &Own| {
            let folded = if o.method == "SCB Transfer" {
                "Bank Transfer"
            } else {
                o.method.as_str()
            };
            folded == method
        };
        let count = own.iter().filter(|o| mine(o)).count() as u64;
        let received = total(&own, mine, |o| dec(&o.received));
        let paid = total(&own, mine, |o| dec(&o.paid));
        assert_eq!(
            channel(&data["byChannel"], method),
            &json!({
                "method": method,
                "count": count,
                "received": fixed2(received),
                "paid": fixed2(paid),
                "net": fixed2(received - paid),
            }),
            "byChannel {method}: payment 1's method, each exchange once"
        );
    }

    let mut total_completed = 0u64;
    for staff in ["dao", "som"] {
        let mine = |o: &Own| o.staff == staff;
        let count = own.iter().filter(|o| mine(o)).count() as u64;
        total_completed += count;
        let e = employee(&data, staff);
        assert_eq!(e["count"], json!(count), "{staff}: count = exchanges: {e}");
        assert_eq!(
            e["completed"],
            json!(count),
            "{staff}: completed = exchanges: {e}"
        );
        let received = total(&own, mine, |o| dec(&o.received));
        assert_eq!(e["received"], json!(fixed2(received)), "{staff}: {e}");
        let paid = total(&own, mine, |o| dec(&o.paid));
        assert_eq!(e["paid"], json!(fixed2(paid)), "{staff}: {e}");
        let fees = total(&own, mine, |o| dec(&o.fees));
        assert_eq!(e["fees"], json!(fixed2(fees)), "{staff}: {e}");
        let bank_fees = total(&own, mine, |o| dec(&o.bank_fee));
        assert_eq!(e["bankFees"], json!(fixed2(bank_fees)), "{staff}: {e}");
        let method_count: u64 = e["methods"]
            .as_array()
            .expect("methods")
            .iter()
            .map(|m| m["count"].as_u64().expect("a count"))
            .sum();
        assert_eq!(
            method_count, count,
            "{staff}: methods count each exchange once: {e}"
        );
    }
    assert_eq!(data["employeeTotals"]["people"], json!(2));
    assert_eq!(
        data["employeeTotals"]["completed"],
        json!(total_completed),
        "employeeTotals.completed = exchanges"
    );

    // The line sums of byCurrency equal the lines, each taken once.
    let lines: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT l.currency::text, sum(l.thb_amount)::text, sum(l.fee_thb)::text \
           FROM transaction_lines l JOIN transactions t ON t.id = l.transaction_id \
          WHERE t.branch_id = 'MAIN' AND t.status = 'COMPLETED' AND t.mode = 'LIVE' \
          GROUP BY l.currency",
    )
    .fetch_all(pool)
    .await
    .expect("read the line sums");
    assert_eq!(lines.len(), 2);
    for (code, principal, fees) in &lines {
        let c = currency(&data, code.trim());
        assert_eq!(money_of(&c["principal"]), dec(principal), "{code}: {c}");
        assert_eq!(money_of(&c["fees"]), dec(fees), "{code}: {c}");
    }
    let line_total: i64 = own.iter().map(|o| o.lines).sum();
    let referenced = data["reference"]["referenced"]
        .as_u64()
        .expect("referenced");
    let unreferenced = data["reference"]["unreferenced"]
        .as_u64()
        .expect("unreferenced");
    assert_eq!(
        (referenced + unreferenced) as i64,
        line_total,
        "reference counts each line once, never once per payment: {}",
        data["reference"]
    );

    room.finish().await;
}

// ── Pins: orders and the reference value ─────────────────────────────────────

/// Behaviour row 3 (pin, green on main): an employee's currencies are listed in the order the
/// employee first handled them, by `(t.seq, l.line_no)`: exchange A has line 1 = EUR and
/// line 2 = USD, a later exchange B has GBP, so the order is EUR, USD, GBP (not the code order
/// EUR, GBP, USD). BUY quantities are `received`, SELL quantities are `paid`.
#[actix_web::test]
async fn an_employees_currencies_keep_first_appearance_order() {
    let room = Room::open().await;
    seed(
        &room,
        "dao",
        &[
            L {
                side: Side::Buy,
                currency: "EUR",
                quantity: "10",
                rate: "35",
                reference: None,
                fee: "0",
            },
            L {
                side: Side::Sell,
                currency: "USD",
                quantity: "5",
                rate: "40",
                reference: None,
                fee: "0",
            },
        ],
        &[P {
            method: PaymentMethod::Cash,
            amount: "1",
            bank_fee: "0",
        }],
    )
    .await;
    seed(
        &room,
        "dao",
        &[L {
            side: Side::Buy,
            currency: "GBP",
            quantity: "3",
            rate: "40",
            reference: None,
            fee: "0",
        }],
        &[P {
            method: PaymentMethod::Cash,
            amount: "1",
            bank_fee: "0",
        }],
    )
    .await;

    let data = room.report().await;

    assert_eq!(
        employee(&data, "dao")["currencies"],
        json!([
            { "currency": "EUR", "received": "10.00", "paid": "0.00" },
            { "currency": "USD", "received": "0.00", "paid": "5.00" },
            { "currency": "GBP", "received": "3.00", "paid": "0.00" },
        ]),
        "first-appearance order by (seq, line_no)"
    );
    let codes: Vec<&str> = data["byCurrency"]
        .as_array()
        .expect("byCurrency")
        .iter()
        .map(|c| c["currency"].as_str().expect("a code"))
        .collect();
    assert_eq!(codes, ["EUR", "GBP", "USD"], "byCurrency in code order");

    room.finish().await;
}

/// Behaviour row 4 (pin, green on main): `byEmployee` is in byte order of `staff_id`, not the
/// database collation: `B-2` (0x42) comes before `a-1` (0x61).
#[actix_web::test]
async fn employees_sort_in_byte_order() {
    let room = Room::open().await;
    for staff in ["a-1", "B-2"] {
        seed(
            &room,
            staff,
            &[L {
                side: Side::Buy,
                currency: "USD",
                quantity: "1",
                rate: "30",
                reference: None,
                fee: "0",
            }],
            &[P {
                method: PaymentMethod::Cash,
                amount: "30",
                bank_fee: "0",
            }],
        )
        .await;
    }

    let data = room.report().await;

    let order: Vec<&str> = data["byEmployee"]
        .as_array()
        .expect("byEmployee")
        .iter()
        .map(|e| e["staff"].as_str().expect("a staff id"))
        .collect();
    assert_eq!(order, ["B-2", "a-1"], "byte order of staff_id");

    room.finish().await;
}

/// Behaviour row 5 (pin, green on main): the reference value is
/// `ExchangeArithmetic::thb(quantity, reference_rate, unit)` per line, rounded per line, then
/// summed. Three exchanges with the same quantity (USD 33.33), reference rate (31.115) and unit
/// (1): two BUY at 30 and one SELL at 32. Per line 33.33 × 31.115 = 1037.06295 → 1037.06, so
/// the value is 3 × 1037.06 = 3111.18 (rounding the summed quantity would give 3111.19).
/// Spread: BUY 1037.06 − 999.90 = 37.16 each, SELL 1066.56 − 1037.06 = 29.50, total 103.82;
/// percent 103.82 / 3111.18 × 100 = 3.3370 → 3.34.
#[actix_web::test]
async fn equal_reference_lines_sum_exactly() {
    let room = Room::open().await;
    for (side, rate) in [(Side::Buy, "30"), (Side::Buy, "30"), (Side::Sell, "32")] {
        seed(
            &room,
            "dao",
            &[L {
                side,
                currency: "USD",
                quantity: "33.33",
                rate,
                reference: Some("31.115"),
                fee: "0",
            }],
            &[P {
                method: PaymentMethod::Cash,
                amount: "1",
                bank_fee: "0",
            }],
        )
        .await;
    }

    let data = room.report().await;

    assert_eq!(
        data["reference"],
        json!({
            "referenced": 3,
            "unreferenced": 0,
            "value": "3111.18",
            "spread": "103.82",
            "percent": "3.34",
        }),
        "the reference value is rounded per line, then summed"
    );

    room.finish().await;
}
