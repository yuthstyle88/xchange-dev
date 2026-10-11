//! Ticket #1135 (TX-8k): `GET /devices/receipt` — the receipt page reads figures from
//! `transaction_lines` (shop_side, currency, quantity, rate_snapshot, fee_thb, unit_size),
//! not from the wide row on `transactions`.
//!
//! Parent ticket #676 (TX-8): readers must never read from the wide row, and must filter
//! `status = 'COMPLETED'`.
//!
//! Written before the implementation by the test-writer (gemini/studio-11, rule A).
//! Do not edit, delete or weaken.

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
mod rpc_common;
mod snapshot_common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use rust_decimal::Decimal;
use serde_json::json;
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
use xchange::bootstrap::AppState;

const ORIGIN: &str = "http://desk.test";
const RECEIPT: &str = "/devices/receipt";
const MAIN: &str = "MAIN";
const MAIN_NAME: &str = "XC-B";

const NOT_FOUND: &str = "ไม่พบรายการ";

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

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    t0: DateTime<Utc>,
}

struct Saved {
    reference: String,
    customer: String,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pages/web");
        let state = AppState::new(
            Arc::new(common::settings(&[(
                "APP_WEB__DIR",
                web.to_str().expect("a UTF-8 path"),
            )])),
            desk.db.pool.clone(),
        );
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
        let t0 = snapshot_common::time("2026-10-06T05:00:00.000Z");
        let room = Room {
            desk,
            app,
            cookies,
            t0,
        };
        rates_re4e_publish::set_counter_rate(&room.desk.db.pool, MAIN, "USD", "32.1", "33").await;
        room.open_shift(MAIN).await;
        room
    }

    fn tick(&self) -> DateTime<Utc> {
        let t = self.t0 + Duration::seconds(next() as i64);
        self.desk.clock.set(t);
        t
    }

    async fn open_shift(&self, branch: &str) {
        self.tick();
        let answer = self
            .app
            .send(
                Call::action(
                    ORIGIN,
                    "/api/rpc",
                    "adminOpenShift",
                    json!({ "branchId": branch, "requestId": request_id(),
                            "opening": { "THB": 100000, "USD": 1000 } }),
                )
                .cookie(&self.cookies["OWNER"]),
            )
            .await;
        assert_eq!(answer.status, 200, "open a shift: {:.300}", answer.text);
    }

    async fn save(&self, branch: &str, side: &str, amount: &str, fee: &str) -> Saved {
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
        let cmd = SaveTransaction {
            customer: person,
            submission_id: format!("submission-{branch}-{:012}", next()),
            customer_id: customer.clone(),
            side: side.to_owned(),
            currency: "USD".to_owned(),
            amount: amount.to_owned(),
            rate: if side == "BUY" { "32.1" } else { "33" }.to_owned(),
            fee: fee.to_owned(),
            bank_fee: "0".to_owned(),
            method: "Cash".to_owned(),
            payment_ref: String::new(),
            account: "Cash drawer 1".to_owned(),
            staff: "EMP-001".to_owned(),
            booking: "WALK-IN".to_owned(),
            notes: String::new(),
            value_date: "2026-10-06".to_owned(),
            paid_at: "2026-10-06T05:30:00.000Z".to_owned(),
            payment_confirmed: true,
            identity_confirmed: true,
            passport: Some(passport),
            slip: None,
            reference_rate: None,
        };
        let reference = self
            .desk
            .workflow
            .save(&here, cmd, "owner", t)
            .await
            .unwrap_or_else(|e| panic!("save a {side} in {branch}: {e:?}"))
            .document_ref;
        Saved {
            reference,
            customer,
        }
    }

    async fn get(&self, who: &str, path: &str) -> Answer {
        let mut call = Call::post(ORIGIN, path, Vec::<u8>::new())
            .without("x-xchange-request")
            .without("content-type")
            .without("origin");
        call.method = Method::GET;
        let call = if who.is_empty() {
            call
        } else {
            call.cookie(&self.cookies[who])
        };
        self.app.send(call).await
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn receipt(query: &str) -> String {
    format!("{RECEIPT}?{query}")
}

// ── Edge cases (§9) ─────────────────────────────────────────────────────────

/// §9 edge case 1: a completed compat-saved exchange renders the receipt HTML
/// exactly matching today's snapshot on main.
#[actix_web::test]
async fn compat_saved_completed_exchange_matches_html_snapshot() {
    let room = Room::open().await;
    let buy = room.save(MAIN, "BUY", "100", "0").await;

    let path = receipt(&format!("branch=MAIN&ref={}", buy.reference));
    let answer = room.get("OWNER", &path).await;

    assert_eq!(answer.status, 200, "status 200 for completed exchange");
    assert_eq!(
        answer.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(answer.header("cache-control"), Some("no-store"));

    let expected_header = format!(
        "<!doctype html><html lang=\"th\"><meta charset=\"utf-8\"><title>ตัวอย่างสลิป {}</title>",
        buy.reference
    );
    assert!(
        answer.text.starts_with(&expected_header),
        "starts with HTML header and title"
    );
    let expected_lines = format!(
        "ลูกค้า {}<br>BUY 100 USD<br>เรท 32.1<br>ค่าธรรมเนียม 0 THB<br>จ่าย 3,210 THB",
        buy.customer
    );
    assert!(
        answer.text.contains(&expected_lines),
        "contains exchange lines"
    );
    assert!(
        answer.text.contains(&format!(
            "สาขา {MAIN_NAME}<br>เลขรายการ {}<br>",
            buy.reference
        )),
        "contains branch and reference"
    );

    room.finish().await;
}

/// §9 edge case 2: an unknown document ref returns 404 "ไม่พบรายการ".
#[actix_web::test]
async fn unknown_document_ref_returns_404_not_found() {
    let room = Room::open().await;

    let path = receipt("branch=MAIN&ref=XC-20261006-999999");
    let answer = room.get("OWNER", &path).await;

    assert_eq!(answer.status, 404);
    assert_eq!(answer.text, NOT_FOUND);

    room.finish().await;
}

/// Parent TX-8: each child seeds one COMPLETED, one DRAFT, one CONFIRMED, and one CANCELLED
/// exchange through TX-2's store. DRAFT, CONFIRMED, CANCELLED rows have no document_ref
/// and accessing receipt for them returns 404 "ไม่พบรายการ".
#[actix_web::test]
async fn draft_confirmed_and_cancelled_exchanges_return_404_not_found() {
    let room = Room::open().await;

    let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
        .fetch_one(&room.desk.db.pool)
        .await
        .expect("query org id");

    let shift_id: String =
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = 'MAIN' AND closed_at IS NULL")
            .fetch_one(&room.desk.db.pool)
            .await
            .expect("shift id");

    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ('CU-DRAFT', $1, 'CU-DRAFT', 'Somchai', 'LIVE', now())",
    )
    .bind(org_id)
    .execute(&room.desk.db.pool)
    .await
    .expect("insert customer");

    sqlx::query(
        "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
         VALUES ('PASSPORT-DRAFT', 'MAIN', 'owner', 'image/jpeg', 'sha-draft', now())",
    )
    .execute(&room.desk.db.pool)
    .await
    .expect("insert passport file");

    let t = room.tick();
    let draft_id = TransactionId(Uuid::new_v4());
    let draft_tx = ExchangeTransaction {
        id: draft_id,
        organization_id: org_id,
        branch_id: BranchId::new(MAIN),
        mode: Mode::Live,
        status: TransactionStatus::Draft,
        document_ref: None,
        quote_id: None,
        booking_id: None,
        shift_id: Some(shift_id),
        submission_id: format!("sub-draft-{}", draft_id.0),
        payload_hash: "hash".to_owned(),
        customer_id: "CU-DRAFT".to_owned(),
        customer_name: "Somchai".to_owned(),
        nationality: "THA".to_owned(),
        staff_id: "EMP-001".to_owned(),
        booking_no: "WALK-IN".to_owned(),
        notes: String::new(),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
        recorded_at: t,
        recorded_by: "owner".to_owned(),
        completed_at: None,
        cancelled_at: None,
        cancel_reason: None,
        approval_request_id: None,
        passport_file_key: "PASSPORT-DRAFT".to_owned(),
        kiosk_request_id: None,
        staff_draft_id: None,
        count_confirmed: false,
        lines: vec![TransactionLine {
            id: Uuid::new_v4(),
            quote_line_id: None,
            shop_side: Side::Buy,
            currency: CurrencyCode::parse("USD").unwrap(),
            quantity: ForeignAmount::from_decimal(Decimal::from(100)),
            unit_size: UnitSize::parse("1").unwrap(),
            rate_snapshot: Rate::parse("32.1").unwrap(),
            reference_rate: None,
            thb_amount: Money::parse("3210.00").unwrap(),
            fee_thb: Money::parse("0.00").unwrap(),
            settle_thb: Money::parse("3210.00").unwrap(),
            cost_basis_thb: None,
            realized_margin_thb: None,
            line_no: 1,
        }],
        payment: TransactionPayment {
            id: Uuid::new_v4(),
            payment_no: 1,
            method: PaymentMethod::Cash,
            reference: String::new(),
            account: "Cash drawer 1".to_owned(),
            slip_file_key: None,
            amount_thb: Money::parse("3210.00").unwrap(),
            bank_fee_thb: Money::parse("0.00").unwrap(),
            paid_at: t,
        },
        receipts: vec![],
    };

    let store = PgTransactionStore::new(room.desk.db.pool.clone());
    let mut db_tx = room.desk.db.pool.begin().await.expect("begin tx");
    store
        .insert_draft_in(&mut db_tx, &draft_tx)
        .await
        .expect("insert draft");
    db_tx.commit().await.expect("commit draft");

    // DRAFT has no document_ref; looking up by its ID returns 404
    let answer = room
        .get(
            "OWNER",
            &receipt(&format!("branch=MAIN&ref={}", draft_id.0)),
        )
        .await;
    assert_eq!(answer.status, 404, "draft returns 404");
    assert_eq!(answer.text, NOT_FOUND);

    // CONFIRMED has no document_ref; looking up returns 404
    let mut db_tx = room.desk.db.pool.begin().await.expect("begin tx");
    store
        .set_status_in(
            &mut db_tx,
            draft_id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Confirmed,
                reason: None,
                approval_request_id: None,
            },
            t,
        )
        .await
        .expect("move to Confirmed");
    db_tx.commit().await.expect("commit confirmed");

    let answer = room
        .get(
            "OWNER",
            &receipt(&format!("branch=MAIN&ref={}", draft_id.0)),
        )
        .await;
    assert_eq!(answer.status, 404, "confirmed returns 404");
    assert_eq!(answer.text, NOT_FOUND);

    // CANCELLED has no document_ref; looking up returns 404
    let mut db_tx = room.desk.db.pool.begin().await.expect("begin tx");
    store
        .set_status_in(
            &mut db_tx,
            draft_id,
            StatusMove {
                from: TransactionStatus::Confirmed,
                to: TransactionStatus::Cancelled,
                reason: Some("customer cancelled".to_owned()),
                approval_request_id: None,
            },
            t,
        )
        .await
        .expect("move to Cancelled");
    db_tx.commit().await.expect("commit cancelled");

    let answer = room
        .get(
            "OWNER",
            &receipt(&format!("branch=MAIN&ref={}", draft_id.0)),
        )
        .await;
    assert_eq!(answer.status, 404, "cancelled returns 404");
    assert_eq!(answer.text, NOT_FOUND);

    room.finish().await;
}

// ── Failing up-front tests (Ticket #1135 / Parent #676) ─────────────────────

/// Ticket #1135 §1 / §3: The customer receipt page reads type, code, amount, rate, fee
/// from `transaction_lines`, NOT from the wide row on `transactions`.
///
/// When the wide row is altered to different figures, the receipt page must still
/// display the figures from `transaction_lines`.
#[actix_web::test]
async fn completed_exchange_reads_figures_from_transaction_lines_when_wide_row_differs() {
    let room = Room::open().await;
    let saved = room.save(MAIN, "BUY", "100", "0").await;

    // Mutate the wide row in `transactions` to have completely different dummy figures.
    // Child table `transaction_lines` remains unchanged:
    // shop_side = 'BUY', currency = 'USD', quantity = 100, rate_snapshot = 32.1, fee_thb = 0.
    sqlx::query(
        "UPDATE transactions \
         SET type = 'SELL', code = 'EUR', amount = 999.0000, rate = 50.00000000, fee = 99.00 \
         WHERE document_ref = $1",
    )
    .bind(&saved.reference)
    .execute(&room.desk.db.pool)
    .await
    .expect("update wide columns on transactions");

    let path = receipt(&format!("branch=MAIN&ref={}", saved.reference));
    let answer = room.get("OWNER", &path).await;

    assert_eq!(answer.status, 200, "status is 200 OK");

    // Must read line figures from `transaction_lines`:
    assert!(
        answer.text.contains("BUY 100 USD"),
        "receipt body should contain line figures (\"BUY 100 USD\"), but got:\n{}",
        answer.text
    );
    assert!(
        answer.text.contains("เรท 32.1"),
        "receipt body should contain line rate (\"เรท 32.1\"), but got:\n{}",
        answer.text
    );
    assert!(
        answer.text.contains("ค่าธรรมเนียม 0 THB"),
        "receipt body should contain line fee (\"ค่าธรรมเนียม 0 THB\"), but got:\n{}",
        answer.text
    );
    assert!(
        answer.text.contains("จ่าย 3,210 THB"),
        "receipt body should contain line total (\"จ่าย 3,210 THB\"), but got:\n{}",
        answer.text
    );

    // Must NOT read from the wide row:
    assert!(
        !answer.text.contains("SELL 999 EUR"),
        "receipt body must NOT read wide row figures (\"SELL 999 EUR\")"
    );
    assert!(
        !answer.text.contains("เรท 50"),
        "receipt body must NOT read wide row rate (\"เรท 50\")"
    );

    room.finish().await;
}

/// Ticket #1135 §1 / §3: A SELL transaction with fee reads `shop_side`, `currency`, `quantity`,
/// `rate_snapshot`, and `fee_thb` from `transaction_lines`, and computes THB as base + fee.
#[actix_web::test]
async fn completed_exchange_reads_sell_with_fee_from_transaction_lines() {
    let room = Room::open().await;
    let saved = room.save(MAIN, "SELL", "50", "20").await;

    // Mutate the wide row in `transactions` to have different figures:
    // wide row says BUY 10 GBP rate 40 fee 0.
    // child table `transaction_lines` has SELL 50 USD rate 33 fee 20 -> รับ 1,670 THB.
    sqlx::query(
        "UPDATE transactions \
         SET type = 'BUY', code = 'GBP', amount = 10.0000, rate = 40.00000000, fee = 0.00 \
         WHERE document_ref = $1",
    )
    .bind(&saved.reference)
    .execute(&room.desk.db.pool)
    .await
    .expect("update wide columns on transactions");

    let path = receipt(&format!("ref={}", saved.reference));
    let answer = room.get("FRONT", &path).await;

    assert_eq!(answer.status, 200, "status is 200 OK");

    assert!(
        answer.text.contains("SELL 50 USD"),
        "receipt body should contain line figures (\"SELL 50 USD\"), but got:\n{}",
        answer.text
    );
    assert!(
        answer.text.contains("เรท 33"),
        "receipt body should contain line rate (\"เรท 33\"), but got:\n{}",
        answer.text
    );
    assert!(
        answer.text.contains("ค่าธรรมเนียม 20 THB"),
        "receipt body should contain line fee (\"ค่าธรรมเนียม 20 THB\"), but got:\n{}",
        answer.text
    );
    assert!(
        answer.text.contains("รับ 1,670 THB"),
        "receipt body should contain line total (\"รับ 1,670 THB\"), but got:\n{}",
        answer.text
    );

    // Must NOT read wide row:
    assert!(
        !answer.text.contains("BUY 10 GBP"),
        "receipt body must NOT read wide row figures (\"BUY 10 GBP\")"
    );

    room.finish().await;
}

/// Ticket #1135 §1 / §3: The receipt page reads `unit_size` from `transaction_lines`.
/// For JPY with unit_size 100, 10,000 JPY at rate 0.25 yields (10,000 / 100) * 0.25 = 25 THB.
#[actix_web::test]
async fn completed_exchange_reads_unit_size_from_transaction_lines() {
    let room = Room::open().await;
    let saved = room.save(MAIN, "BUY", "100", "0").await;

    // Update child line to JPY with unit_size 100:
    sqlx::query(
        "UPDATE transaction_lines \
         SET currency = 'JPY', quantity = 10000.0000, rate_snapshot = 0.25000000, unit_size = 100.0000, \
             fee_thb = 0.00, thb_amount = 25.00, settle_thb = 25.00 \
         WHERE transaction_id = (SELECT id FROM transactions WHERE document_ref = $1)",
    )
    .bind(&saved.reference)
    .execute(&room.desk.db.pool)
    .await
    .expect("update transaction_lines to JPY with unit_size 100");

    // Update wide row to have unit_size = 1 and amount = 100:
    // If wide row is read, it displays "BUY 100 JPY", and unit_size=1 would compute 25 THB for 100 units.
    // If lines are read, it displays "BUY 10,000 JPY" and computes 25 THB with unit_size 100.
    sqlx::query(
        "UPDATE transactions \
         SET code = 'JPY', amount = 100.0000, rate = 0.25000000, unit_size = 1.0000, fee = 0.00 \
         WHERE document_ref = $1",
    )
    .bind(&saved.reference)
    .execute(&room.desk.db.pool)
    .await
    .expect("update wide columns on transactions");

    let path = receipt(&format!("branch=MAIN&ref={}", saved.reference));
    let answer = room.get("OWNER", &path).await;

    assert_eq!(answer.status, 200, "status is 200 OK");

    assert!(
        answer.text.contains("BUY 10,000 JPY"),
        "receipt body should contain line quantity (\"BUY 10,000 JPY\"), but got:\n{}",
        answer.text
    );
    assert!(
        answer.text.contains("เรท 0.25"),
        "receipt body should contain rate 0.25, but got:\n{}",
        answer.text
    );
    assert!(
        answer.text.contains("จ่าย 25 THB"),
        "receipt body should contain total 25 THB, but got:\n{}",
        answer.text
    );

    assert!(
        !answer.text.contains("BUY 100 JPY"),
        "receipt body must NOT read wide row amount (\"BUY 100 JPY\")"
    );

    room.finish().await;
}

/// Ticket #1135 §3 / §4 item 3: The SQL query in `pages.rs` must query `transaction_lines`,
/// filter `status = 'COMPLETED'`, and contain no reads of the wide row.
#[test]
fn pages_receipt_query_has_no_wide_row_reads_and_filters_completed_status() {
    let source = include_str!("../src/presentation/pages.rs");

    assert!(
        source.contains("transaction_lines"),
        "pages.rs must query transaction_lines for receipt figures"
    );
    assert!(
        source.contains("status = 'COMPLETED'") || source.contains("status = \"COMPLETED\""),
        "pages.rs receipt query must explicitly require status = 'COMPLETED'"
    );
    assert!(
        !source.contains(
            "SELECT customer_id, type, code, amount::text, rate::text, fee::text, unit_size::text \
         FROM transactions"
        ),
        "pages.rs must not read wide row columns from transactions"
    );
}
