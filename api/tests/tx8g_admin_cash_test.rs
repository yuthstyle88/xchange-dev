//! Ticket #1132 (TX-8g, `docs/design/blueprint/quote-transaction.md` §3.2–§3.4, §4.1, §9):
//! admin_cash view reads completed exchanges only, and never reads wide columns.
//! Written before the implementation by the test-writer (gemini/warut-5).
//!
//! §9 Edge cases:
//! - A shift with the four seeded rows, all paid in cash: cash in/out from the completed one only.
//! - A completed non-cash exchange is not in the cash sums.
//! - The compat-saved exchange gives today's figures.

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
mod rpc_common;
mod snapshot_common;

use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, Utc};
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
const CASH: &str = "admin_cash";
const MAIN: &str = "MAIN";

const BUY_RATE: &str = "30";
const SELL_RATE: &str = "40";

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];

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

#[derive(Clone, Copy)]
struct Ex {
    side: &'static str,
    amount: &'static str,
    fee: &'static str,
    method: &'static str,
    reference: Option<&'static str>,
}

impl Ex {
    fn sell(amount: &'static str, fee: &'static str) -> Ex {
        Ex {
            side: "SELL",
            amount,
            fee,
            method: "Cash",
            reference: None,
        }
    }

    fn reference(self, rate: &'static str) -> Ex {
        Ex {
            reference: Some(rate),
            ..self
        }
    }
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    t0: DateTime<Utc>,
    store: PgTransactionStore,
    org_id: Uuid,
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

        rates_re4e_publish::set_counter_rate(&desk.db.pool, MAIN, "USD", BUY_RATE, SELL_RATE).await;

        let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
            .bind(MAIN)
            .fetch_one(&desk.db.pool)
            .await
            .expect("branch organization_id");

        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('CU-1', $1, 'CU-1', 'Somchai', 'LIVE', now()) \
             ON CONFLICT DO NOTHING",
        )
        .bind(org_id)
        .execute(&desk.db.pool)
        .await
        .expect("seed customer");

        for key in [
            "PASSPORT-MAIN",
            "RECEIPT-1",
            "RECEIPT-2",
            "RECEIPT-3",
            "RECEIPT-4",
            "RECEIPT-5",
            "SLIP-1",
        ] {
            sqlx::query(
                "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ($1, 'MAIN', 'owner', 'application/pdf', 'sha', now()) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(key)
            .execute(&desk.db.pool)
            .await
            .expect("seed file");
        }

        let store = PgTransactionStore::new(desk.db.pool.clone());

        Room {
            desk,
            app,
            cookies,
            t0,
            store,
            org_id,
        }
    }

    fn tick(&self) -> DateTime<Utc> {
        let t = self.t0 + Duration::seconds(next() as i64);
        self.desk.clock.set(t);
        t
    }

    async fn get(&self, who: &str, path: &str) -> Answer {
        let call = get(path).cookie(&self.cookies[who]);
        self.app.send(call).await
    }

    async fn cash(&self, query: &str) -> Value {
        let path = if query.is_empty() {
            format!("/api/v3/views/{CASH}")
        } else {
            format!("/api/v3/views/{CASH}?{query}")
        };
        let answer = self.get("OWNER", &path).await;
        assert_eq!(answer.status, 200, "GET {path}: {}", answer.text);
        let body = answer.json();
        assert_eq!(body["changed"], json!(true), "GET {path}: {body}");
        body.get("data")
            .cloned()
            .unwrap_or_else(|| panic!("data in {body}"))
    }

    async fn open_shift(&self, branch: &str) -> String {
        let req = request_id();
        let answer = self
            .app
            .send(
                Call::action(
                    ORIGIN,
                    "/api/rpc",
                    "adminOpenShift",
                    json!({
                        "branchId": branch,
                        "requestId": req,
                        "opening": { "THB": 100000, "USD": 1000 }
                    }),
                )
                .cookie(&self.cookies["OWNER"]),
            )
            .await;
        assert_eq!(answer.status, 200, "adminOpenShift: {}", answer.text);

        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = $1 AND status = 'OPEN'")
            .bind(branch)
            .fetch_one(&self.desk.db.pool)
            .await
            .expect("open shift id")
    }

    async fn insert_draft(&self, t: &ExchangeTransaction) {
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        self.store.insert_draft_in(&mut tx, t).await.unwrap();
        tx.commit().await.unwrap();
    }

    async fn move_status(
        &self,
        id: TransactionId,
        from: TransactionStatus,
        to: TransactionStatus,
        reason: Option<&str>,
        at: DateTime<Utc>,
    ) {
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        self.store
            .set_status_in(
                &mut tx,
                id,
                StatusMove {
                    from,
                    to,
                    reason: reason.map(str::to_owned),
                    approval_request_id: None,
                },
                at,
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    async fn complete_tx(&self, id: TransactionId, receipt_key: &str, at: DateTime<Utc>) {
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        self.store
            .complete_in(&mut tx, id, receipt_key, at)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    async fn save(&self, branch: &str, ex: Ex) -> String {
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
        let rate = if ex.side == "BUY" {
            BUY_RATE
        } else {
            SELL_RATE
        };
        let cash = ex.method == "Cash";
        let slip = if cash {
            None
        } else {
            Some(
                self.desk
                    .files
                    .upload(&here, "owner", "image/jpeg", JPEG, t)
                    .await
                    .expect("upload a slip"),
            )
        };
        let cmd = SaveTransaction {
            customer: person,
            submission_id: format!("submission-{branch}-{:012}", next()),
            customer_id: customer,
            side: ex.side.to_owned(),
            currency: "USD".to_owned(),
            amount: ex.amount.to_owned(),
            rate: rate.to_owned(),
            fee: ex.fee.to_owned(),
            bank_fee: "0".to_owned(),
            method: ex.method.to_owned(),
            payment_ref: if cash {
                String::new()
            } else {
                format!("TRF-{:08}", next())
            },
            account: if cash {
                "Cash drawer 1".to_owned()
            } else {
                "Bank account 1".to_owned()
            },
            staff: "EMP-001".to_owned(),
            booking: "WALK-IN".to_owned(),
            notes: String::new(),
            value_date: "2026-10-06".to_owned(),
            paid_at: "2026-10-06T05:30:00.000Z".to_owned(),
            payment_confirmed: true,
            identity_confirmed: true,
            passport: Some(passport),
            slip,
            reference_rate: ex.reference.map(str::to_owned),
        };
        self.desk
            .workflow
            .save(&here, cmd, "owner", t)
            .await
            .unwrap_or_else(|e| panic!("save an exchange in {branch}: {e:?}"))
            .document_ref
    }
}

#[allow(clippy::too_many_arguments)]
fn make_exchange(
    org_id: Uuid,
    branch: &str,
    shift_id: &str,
    side: Side,
    qty: &str,
    rate: &str,
    ref_rate: Option<&str>,
    method: PaymentMethod,
    slip: Option<&str>,
    now: DateTime<Utc>,
) -> ExchangeTransaction {
    let id = TransactionId(Uuid::new_v4());
    let qty_dec = Decimal::from_str(qty).unwrap();
    let rate_dec = Decimal::from_str(rate).unwrap();
    let thb_dec = qty_dec * rate_dec;
    let thb_str = format!("{:.2}", thb_dec);

    ExchangeTransaction {
        id,
        organization_id: org_id,
        branch_id: BranchId::new(branch.to_owned()),
        mode: Mode::Live,
        status: TransactionStatus::Draft,
        document_ref: None,
        quote_id: None,
        booking_id: None,
        shift_id: Some(shift_id.to_owned()),
        submission_id: format!("sub-{}", id.0),
        payload_hash: format!("hash-{}", id.0),
        customer_id: "CU-1".to_owned(),
        customer_name: "Somchai".to_owned(),
        nationality: "THA".to_owned(),
        staff_id: "EMP-001".to_owned(),
        booking_no: "WALK-IN".to_owned(),
        notes: String::new(),
        value_date: now.date_naive(),
        recorded_at: now,
        recorded_by: "owner".to_owned(),
        completed_at: None,
        cancelled_at: None,
        cancel_reason: None,
        approval_request_id: None,
        passport_file_key: format!("PASSPORT-{branch}"),
        kiosk_request_id: None,
        staff_draft_id: None,
        count_confirmed: false,
        lines: vec![TransactionLine {
            id: Uuid::new_v4(),
            quote_line_id: None,
            shop_side: side,
            currency: CurrencyCode::parse("USD").unwrap(),
            quantity: ForeignAmount::from_decimal(qty_dec),
            unit_size: UnitSize::parse("1").unwrap(),
            rate_snapshot: Rate::parse(rate).unwrap(),
            reference_rate: ref_rate.map(|r| Rate::parse(r).unwrap()),
            thb_amount: Money::parse(&thb_str).unwrap(),
            fee_thb: Money::parse("0.00").unwrap(),
            settle_thb: Money::parse(&thb_str).unwrap(),
            cost_basis_thb: None,
            realized_margin_thb: None,
            line_no: 1,
        }],
        payment: TransactionPayment {
            id: Uuid::new_v4(),
            payment_no: 1,
            method,
            reference: if method.is_cash() {
                String::new()
            } else {
                "TRX-1".to_owned()
            },
            account: if method.is_cash() {
                "Cash drawer 1".to_owned()
            } else {
                "KBank 1".to_owned()
            },
            slip_file_key: slip.map(str::to_owned),
            amount_thb: Money::parse(&thb_str).unwrap(),
            bank_fee_thb: Money::parse("0.00").unwrap(),
            paid_at: now,
        },
        receipts: vec![],
    }
}

/// §9 Edge case 1 (cash flow):
/// A shift with the four seeded rows (COMPLETED, DRAFT, CONFIRMED, CANCELLED), all paid in cash:
/// cash in/out from the completed one only.
///
/// Setup:
/// - 1 COMPLETED SELL: 100 USD @ 40 -> 4000.00 THB cash received.
/// - 1 DRAFT SELL: 100 USD @ 40 -> 4000.00 THB cash received.
/// - 1 CONFIRMED BUY: 50 USD @ 30 -> 1500.00 THB cash paid.
/// - 1 CANCELLED BUY: 50 USD @ 30 -> 1500.00 THB cash paid.
///
/// Required:
/// - `stats.incoming` = "4000.00" (the COMPLETED SELL only; DRAFT is ignored)
/// - `stats.outgoing` = "0.00" (neither BUY is COMPLETED)
/// - `stats.remaining` = "104000.00" (100000 + 4000 - 0)
///
/// On base:
/// Base sums all wide rows with `method = 'Cash'` without filtering status:
/// - incoming reads 8000.00 (draft included)
/// - outgoing reads 3000.00 (confirmed & cancelled included)
#[actix_web::test]
async fn shift_with_four_seeded_cash_rows_cash_flow_counts_completed_only() {
    let room = Room::open().await;
    let shift_id = room.open_shift(MAIN).await;
    let t = room.tick();

    // 1. COMPLETED SELL (100 USD @ 40 = 4000 THB)
    let tx_completed = make_exchange(
        room.org_id,
        MAIN,
        &shift_id,
        Side::Sell,
        "100",
        "40",
        Some("36"),
        PaymentMethod::Cash,
        None,
        t,
    );
    let id_completed = tx_completed.id;
    room.insert_draft(&tx_completed).await;
    room.move_status(
        id_completed,
        TransactionStatus::Draft,
        TransactionStatus::Confirmed,
        None,
        t,
    )
    .await;
    room.complete_tx(id_completed, "RECEIPT-1", t).await;

    // 2. DRAFT SELL (100 USD @ 40 = 4000 THB)
    let tx_draft = make_exchange(
        room.org_id,
        MAIN,
        &shift_id,
        Side::Sell,
        "100",
        "40",
        Some("36"),
        PaymentMethod::Cash,
        None,
        t,
    );
    room.insert_draft(&tx_draft).await;

    // 3. CONFIRMED BUY (50 USD @ 30 = 1500 THB)
    let tx_confirmed = make_exchange(
        room.org_id,
        MAIN,
        &shift_id,
        Side::Buy,
        "50",
        "30",
        Some("32"),
        PaymentMethod::Cash,
        None,
        t,
    );
    let id_confirmed = tx_confirmed.id;
    room.insert_draft(&tx_confirmed).await;
    room.move_status(
        id_confirmed,
        TransactionStatus::Draft,
        TransactionStatus::Confirmed,
        None,
        t,
    )
    .await;

    // 4. CANCELLED BUY (50 USD @ 30 = 1500 THB)
    let tx_cancelled = make_exchange(
        room.org_id,
        MAIN,
        &shift_id,
        Side::Buy,
        "50",
        "30",
        Some("32"),
        PaymentMethod::Cash,
        None,
        t,
    );
    let id_cancelled = tx_cancelled.id;
    room.insert_draft(&tx_cancelled).await;
    room.move_status(
        id_cancelled,
        TransactionStatus::Draft,
        TransactionStatus::Cancelled,
        Some("Customer left"),
        t,
    )
    .await;

    let data = room.cash("branch=MAIN").await;
    assert_eq!(
        data["stats"]["incoming"],
        json!("4000.00"),
        "incoming counts COMPLETED SELL only: {data}"
    );
    assert_eq!(
        data["stats"]["outgoing"],
        json!("0.00"),
        "outgoing counts COMPLETED BUY only: {data}"
    );
    assert_eq!(
        data["stats"]["remaining"],
        json!("104000.00"),
        "remaining reflects only COMPLETED cash flows: {data}"
    );
}

/// §9 Edge case 1 (reference spread):
/// The reference spread is over COMPLETED lines (shop_side, thb_amount, quantity, reference_rate, unit_size).
/// Unfinished and cancelled exchanges must not be counted in with_reference or the spread calculation.
///
/// Required:
/// - `reference.with_reference` = 1 (the single COMPLETED exchange)
/// - `reference.without_reference` = 0
/// - `reference.spread` = "400.00" (SELL: 4000 actual - 3600 reference)
/// - `reference.percent` = "11.11" (400 / 3600 * 100 = 11.11%)
///
/// On base:
/// Base selects all transactions in shift without status filter:
/// - with_reference = 4
/// - spread includes all 4 exchanges
#[actix_web::test]
async fn shift_with_four_seeded_cash_rows_reference_spread_counts_completed_only() {
    let room = Room::open().await;
    let shift_id = room.open_shift(MAIN).await;
    let t = room.tick();

    // 1. COMPLETED SELL (100 USD @ 40, ref 36: ref 3600, actual 4000, spread +400)
    let tx_completed = make_exchange(
        room.org_id,
        MAIN,
        &shift_id,
        Side::Sell,
        "100",
        "40",
        Some("36"),
        PaymentMethod::Cash,
        None,
        t,
    );
    let id_completed = tx_completed.id;
    room.insert_draft(&tx_completed).await;
    room.move_status(
        id_completed,
        TransactionStatus::Draft,
        TransactionStatus::Confirmed,
        None,
        t,
    )
    .await;
    room.complete_tx(id_completed, "RECEIPT-1", t).await;

    // 2. DRAFT SELL
    let tx_draft = make_exchange(
        room.org_id,
        MAIN,
        &shift_id,
        Side::Sell,
        "100",
        "40",
        Some("36"),
        PaymentMethod::Cash,
        None,
        t,
    );
    room.insert_draft(&tx_draft).await;

    // 3. CONFIRMED BUY
    let tx_confirmed = make_exchange(
        room.org_id,
        MAIN,
        &shift_id,
        Side::Buy,
        "50",
        "30",
        Some("32"),
        PaymentMethod::Cash,
        None,
        t,
    );
    let id_confirmed = tx_confirmed.id;
    room.insert_draft(&tx_confirmed).await;
    room.move_status(
        id_confirmed,
        TransactionStatus::Draft,
        TransactionStatus::Confirmed,
        None,
        t,
    )
    .await;

    // 4. CANCELLED BUY
    let tx_cancelled = make_exchange(
        room.org_id,
        MAIN,
        &shift_id,
        Side::Buy,
        "50",
        "30",
        Some("32"),
        PaymentMethod::Cash,
        None,
        t,
    );
    let id_cancelled = tx_cancelled.id;
    room.insert_draft(&tx_cancelled).await;
    room.move_status(
        id_cancelled,
        TransactionStatus::Draft,
        TransactionStatus::Cancelled,
        Some("Customer left"),
        t,
    )
    .await;

    let data = room.cash("branch=MAIN").await;
    assert_eq!(
        data["reference"]["withReference"],
        json!(1),
        "only COMPLETED exchange is counted in withReference: {data}"
    );
    assert_eq!(
        data["reference"]["withoutReference"],
        json!(0),
        "no COMPLETED exchange without reference: {data}"
    );
    assert_eq!(
        data["reference"]["spread"],
        json!("400.00"),
        "spread calculated over COMPLETED line only: {data}"
    );
    assert_eq!(
        data["reference"]["percent"],
        json!("11.11"),
        "percent calculated over COMPLETED line only: {data}"
    );
}

/// §9 Edge case 2:
/// A completed non-cash exchange is not in the cash sums.
///
/// Setup:
/// - 1 COMPLETED SELL via BankTransfer (100 USD @ 40 = 4000.00 THB) with ref rate 36.
/// - 1 COMPLETED BUY via BankTransfer (50 USD @ 30 = 1500.00 THB) with ref rate 32.
///
/// Required:
/// - Neither non-cash exchange enters incoming or outgoing cash sums.
/// - `stats.incoming` = "0.00"
/// - `stats.outgoing` = "0.00"
/// - `stats.remaining` = "100000.00"
/// - Both completed exchanges ARE included in the reference summary:
///   `with_reference` = 2, `spread` = "500.00" (400 + 100), `percent` = "9.62" (500 / 5200 * 100)
#[actix_web::test]
async fn completed_non_cash_exchange_is_excluded_from_cash_sums() {
    let room = Room::open().await;
    let shift_id = room.open_shift(MAIN).await;
    let t = room.tick();

    // 1. COMPLETED SELL via BankTransfer (4000 THB, ref 36: ref 3600, actual 4000, spread +400)
    let tx_sell = make_exchange(
        room.org_id,
        MAIN,
        &shift_id,
        Side::Sell,
        "100",
        "40",
        Some("36"),
        PaymentMethod::BankTransfer,
        Some("SLIP-1"),
        t,
    );
    let id_sell = tx_sell.id;
    room.insert_draft(&tx_sell).await;
    room.move_status(
        id_sell,
        TransactionStatus::Draft,
        TransactionStatus::Confirmed,
        None,
        t,
    )
    .await;
    room.complete_tx(id_sell, "RECEIPT-1", t).await;

    // 2. COMPLETED BUY via BankTransfer (1500 THB, ref 32: ref 1600, actual 1500, spread +100)
    let tx_buy = make_exchange(
        room.org_id,
        MAIN,
        &shift_id,
        Side::Buy,
        "50",
        "30",
        Some("32"),
        PaymentMethod::BankTransfer,
        Some("SLIP-1"),
        t,
    );
    let id_buy = tx_buy.id;
    room.insert_draft(&tx_buy).await;
    room.move_status(
        id_buy,
        TransactionStatus::Draft,
        TransactionStatus::Confirmed,
        None,
        t,
    )
    .await;
    room.complete_tx(id_buy, "RECEIPT-2", t).await;

    let data = room.cash("branch=MAIN").await;
    assert_eq!(
        data["stats"]["incoming"],
        json!("0.00"),
        "non-cash completed SELL is not in cash incoming: {data}"
    );
    assert_eq!(
        data["stats"]["outgoing"],
        json!("0.00"),
        "non-cash completed BUY is not in cash outgoing: {data}"
    );
    assert_eq!(
        data["stats"]["remaining"],
        json!("100000.00"),
        "cash remaining unchanged by non-cash exchanges: {data}"
    );
    assert_eq!(
        data["reference"]["withReference"],
        json!(2),
        "completed non-cash exchanges are included in reference summary: {data}"
    );
    assert_eq!(
        data["reference"]["spread"],
        json!("500.00"),
        "spread includes completed non-cash exchanges: {data}"
    );
    assert_eq!(
        data["reference"]["percent"],
        json!("9.62"),
        "percent includes completed non-cash exchanges: {data}"
    );
}

/// §9 Edge case 3:
/// The compat-saved exchange gives today's figures.
///
/// An exchange saved through `SaveTransactionWorkflow` (compat save) writes both wide
/// and child rows, and admin_cash must give the expected figures for stats, reference, and stock.
#[actix_web::test]
async fn compat_saved_exchange_gives_todays_figures() {
    let room = Room::open().await;
    room.open_shift(MAIN).await;

    // SELL 100 USD @ 40 (counter rate) = 4000.00 THB cash, reference rate 36.
    room.save(MAIN, Ex::sell("100", "0").reference("36")).await;

    let data = room.cash("branch=MAIN").await;

    // stats
    assert_eq!(
        data["stats"],
        json!({
            "opening": "100000.00",
            "incoming": "4000.00",
            "outgoing": "0.00",
            "remaining": "104000.00"
        }),
        "compat save cashFlow matches today's figures: {data}"
    );

    // reference
    assert_eq!(
        data["reference"],
        json!({
            "withReference": 1,
            "withoutReference": 0,
            "spread": "400.00",
            "percent": "11.11"
        }),
        "compat save referenceSummary matches today's figures: {data}"
    );

    // stock
    let stock = data["stock"].as_array().expect("stock list");
    let thb = stock
        .iter()
        .find(|r| r["code"] == "THB")
        .expect("THB stock row");
    assert_eq!(thb["opening"], json!("100000.00"));
    assert_eq!(thb["net"], json!("4000.00"));
    assert_eq!(thb["expected"], json!("104000.00"));

    let usd = stock
        .iter()
        .find(|r| r["code"] == "USD")
        .expect("USD stock row");
    assert_eq!(usd["opening"], json!("1000.00"));
    assert_eq!(usd["net"], json!("-100.00"));
    assert_eq!(usd["expected"], json!("900.00"));
}

/// Child Table Reads (§3 & §8, Done Criterion 3):
/// The view must read cash flow and reference spread from `transaction_lines` (settle_thb, thb_amount,
/// quantity, reference_rate, unit_size, shop_side) and `transaction_payments` (method), never from the
/// wide `transactions` columns.
///
/// Setup:
/// - 1 COMPLETED cash SELL: 100 USD @ 40 = 4000.00 THB, ref rate 36.
/// - 1 COMPLETED cash BUY: 50 USD @ 30 = 1500.00 THB, no ref rate.
/// - 1 COMPLETED non-cash SELL (mirror case): 50 USD @ 40 = 2000.00 THB via BankTransfer.
///
/// Wide column desync:
/// - For cash exchanges: wide `method` -> 'SCB Transfer', `received` -> 0, `paid` -> 0, `principal` -> 0,
///   `amount` -> 1, `unit_size` -> 1000, `type` flipped, `reference_rate` -> NULL.
/// - For non-cash exchange: wide `method` -> 'Cash', `received` -> 9999.00.
///
/// Required:
/// - `stats.incoming` = "4000.00" (from line settle_thb where payment method = 'Cash')
/// - `stats.outgoing` = "1500.00" (from line settle_thb where payment method = 'Cash')
/// - `stats.remaining` = "102500.00" (100000 + 4000 - 1500)
/// - `reference.withReference` = 1 (from line reference_rate)
/// - `reference.withoutReference` = 2
/// - `reference.spread` = "400.00"
/// - `reference.percent` = "11.11"
#[actix_web::test]
async fn cash_view_reads_figures_from_lines_and_payments_not_wide_columns() {
    let room = Room::open().await;
    let shift_id = room.open_shift(MAIN).await;
    let t = room.tick();

    // 1. COMPLETED cash SELL: 100 USD @ 40 = 4000 THB, ref 36 (ref val 3600, spread +400)
    let tx_sell_cash = make_exchange(
        room.org_id,
        MAIN,
        &shift_id,
        Side::Sell,
        "100",
        "40",
        Some("36"),
        PaymentMethod::Cash,
        None,
        t,
    );
    let id_sell_cash = tx_sell_cash.id;
    room.insert_draft(&tx_sell_cash).await;
    room.move_status(
        id_sell_cash,
        TransactionStatus::Draft,
        TransactionStatus::Confirmed,
        None,
        t,
    )
    .await;
    room.complete_tx(id_sell_cash, "RECEIPT-1", t).await;

    // 2. COMPLETED cash BUY: 50 USD @ 30 = 1500 THB, no ref
    let tx_buy_cash = make_exchange(
        room.org_id,
        MAIN,
        &shift_id,
        Side::Buy,
        "50",
        "30",
        None,
        PaymentMethod::Cash,
        None,
        t,
    );
    let id_buy_cash = tx_buy_cash.id;
    room.insert_draft(&tx_buy_cash).await;
    room.move_status(
        id_buy_cash,
        TransactionStatus::Draft,
        TransactionStatus::Confirmed,
        None,
        t,
    )
    .await;
    room.complete_tx(id_buy_cash, "RECEIPT-2", t).await;

    // 3. COMPLETED non-cash SELL (mirror case): 50 USD @ 40 = 2000 THB via BankTransfer
    let tx_sell_transfer = make_exchange(
        room.org_id,
        MAIN,
        &shift_id,
        Side::Sell,
        "50",
        "40",
        None,
        PaymentMethod::BankTransfer,
        Some("SLIP-1"),
        t,
    );
    let id_sell_transfer = tx_sell_transfer.id;
    room.insert_draft(&tx_sell_transfer).await;
    room.move_status(
        id_sell_transfer,
        TransactionStatus::Draft,
        TransactionStatus::Confirmed,
        None,
        t,
    )
    .await;
    room.complete_tx(id_sell_transfer, "RECEIPT-3", t).await;

    // Desync the wide columns for the cash exchanges
    sqlx::query(
        "UPDATE transactions \
         SET method = 'SCB Transfer', \
             received = 0, \
             paid = 0, \
             principal = 0, \
             amount = 1, \
             type = CASE WHEN type = 'SELL' THEN 'BUY' ELSE 'SELL' END, \
             reference_rate = NULL, \
             unit_size = 1000 \
         WHERE id IN ($1, $2)",
    )
    .bind(id_sell_cash.0)
    .bind(id_buy_cash.0)
    .execute(&room.desk.db.pool)
    .await
    .expect("desync wide columns for cash exchanges");

    // Mirror case: desync the non-cash exchange with wide method = 'Cash' and wide received = 9999.00
    sqlx::query(
        "UPDATE transactions \
         SET method = 'Cash', \
             received = 9999.00 \
         WHERE id = $1",
    )
    .bind(id_sell_transfer.0)
    .execute(&room.desk.db.pool)
    .await
    .expect("desync wide columns for mirror non-cash exchange");

    let data = room.cash("branch=MAIN").await;

    assert_eq!(
        data["stats"]["incoming"],
        json!("4000.00"),
        "stats.incoming must be read from transaction_lines.settle_thb with transaction_payments.method = 'Cash', not wide received/method: {data}"
    );
    assert_eq!(
        data["stats"]["outgoing"],
        json!("1500.00"),
        "stats.outgoing must be read from transaction_lines.settle_thb with transaction_payments.method = 'Cash', not wide paid/method: {data}"
    );
    assert_eq!(
        data["stats"]["remaining"],
        json!("102500.00"),
        "stats.remaining reflects child-table cash flows: {data}"
    );
    assert_eq!(
        data["reference"]["withReference"],
        json!(1),
        "reference.withReference must be read from transaction_lines.reference_rate: {data}"
    );
    assert_eq!(
        data["reference"]["withoutReference"],
        json!(2),
        "reference.withoutReference must count completed lines without reference: {data}"
    );
    assert_eq!(
        data["reference"]["spread"],
        json!("400.00"),
        "reference.spread must be computed from transaction_lines figures: {data}"
    );
    assert_eq!(
        data["reference"]["percent"],
        json!("11.11"),
        "reference.percent must be computed from transaction_lines figures: {data}"
    );
}
