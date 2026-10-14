//! Ticket #1131 (TX-8f, `docs/design/blueprint/quote-transaction.md` §3.2–§3.4, §4.1, §9):
//! the staff reports view (`api/src/presentation/http/v3/views/admin_reports.rs`) must count
//! COMPLETED exchanges only and read figures from child tables (`transaction_lines` and
//! `transaction_payments`), never from the wide `transactions` row.
//!
//! Written before the implementation by the test-writer (gemini/warut-3, rule A).
//!
//! §9 Edge cases covered here:
//! - The four seeded rows: every report counts the completed one only.
//! - By channel: the payment's `method` (read from `transaction_payments.method`).
//! - The compat-saved exchange: every report equals today's snapshot, ignoring uncompleted rows.
//! - Figures (`principal`, `fee`, `bank_fee`, `received`/`paid`) read from child rows.

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
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_exchange::application::customer::service::CustomerService;
use xc_exchange::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::repository::{StatusMove, TransactionStore};
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};

const ORIGIN: &str = "http://desk.test";
#[allow(dead_code)]
const RPC: &str = "/api/rpc";
const REPORTS: &str = "admin_reports";
const MAIN: &str = "MAIN";

const RATES: [(&str, &str, &str); 4] = [
    ("USD", "30", "40"),
    ("EUR", "35", "38"),
    ("JPY", "0.2", "0.25"),
    ("GBP", "40", "45"),
];

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[allow(dead_code)]
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

fn view_path(query: &str) -> String {
    if query.is_empty() {
        format!("/api/v3/views/{REPORTS}")
    } else {
        format!("/api/v3/views/{REPORTS}?{query}")
    }
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
            room.set_rate(MAIN, code, buy, sell).await;
        }
        room.open_shift(MAIN).await;
        room
    }

    fn tick(&self) -> DateTime<Utc> {
        let t = self.t0 + Duration::seconds(next() as i64);
        self.desk.clock.set(t);
        t
    }

    fn cookie(&self, who: &str) -> String {
        self.cookies[who].clone()
    }

    async fn get(&self, who: &str, path: &str) -> Answer {
        let call = get(path).cookie(&self.cookie(who));
        self.app.send(call).await
    }

    async fn data(&self, who: &str, query: &str) -> Value {
        let path = view_path(query);
        let answer = self.get(who, &path).await;
        assert_eq!(
            answer.status, 200,
            "GET {path} as {who:?}: {:.400}",
            answer.text
        );
        let body = answer.json();
        body.get("data")
            .cloned()
            .unwrap_or_else(|| panic!("GET {path}: data in {body}"))
    }

    async fn set_rate(&self, branch: &str, code: &str, buy: &str, sell: &str) {
        rates_re4e_publish::set_counter_rate(&self.desk.db.pool, branch, code, buy, sell).await;
    }

    async fn open_shift(&self, branch: &str) {
        let t = self.tick();
        shift_setup_common::open_shift_by_count(
            &self.desk.db.pool,
            branch,
            json!({
                "THB": 1000000, "USD": 100000, "EUR": 100000, "JPY": 10000000
            }),
            t,
        )
        .await;
    }

    /// Saves a compat transaction (writing both wide and child rows with identical figures).
    #[allow(clippy::too_many_arguments)]
    async fn save_compat(
        &self,
        branch: &str,
        side: &str,
        currency: &str,
        amount: &str,
        fee: &str,
        bank_fee: &str,
        method: &str,
        value_date: &str,
    ) -> String {
        let t = self.tick();
        let here = BranchId::new(branch);
        let person = ReserveCustomer {
            name: "Somchai Jaidee".to_owned(),
            passport_no: "AA1234567".to_owned(),
            nationality: "THA".to_owned(),
            birth_date: String::new(),
            passport_expiry: String::new(),
            mode: "LIVE".to_owned(),
        };
        let customer = CustomerService::new(self.desk.db.branch_tx())
            .reserve(&here, person.clone(), t)
            .await
            .expect("reserve the customer")
            .customer_id;
        let passport = self
            .desk
            .files
            .upload(&here, "owner", "image/jpeg", JPEG, t)
            .await
            .expect("upload a passport");
        let (_, buy, sell) = RATES
            .iter()
            .find(|(code, _, _)| *code == currency)
            .copied()
            .unwrap_or_else(|| panic!("a counter rate for {currency}"));
        let rate = if side == "BUY" { buy } else { sell };
        let cmd = SaveTransaction {
            customer: person,
            submission_id: format!("submission-{branch}-{:012}", next()),
            customer_id: customer,
            side: side.to_owned(),
            currency: currency.to_owned(),
            amount: amount.to_owned(),
            rate: rate.to_owned(),
            fee: fee.to_owned(),
            bank_fee: bank_fee.to_owned(),
            method: method.to_owned(),
            payment_ref: String::new(),
            account: "Cash drawer 1".to_owned(),
            staff: "dao".to_owned(),
            booking: "WALK-IN".to_owned(),
            notes: String::new(),
            value_date: value_date.to_owned(),
            paid_at: "2026-10-06T05:30:00.000Z".to_owned(),
            payment_confirmed: true,
            identity_confirmed: true,
            passport: Some(passport),
            slip: None,
            reference_rate: None,
        };
        self.desk
            .workflow
            .save(&here, cmd, "owner", t)
            .await
            .unwrap_or_else(|e| panic!("save an exchange: {e:?}"))
            .document_ref
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// Seeds an exchange through TX-2's PgTransactionStore at the requested status.
#[allow(clippy::too_many_arguments)]
async fn seed_store_tx(
    room: &Room,
    status: TransactionStatus,
    side: Side,
    currency: &str,
    quantity: &str,
    rate: &str,
    fee: &str,
    bank_fee: &str,
    method: PaymentMethod,
    reference_rate: Option<&str>,
    value_date: &str,
) -> TransactionId {
    let pool = &room.desk.db.pool;
    let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
        .fetch_one(pool)
        .await
        .expect("read MAIN organization_id");

    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ('CU-SEED', $1, 'CU-SEED', 'Somchai', 'LIVE', now()) ON CONFLICT DO NOTHING",
    )
    .bind(org_id)
    .execute(pool)
    .await
    .expect("seed customer");

    sqlx::query(
        "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
         VALUES ('PASSPORT-MAIN', 'MAIN', 'owner', 'application/pdf', 'sha-p', now()) ON CONFLICT DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("seed passport");

    sqlx::query(
        "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
         VALUES ('RECEIPT-SEED', 'MAIN', 'owner', 'application/pdf', 'sha-r', now()) ON CONFLICT DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("seed receipt");

    let id = TransactionId(Uuid::new_v4());
    let qty_dec = Decimal::from_str(quantity).unwrap();
    let rate_val = Rate::parse(rate).unwrap();
    let thb_amount = qty_dec * Decimal::from_str(rate).unwrap();
    let fee_dec = Decimal::from_str(fee).unwrap();
    let bank_fee_dec = Decimal::from_str(bank_fee).unwrap();
    let settle_dec = match side {
        Side::Buy => thb_amount - fee_dec,
        Side::Sell => thb_amount + fee_dec,
    };

    let ref_rate = reference_rate.map(|r| Rate::parse(r).unwrap());

    let line = TransactionLine {
        id: Uuid::new_v4(),
        quote_line_id: None,
        shop_side: side,
        currency: CurrencyCode::parse(currency).unwrap(),
        quantity: ForeignAmount::from_decimal(qty_dec),
        unit_size: UnitSize::parse("1").unwrap(),
        rate_snapshot: rate_val,
        reference_rate: ref_rate,
        thb_amount: Money::parse(&format!("{thb_amount:.2}")).unwrap(),
        fee_thb: Money::parse(&format!("{fee_dec:.2}")).unwrap(),
        settle_thb: Money::parse(&format!("{settle_dec:.2}")).unwrap(),
        cost_basis_thb: None,
        realized_margin_thb: None,
        line_no: 1,
    };

    let payment = TransactionPayment {
        id: Uuid::new_v4(),
        payment_no: 1,
        method,
        reference: format!("REF-{}", id.0),
        account: "Cash drawer 1".to_owned(),
        slip_file_key: None,
        amount_thb: Money::parse(&format!("{settle_dec:.2}")).unwrap(),
        bank_fee_thb: Money::parse(&format!("{bank_fee_dec:.2}")).unwrap(),
        paid_at: room.t0,
    };

    let shift_id: String = sqlx::query_scalar(
        "SELECT id FROM shifts WHERE branch_id = 'MAIN' AND closed_at IS NULL ORDER BY opened_at DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("read open shift");

    let tx_entity = ExchangeTransaction {
        id,
        organization_id: org_id,
        branch_id: BranchId::new("MAIN"),
        mode: Mode::Live,
        status: TransactionStatus::Draft,
        document_ref: None,
        quote_id: None,
        booking_id: None,
        shift_id: Some(shift_id),
        submission_id: format!("sub-{}", id.0),
        payload_hash: "hash".to_owned(),
        customer_id: "CU-SEED".to_owned(),
        customer_name: "Somchai".to_owned(),
        nationality: "THA".to_owned(),
        staff_id: "dao".to_owned(),
        booking_no: "WALK-IN".to_owned(),
        notes: "seed note".to_owned(),
        value_date: NaiveDate::parse_from_str(value_date, "%Y-%m-%d").unwrap(),
        recorded_at: room.t0,
        recorded_by: "owner".to_owned(),
        completed_at: None,
        cancelled_at: None,
        cancel_reason: None,
        approval_request_id: None,
        passport_file_key: "PASSPORT-MAIN".to_owned(),
        kiosk_request_id: None,
        staff_draft_id: None,
        count_confirmed: false,
        lines: vec![line],
        payment,
        receipts: vec![],
    };

    let store = PgTransactionStore::new(pool.clone());
    let mut db_tx = pool.begin().await.unwrap();
    store
        .insert_draft_in(&mut db_tx, &tx_entity)
        .await
        .expect("insert draft");
    db_tx.commit().await.unwrap();

    match status {
        TransactionStatus::Draft => {}
        TransactionStatus::Confirmed => {
            let mut db_tx = pool.begin().await.unwrap();
            store
                .set_status_in(
                    &mut db_tx,
                    id,
                    StatusMove {
                        from: TransactionStatus::Draft,
                        to: TransactionStatus::Confirmed,
                        reason: None,
                        approval_request_id: None,
                    },
                    room.t0,
                )
                .await
                .expect("move to confirmed");
            db_tx.commit().await.unwrap();
        }
        TransactionStatus::Cancelled => {
            let mut db_tx = pool.begin().await.unwrap();
            store
                .set_status_in(
                    &mut db_tx,
                    id,
                    StatusMove {
                        from: TransactionStatus::Draft,
                        to: TransactionStatus::Cancelled,
                        reason: Some("customer cancelled".to_owned()),
                        approval_request_id: None,
                    },
                    room.t0,
                )
                .await
                .expect("move to cancelled");
            db_tx.commit().await.unwrap();
        }
        TransactionStatus::Completed => {
            let mut db_tx = pool.begin().await.unwrap();
            store
                .set_status_in(
                    &mut db_tx,
                    id,
                    StatusMove {
                        from: TransactionStatus::Draft,
                        to: TransactionStatus::Confirmed,
                        reason: None,
                        approval_request_id: None,
                    },
                    room.t0,
                )
                .await
                .expect("move to confirmed");
            db_tx.commit().await.unwrap();

            let mut db_tx = pool.begin().await.unwrap();
            store
                .complete_in(&mut db_tx, id, "RECEIPT-SEED", room.t0)
                .await
                .expect("complete");
            db_tx.commit().await.unwrap();
        }
        TransactionStatus::PendingApproval => {
            panic!("not used here");
        }
    }

    id
}

// ── Edge Case 1: The four seeded rows ────────────────────────────────────────

/// Ticket #1131 §9: "The four seeded rows: every report counts the completed one only."
/// Seeding 1 COMPLETED, 1 DRAFT, 1 CONFIRMED and 1 CANCELLED row through TX-2's store.
/// The report must count ONLY the COMPLETED row across all sections (stats, byChannel,
/// byCurrency, reference, byEmployee, employeeTotals).
#[actix_web::test]
async fn the_four_seeded_rows_every_report_counts_the_completed_one_only() {
    let room = Room::open().await;

    // Seed 4 rows for staff "dao" on 2026-10-06:
    // 1. COMPLETED: BUY 100 USD at 30, fee 10, ref_rate 31, paid 2990.00
    seed_store_tx(
        &room,
        TransactionStatus::Completed,
        Side::Buy,
        "USD",
        "100",
        "30",
        "10",
        "0",
        PaymentMethod::Cash,
        Some("31"),
        "2026-10-06",
    )
    .await;

    // 2. DRAFT (must be ignored)
    seed_store_tx(
        &room,
        TransactionStatus::Draft,
        Side::Buy,
        "USD",
        "100",
        "30",
        "10",
        "0",
        PaymentMethod::Cash,
        Some("31"),
        "2026-10-06",
    )
    .await;

    // 3. CONFIRMED (must be ignored)
    seed_store_tx(
        &room,
        TransactionStatus::Confirmed,
        Side::Buy,
        "USD",
        "100",
        "30",
        "10",
        "0",
        PaymentMethod::Cash,
        None,
        "2026-10-06",
    )
    .await;

    // 4. CANCELLED (must be ignored)
    seed_store_tx(
        &room,
        TransactionStatus::Cancelled,
        Side::Buy,
        "USD",
        "100",
        "30",
        "10",
        "0",
        PaymentMethod::Cash,
        Some("31"),
        "2026-10-06",
    )
    .await;

    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-06&to=2026-10-06")
        .await;

    // Stats must reflect ONLY the 1 completed exchange (2990.00, not 4 x 2990 = 11960.00)
    assert_eq!(
        data["stats"]["paid"],
        json!("2990.00"),
        "stats.paid must count completed only"
    );
    assert_eq!(
        data["stats"]["fees"],
        json!("10.00"),
        "stats.fees must count completed only"
    );

    // byChannel: Cash count must be 1, not 4
    let cash_ch = data["byChannel"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["method"] == "Cash")
        .unwrap();
    assert_eq!(cash_ch["count"], json!(1), "byChannel Cash count must be 1");
    assert_eq!(cash_ch["paid"], json!("2990.00"));

    // employeeTotals: completed must be 1, not 4
    assert_eq!(
        data["employeeTotals"]["completed"],
        json!(1),
        "employeeTotals.completed must be 1"
    );

    // byEmployee: count and completed must be 1, not 4
    let emp = &data["byEmployee"][0];
    assert_eq!(emp["count"], json!(1), "byEmployee count must be 1");
    assert_eq!(emp["completed"], json!(1), "byEmployee completed must be 1");
    assert_eq!(emp["paid"], json!("2990.00"));

    // byCurrency: must carry only the completed exchange figures (USD)
    let by_curr = data["byCurrency"].as_array().expect("byCurrency array");
    assert_eq!(by_curr.len(), 1, "byCurrency must have only 1 currency");
    let usd = &by_curr[0];
    assert_eq!(usd["currency"], json!("USD"));
    assert_eq!(
        usd["buyAmount"],
        json!("100.00"),
        "byCurrency.buyAmount must count completed only"
    );
    assert_eq!(
        usd["sellAmount"],
        json!("0.00"),
        "byCurrency.sellAmount must count completed only"
    );
    assert_eq!(
        usd["principal"],
        json!("3000.00"),
        "byCurrency.principal must count completed only"
    );
    assert_eq!(
        usd["fees"],
        json!("10.00"),
        "byCurrency.fees must count completed only"
    );

    // reference: must count the completed row only (1 referenced, 0 unreferenced)
    let ref_sec = &data["reference"];
    assert_eq!(
        ref_sec["referenced"],
        json!(1),
        "reference.referenced must count completed only"
    );
    assert_eq!(
        ref_sec["unreferenced"],
        json!(0),
        "reference.unreferenced must count completed only"
    );
    assert_eq!(
        ref_sec["value"],
        json!("3100.00"),
        "reference.value must count completed only"
    );
    assert_eq!(
        ref_sec["spread"],
        json!("100.00"),
        "reference.spread must count completed only"
    );
    assert_eq!(
        ref_sec["percent"],
        json!("3.23"),
        "reference.percent must count completed only"
    );

    room.finish().await;
}

// ── Edge Case 2: By channel: the payment's method ────────────────────────────

/// Ticket #1131 §9: "By channel: the payment's method."
/// A completed exchange has its method in `transaction_payments.method`.
/// The reader must read from the payment table, not `transactions.method`.
#[actix_web::test]
async fn by_channel_reads_the_payments_method() {
    let room = Room::open().await;

    // Seed a COMPLETED exchange whose payment is PromptPay
    let tx_id = seed_store_tx(
        &room,
        TransactionStatus::Completed,
        Side::Sell,
        "USD",
        "50",
        "40",
        "20",
        "0",
        PaymentMethod::PromptPay,
        None,
        "2026-10-06",
    )
    .await;

    // Intentionally corrupt the wide row's method to 'Cash' to prove the view
    // reads transaction_payments.method, not transactions.method.
    sqlx::query("UPDATE transactions SET method = 'Cash' WHERE id = $1")
        .bind(tx_id.0)
        .execute(&room.desk.db.pool)
        .await
        .expect("corrupt wide method");

    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-06&to=2026-10-06")
        .await;

    // byChannel: PromptPay must have count 1, and Cash must have count 0
    let promptpay_ch = data["byChannel"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["method"] == "PromptPay")
        .unwrap();
    assert_eq!(
        promptpay_ch["count"],
        json!(1),
        "PromptPay count must be 1 (from transaction_payments.method)"
    );
    assert_eq!(
        promptpay_ch["received"],
        json!("2020.00"),
        "PromptPay received must be 2020.00"
    );

    let cash_ch = data["byChannel"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["method"] == "Cash")
        .unwrap();
    assert_eq!(
        cash_ch["count"],
        json!(0),
        "Cash count must be 0 (wide row method was ignored)"
    );

    room.finish().await;
}

// ── Edge Case 3: The compat-saved exchange equals today's snapshot ───────────

/// Ticket #1131 §9: "The compat-saved exchange: every report equals today's (a snapshot taken on main)."
/// A compat-saved exchange alongside uncompleted rows must produce exactly today's report
/// for that exchange alone, ignoring uncompleted DRAFT, CONFIRMED, and CANCELLED rows.
#[actix_web::test]
async fn the_compat_saved_exchange_every_report_equals_todays_snapshot() {
    let room = Room::open().await;

    // Seed 1 compat-saved exchange: BUY 100 USD at 30, fee 10, Cash paid 2990.00
    room.save_compat(MAIN, "BUY", "USD", "100", "10", "0", "Cash", "2026-10-06")
        .await;

    // Snapshot taken on origin/main for the single compat-saved exchange alone
    let snapshot = room
        .data("OWNER", "branch=MAIN&from=2026-10-06&to=2026-10-06")
        .await;

    let expected_snapshot = json!({
        "period": { "from": "2026-10-06", "to": "2026-10-06" },
        "stats": { "received": "0.00", "paid": "2990.00", "fees": "10.00", "bankFees": "0.00" },
        "byChannel": [
            { "method": "Cash", "count": 1, "received": "0.00", "paid": "2990.00", "net": "-2990.00" },
            { "method": "Bank Transfer", "count": 0, "received": "0.00", "paid": "0.00", "net": "0.00" },
            { "method": "Credit Card", "count": 0, "received": "0.00", "paid": "0.00", "net": "0.00" },
            { "method": "Alipay", "count": 0, "received": "0.00", "paid": "0.00", "net": "0.00" },
            { "method": "PromptPay", "count": 0, "received": "0.00", "paid": "0.00", "net": "0.00" }
        ],
        "byCurrency": [
            { "currency": "USD", "buyAmount": "100.00", "sellAmount": "0.00", "principal": "3000.00", "fees": "10.00" }
        ],
        "reference": { "referenced": 0, "unreferenced": 1, "value": "0.00", "spread": "0.00", "percent": null },
        "byEmployee": [
            {
                "staff": "dao",
                "count": 1,
                "completed": 1,
                "pending": 0,
                "cancelled": 0,
                "received": "0.00",
                "paid": "2990.00",
                "net": "-2990.00",
                "fees": "10.00",
                "bankFees": "0.00",
                "methods": [
                    { "method": "Cash", "count": 1, "received": "0.00", "paid": "2990.00", "net": "-2990.00" },
                    { "method": "Credit Card", "count": 0, "received": "0.00", "paid": "0.00", "net": "0.00" },
                    { "method": "Alipay", "count": 0, "received": "0.00", "paid": "0.00", "net": "0.00" },
                    { "method": "PromptPay", "count": 0, "received": "0.00", "paid": "0.00", "net": "0.00" },
                    { "method": "Bank Transfer", "count": 0, "received": "0.00", "paid": "0.00", "net": "0.00" }
                ],
                "currencies": [
                    { "currency": "USD", "received": "100.00", "paid": "0.00" }
                ]
            }
        ],
        "employeeTotals": { "people": 1, "completed": 1, "received": "0.00", "paid": "2990.00", "net": "-2990.00" }
    });
    assert_eq!(
        snapshot, expected_snapshot,
        "snapshot taken on origin/main matches expected"
    );

    // ALSO seed uncompleted rows through TX-2's store on the same date/branch
    seed_store_tx(
        &room,
        TransactionStatus::Draft,
        Side::Buy,
        "USD",
        "50",
        "30",
        "5",
        "0",
        PaymentMethod::Cash,
        None,
        "2026-10-06",
    )
    .await;
    seed_store_tx(
        &room,
        TransactionStatus::Confirmed,
        Side::Buy,
        "USD",
        "50",
        "30",
        "5",
        "0",
        PaymentMethod::Cash,
        None,
        "2026-10-06",
    )
    .await;
    seed_store_tx(
        &room,
        TransactionStatus::Cancelled,
        Side::Buy,
        "USD",
        "50",
        "30",
        "5",
        "0",
        PaymentMethod::Cash,
        None,
        "2026-10-06",
    )
    .await;

    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-06&to=2026-10-06")
        .await;

    // The report with uncompleted rows present must equal the origin/main snapshot
    assert_eq!(
        data, expected_snapshot,
        "every report must equal today's snapshot on main, ignoring uncompleted rows"
    );

    room.finish().await;
}

// ── Child Table Reads: figures read from lines and payments ──────────────────

/// Ticket #1131 §3 & §4.3: Figures must be read from `transaction_lines` (thb_amount, fee_thb,
/// settle_thb, shop_side, currency, quantity, rate_snapshot, reference_rate, unit_size) and
/// `transaction_payments` (bank_fee_thb, method), not the wide `transactions` columns.
#[actix_web::test]
async fn reports_read_figures_from_lines_and_payments_not_wide_columns() {
    let room = Room::open().await;

    // 1. Seed a COMPLETED BUY exchange:
    // BUY 100 USD at 30, fee 10, bank_fee 5.50, BankTransfer, ref_rate 32.00
    // Child line: shop_side=Buy, currency=USD, quantity=100, unit_size=1, rate=30, ref_rate=32, thb=3000, fee=10, settle=2990
    // Child payment: method=BankTransfer, amount_thb=2990, bank_fee=5.50
    let buy_id = seed_store_tx(
        &room,
        TransactionStatus::Completed,
        Side::Buy,
        "USD",
        "100",
        "30",
        "10",
        "5.50",
        PaymentMethod::BankTransfer,
        Some("32"),
        "2026-10-06",
    )
    .await;

    // 2. Seed a COMPLETED SELL exchange:
    // SELL 10000 JPY at 0.25, fee 20, bank_fee 0, Cash, ref_rate 0.24
    // Child line: shop_side=Sell, currency=JPY, quantity=10000, unit_size=1, rate=0.25, ref_rate=0.24, thb=2500, fee=20, settle=2520
    // Child payment: method=Cash, amount_thb=2520, bank_fee=0
    let sell_id = seed_store_tx(
        &room,
        TransactionStatus::Completed,
        Side::Sell,
        "JPY",
        "10000",
        "0.25",
        "20",
        "0",
        PaymentMethod::Cash,
        Some("0.24"),
        "2026-10-06",
    )
    .await;

    // Corrupt / desync wide columns in `transactions`:
    // For BUY: set wide type='SELL', code='EUR', amount=1, rate=1, reference_rate=NULL, unit_size=1000,
    //          principal=0, fee=0, bank_fee=0, paid=0, received=99999
    sqlx::query(
        "UPDATE transactions \
         SET principal = 0, fee = 0, bank_fee = 0, paid = 0, received = 99999, \
             type = 'SELL', code = 'EUR', amount = 1, rate = 1, reference_rate = NULL, unit_size = 1000 \
         WHERE id = $1",
    )
    .bind(buy_id.0)
    .execute(&room.desk.db.pool)
    .await
    .expect("corrupt wide columns for buy");

    // For SELL: set wide type='BUY', code='GBP', amount=1, rate=1, reference_rate=NULL, unit_size=1000,
    //           principal=0, fee=0, bank_fee=0, paid=99999, received=0
    sqlx::query(
        "UPDATE transactions \
         SET principal = 0, fee = 0, bank_fee = 0, paid = 99999, received = 0, \
             type = 'BUY', code = 'GBP', amount = 1, rate = 1, reference_rate = NULL, unit_size = 1000 \
         WHERE id = $1",
    )
    .bind(sell_id.0)
    .execute(&room.desk.db.pool)
    .await
    .expect("corrupt wide columns for sell");

    let data = room
        .data("OWNER", "branch=MAIN&from=2026-10-06&to=2026-10-06")
        .await;

    // 1. Stats: paid from BUY settle_thb, received from SELL settle_thb
    assert_eq!(
        data["stats"]["paid"],
        json!("2990.00"),
        "stats.paid must be read from line settle_thb"
    );
    assert_eq!(
        data["stats"]["received"],
        json!("2520.00"),
        "stats.received must be read from line settle_thb"
    );
    assert_eq!(
        data["stats"]["fees"],
        json!("30.00"),
        "stats.fees must be read from line fee_thb (10 + 20)"
    );
    assert_eq!(
        data["stats"]["bankFees"],
        json!("5.50"),
        "stats.bankFees must be read from payment bank_fee_thb"
    );

    // 2. byCurrency: must read line currency (USD, JPY) not wide (EUR, GBP),
    //    and line quantity & thb_amount & fee_thb, following shop_side
    let by_currency = data["byCurrency"].as_array().expect("byCurrency array");
    assert!(
        by_currency
            .iter()
            .all(|c| c["currency"] != "EUR" && c["currency"] != "GBP"),
        "byCurrency must not read wide currency codes EUR or GBP"
    );

    let usd = by_currency
        .iter()
        .find(|c| c["currency"] == "USD")
        .expect("USD currency report");
    assert_eq!(
        usd["buyAmount"],
        json!("100.00"),
        "USD buyAmount from line quantity"
    );
    assert_eq!(usd["sellAmount"], json!("0.00"), "USD sellAmount must be 0");
    assert_eq!(
        usd["principal"],
        json!("3000.00"),
        "USD principal from line thb_amount"
    );
    assert_eq!(usd["fees"], json!("10.00"), "USD fees from line fee_thb");

    let jpy = by_currency
        .iter()
        .find(|c| c["currency"] == "JPY")
        .expect("JPY currency report");
    assert_eq!(jpy["buyAmount"], json!("0.00"), "JPY buyAmount must be 0");
    assert_eq!(
        jpy["sellAmount"],
        json!("10000.00"),
        "JPY sellAmount from line quantity"
    );
    assert_eq!(
        jpy["principal"],
        json!("2500.00"),
        "JPY principal from line thb_amount"
    );
    assert_eq!(jpy["fees"], json!("20.00"), "JPY fees from line fee_thb");

    // 3. reference: computed from lines (quantity, reference_rate, unit_size)
    // BUY:  val = 100 * 32 = 3200, spread = 3200 - 3000 = 200
    // SELL: val = 10000 * 0.24 = 2400, spread = 2500 - 2400 = 100
    // Total val = 5600.00, total spread = 300.00, percent = 300/5600*100 = 5.36
    let ref_sec = &data["reference"];
    assert_eq!(
        ref_sec["referenced"],
        json!(2),
        "reference.referenced must count both lines"
    );
    assert_eq!(
        ref_sec["unreferenced"],
        json!(0),
        "reference.unreferenced must be 0"
    );
    assert_eq!(
        ref_sec["value"],
        json!("5600.00"),
        "reference.value from lines"
    );
    assert_eq!(
        ref_sec["spread"],
        json!("300.00"),
        "reference.spread from lines"
    );
    assert_eq!(
        ref_sec["percent"],
        json!("5.36"),
        "reference.percent from lines"
    );

    room.finish().await;
}
