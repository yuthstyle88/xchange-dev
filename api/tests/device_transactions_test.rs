//! Ticket #1133 (TX-8h) — `device_transactions` view shows completed exchanges only,
//! and reads type, amount and code from the line (shop_side, quantity, currency),
//! ignoring unfinished transactions and not reading wide transaction columns.
//!
//! Up-front tests written by the test-writer (gemini/warut-6) under Rule A.
//! Based on the TX-2 branch (PR 1118, commit a6ceae2).
//!
//! Edge cases from #1133 §9:
//! 1. The four seeded rows (COMPLETED, DRAFT, CONFIRMED, CANCELLED): the completed one only.
//! 2. Paging with `before` over completed rows only.
//! 3. The compat-saved exchange's row equals today's.
//! 4. Figures (type, amount, currency) are read from the line, not wide columns.

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
#[macro_use]
mod rpc_common;
mod shift_holder_common;
mod shift_setup_common;
mod snapshot_common;

use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, TimeZone, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
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
use xc_kernel::mode::Mode;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};

const ORIGIN: &str = "http://desk.test";
const MAIN: &str = "MAIN";
const TRANSACTION_KEYS: [&str; 4] = ["id", "type", "amount", "currency"];

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

fn path(query: &str) -> String {
    if query.is_empty() {
        "/api/v3/views/device_transactions".to_string()
    } else {
        format!("/api/v3/views/device_transactions?{query}")
    }
}

fn get(path: &str) -> Call {
    let mut call = Call::post(ORIGIN, path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

fn decimal(text: &Value, what: &str) -> Decimal {
    let s = text
        .as_str()
        .unwrap_or_else(|| panic!("{what}: money is a decimal string (W7), got {text}"));
    Decimal::from_str(s).unwrap_or_else(|e| panic!("{what}: {s:?} is a decimal: {e}"))
}

fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("an object: {value}"))
        .keys()
        .cloned()
        .collect()
}

fn set_of(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| (*s).to_string()).collect()
}

#[derive(Clone)]
struct Who {
    name: String,
    passport: String,
    nationality: String,
}

impl Who {
    fn new(name: &str) -> Who {
        Who {
            name: name.to_owned(),
            passport: format!("AA{:07}", next()),
            nationality: "THA".to_owned(),
        }
    }

    fn command(&self) -> ReserveCustomer {
        ReserveCustomer {
            name: self.name.clone(),
            passport_no: self.passport.clone(),
            nationality: self.nationality.clone(),
            birth_date: String::new(),
            passport_expiry: String::new(),
            mode: "LIVE".to_owned(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn make_tx(
    org_id: Uuid,
    branch_id: &str,
    shift_id: Option<String>,
    customer_id: &str,
    customer_name: &str,
    side: Side,
    currency: &str,
    quantity: &str,
    rate: &str,
    recorded_at: DateTime<Utc>,
) -> ExchangeTransaction {
    let id = TransactionId(Uuid::new_v4());
    let qty_dec = Decimal::from_str(quantity).expect("valid quantity decimal");
    let rate_dec = Decimal::from_str(rate).expect("valid rate decimal");
    let thb_dec = (qty_dec * rate_dec).round_dp(2);
    let amount = Money::parse(&thb_dec.to_string()).expect("valid thb amount");
    let qty = ForeignAmount::from_decimal(qty_dec);
    let r = Rate::parse(rate).expect("valid rate");

    ExchangeTransaction {
        id,
        organization_id: org_id,
        branch_id: BranchId::new(branch_id.to_owned()),
        mode: Mode::Live,
        status: TransactionStatus::Draft,
        document_ref: None,
        quote_id: None,
        booking_id: None,
        shift_id,
        submission_id: format!("sub-{}", id.0),
        payload_hash: "hash".to_owned(),
        customer_id: customer_id.to_owned(),
        customer_name: customer_name.to_owned(),
        nationality: "THA".to_owned(),
        staff_id: "owner".to_owned(),
        booking_no: "WALK-IN".to_owned(),
        notes: "test".to_owned(),
        value_date: recorded_at.date_naive(),
        recorded_at,
        recorded_by: "owner".to_owned(),
        completed_at: None,
        cancelled_at: None,
        cancel_reason: None,
        approval_request_id: None,
        passport_file_key: "PASSPORT-MAIN".to_owned(),
        kiosk_request_id: None,
        staff_draft_id: None,
        count_confirmed: false,
        lines: vec![TransactionLine {
            id: Uuid::new_v4(),
            quote_line_id: None,
            shop_side: side,
            currency: CurrencyCode::parse(currency).expect("valid currency code"),
            quantity: qty,
            unit_size: UnitSize::parse("1").expect("valid unit size"),
            rate_snapshot: r,
            reference_rate: Some(r),
            thb_amount: amount,
            fee_thb: Money::parse("0.00").unwrap(),
            settle_thb: amount,
            cost_basis_thb: None,
            realized_margin_thb: None,
            line_no: 1,
        }],
        payment: TransactionPayment {
            id: Uuid::new_v4(),
            payment_no: 1,
            method: PaymentMethod::Cash,
            reference: "".to_owned(),
            account: "Cash drawer 1".to_owned(),
            slip_file_key: None,
            amount_thb: amount,
            bank_fee_thb: Money::parse("0.00").unwrap(),
            paid_at: recorded_at,
        },
        receipts: vec![],
    }
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    t0: DateTime<Utc>,
    store: PgTransactionStore,
    org_id: Uuid,
    shift_id: String,
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

        sqlx::query(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ('PASSPORT-MAIN', 'MAIN', 'owner', 'application/pdf', 'sha-p', now()), \
                    ('RECEIPT-MAIN', 'MAIN', 'owner', 'application/pdf', 'sha-r', now()) \
             ON CONFLICT (key) DO NOTHING",
        )
        .execute(&desk.db.pool)
        .await
        .expect("seed files");

        let store = PgTransactionStore::new(desk.db.pool.clone());
        let org_id: Uuid =
            sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
                .fetch_one(&desk.db.pool)
                .await
                .expect("fetch org_id");
        let room = Room {
            desk,
            app,
            cookies,
            t0,
            store,
            org_id,
            shift_id: String::new(),
        };
        room.set_rate(MAIN, "USD", "32.1", "33").await;
        room.open_shift(MAIN).await;
        let shift_id: String = sqlx::query_scalar(
            "SELECT id FROM shifts WHERE branch_id = $1 AND closed_at IS NULL ORDER BY seq DESC LIMIT 1",
        )
        .bind(MAIN)
        .fetch_one(&room.desk.db.pool)
        .await
        .expect("active shift id");
        Room { shift_id, ..room }
    }

    fn tick(&self) -> DateTime<Utc> {
        let t = self.t0 + Duration::seconds(next() as i64);
        self.desk.clock.set(t);
        t
    }

    fn cookie(&self, who: &str) -> String {
        self.cookies[who].clone()
    }

    async fn get(&self, who: &str, query: &str) -> Answer {
        let target = path(query);
        let call = get(&target);
        let call = if who.is_empty() {
            call
        } else {
            call.cookie(&self.cookie(who))
        };
        self.app.send(call).await
    }

    async fn view(&self, who: &str, query: &str) -> Value {
        let answer = self.get(who, query).await;
        assert_eq!(
            answer.status,
            200,
            "GET {} as {who:?}: {:.400}",
            path(query),
            answer.text
        );
        answer.json()
    }

    async fn rows(&self, who: &str, query: &str) -> Vec<Value> {
        let body = self.view(who, query).await;
        assert_eq!(body["changed"], json!(true), "GET {}: {body}", path(query));
        body.get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| panic!("GET {}: a list in data: {body}", path(query)))
    }

    async fn page(&self, query: &str) -> Vec<Value> {
        self.rows("OWNER", query).await
    }

    async fn set_rate(&self, branch: &str, code: &str, buy: &str, sell: &str) {
        rates_re4e_publish::set_counter_rate(&self.desk.db.pool, branch, code, buy, sell).await;
    }

    async fn open_shift(&self, branch: &str) {
        let t = self.tick();
        shift_setup_common::open_shift_by_count(
            &self.desk.db.pool,
            branch,
            json!({ "THB": 100000, "USD": 1000, "EUR": 1000 }),
            t,
        )
        .await;
    }

    async fn reserve(&self, branch: &'static str, who: &Who) -> String {
        let t = self.tick();
        CustomerService::new(self.desk.db.branch_tx())
            .reserve(&BranchId::new(branch), who.command(), t)
            .await
            .unwrap_or_else(|e| panic!("reserve {} in {branch}: {e:?}", who.name))
            .customer_id
    }

    async fn save_compat(
        &self,
        branch: &'static str,
        who: &Who,
        side: &str,
        currency: &str,
        amount: &str,
        rate: &str,
    ) -> String {
        let customer_id = self.reserve(branch, who).await;
        let t = self.tick();
        let here = BranchId::new(branch);
        let saver = shift_holder_common::teller_of_open_shift(&self.desk.db.pool, branch).await;
        let passport = self
            .desk
            .files
            .upload(&here, &saver, "image/jpeg", JPEG, t)
            .await
            .expect("upload a passport");
        let cmd = SaveTransaction {
            customer: who.command(),
            submission_id: format!("submission-{branch}-{:012}", next()),
            customer_id,
            side: side.to_owned(),
            currency: currency.to_owned(),
            amount: amount.to_owned(),
            rate: rate.to_owned(),
            fee: "0".to_owned(),
            bank_fee: "0".to_owned(),
            method: "Cash".to_owned(),
            payment_ref: String::new(),
            account: "Cash drawer 1".to_owned(),
            staff: "owner".to_owned(),
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
        let saved = self
            .desk
            .workflow
            .save(&here, cmd, &saver, t)
            .await
            .unwrap_or_else(|e| panic!("save an exchange of {} in {branch}: {e:?}", who.name));
        saved.document_ref
    }

    #[allow(clippy::too_many_arguments)]
    fn make_tx(
        &self,
        customer_id: &str,
        customer_name: &str,
        side: Side,
        currency: &str,
        quantity: &str,
        rate: &str,
        recorded_at: DateTime<Utc>,
    ) -> ExchangeTransaction {
        make_tx(
            self.org_id,
            MAIN,
            Some(self.shift_id.clone()),
            customer_id,
            customer_name,
            side,
            currency,
            quantity,
            rate,
            recorded_at,
        )
    }

    async fn seed_draft(&self, t: ExchangeTransaction) {
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        self.store.insert_draft_in(&mut tx, &t).await.unwrap();
        tx.commit().await.unwrap();
    }

    async fn seed_confirmed(&self, t: ExchangeTransaction) {
        let at = t.recorded_at;
        let id = t.id;
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        self.store.insert_draft_in(&mut tx, &t).await.unwrap();
        self.store
            .set_status_in(
                &mut tx,
                id,
                StatusMove {
                    from: TransactionStatus::Draft,
                    to: TransactionStatus::Confirmed,
                    reason: None,
                    approval_request_id: None,
                },
                at,
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    async fn seed_completed(&self, t: ExchangeTransaction) -> String {
        let at = t.recorded_at;
        let id = t.id;
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        self.store.insert_draft_in(&mut tx, &t).await.unwrap();
        self.store
            .set_status_in(
                &mut tx,
                id,
                StatusMove {
                    from: TransactionStatus::Draft,
                    to: TransactionStatus::Confirmed,
                    reason: None,
                    approval_request_id: None,
                },
                at,
            )
            .await
            .unwrap();
        let (doc_ref, _) = self
            .store
            .complete_in(&mut tx, id, "RECEIPT-MAIN", at)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        doc_ref
    }

    async fn seed_cancelled(&self, t: ExchangeTransaction) {
        let at = t.recorded_at;
        let id = t.id;
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        self.store.insert_draft_in(&mut tx, &t).await.unwrap();
        self.store
            .set_status_in(
                &mut tx,
                id,
                StatusMove {
                    from: TransactionStatus::Draft,
                    to: TransactionStatus::Cancelled,
                    reason: Some("customer changed mind".to_string()),
                    approval_request_id: None,
                },
                at,
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// #1133 §9 case 1:
/// The four seeded rows (COMPLETED, DRAFT, CONFIRMED, CANCELLED): the completed one only.
///
/// On base: does not filter status = 'COMPLETED', crashing with UnexpectedNull because
/// DRAFT, CONFIRMED, and CANCELLED rows have NULL document_ref, or returning all 4 rows
/// if document_ref were decoded.
#[actix_web::test]
async fn the_four_seeded_rows_show_the_completed_one_only() {
    let room = Room::open().await;
    let who = Who::new("Somchai FourRows");
    let customer_id = room.reserve(MAIN, &who).await;

    let t1 = Utc.with_ymd_and_hms(2026, 10, 6, 10, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2026, 10, 6, 11, 0, 0).unwrap();
    let t3 = Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap();
    let t4 = Utc.with_ymd_and_hms(2026, 10, 6, 13, 0, 0).unwrap();

    // 1. One COMPLETED exchange (SELL 100 USD @ 33)
    let tx1 = room.make_tx(&customer_id, &who.name, Side::Sell, "USD", "100", "33", t1);
    let doc_ref = room.seed_completed(tx1).await;

    // 2. One DRAFT exchange
    let tx2 = room.make_tx(&customer_id, &who.name, Side::Buy, "USD", "200", "32.1", t2);
    room.seed_draft(tx2).await;

    // 3. One CONFIRMED exchange
    let tx3 = room.make_tx(&customer_id, &who.name, Side::Buy, "EUR", "300", "35", t3);
    room.seed_confirmed(tx3).await;

    // 4. One CANCELLED exchange
    let tx4 = room.make_tx(
        &customer_id,
        &who.name,
        Side::Sell,
        "JPY",
        "400",
        "0.25",
        t4,
    );
    room.seed_cancelled(tx4).await;

    let rows = room.page("branch=MAIN").await;

    assert_eq!(
        rows.len(),
        1,
        "device_transactions must return completed exchanges only (got: {rows:?})"
    );
    let row = &rows[0];
    assert_eq!(
        keys(row),
        set_of(&TRANSACTION_KEYS),
        "a transaction row: {row}"
    );
    assert_eq!(
        row["id"],
        json!(doc_ref),
        "id must be the completed exchange's document_ref"
    );
    assert_eq!(
        row["type"],
        json!("SELL"),
        "type must be SELL from the completed exchange line"
    );
    assert_eq!(
        decimal(&row["amount"], "SELL"),
        Decimal::from(100),
        "amount must be 100"
    );
    assert_eq!(row["currency"], json!("USD"), "currency must be USD");

    room.finish().await;
}

/// #1133 §9 case 2:
/// Paging with `before` walks over completed rows only.
///
/// Seeds completed and uncompleted rows interleaved:
/// C1 (seq 1), DRAFT (seq 2), C2 (seq 3), CONFIRMED (seq 4), CANCELLED (seq 5), C3 (seq 6).
/// Page 1 (limit 2) must return [C3, C2].
/// Page 2 (before=C2, limit 2) must return [C1].
///
/// On base: encounters uncompleted rows within cursor / order range and crashes with 500
/// UnexpectedNull or includes unfinished rows.
#[actix_web::test]
async fn paging_with_before_walks_over_completed_rows_only() {
    let room = Room::open().await;
    let who = Who::new("Paging DeviceTx");
    let customer_id = room.reserve(MAIN, &who).await;

    let t1 = Utc.with_ymd_and_hms(2026, 10, 6, 10, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2026, 10, 6, 10, 5, 0).unwrap();
    let t3 = Utc.with_ymd_and_hms(2026, 10, 6, 10, 10, 0).unwrap();
    let t4 = Utc.with_ymd_and_hms(2026, 10, 6, 10, 15, 0).unwrap();
    let t5 = Utc.with_ymd_and_hms(2026, 10, 6, 10, 20, 0).unwrap();
    let t6 = Utc.with_ymd_and_hms(2026, 10, 6, 10, 25, 0).unwrap();

    // C1: completed
    let tx1 = room.make_tx(&customer_id, &who.name, Side::Buy, "USD", "10", "32.1", t1);
    let c1_ref = room.seed_completed(tx1).await;

    // D1: draft
    let tx2 = room.make_tx(&customer_id, &who.name, Side::Buy, "USD", "20", "32.1", t2);
    room.seed_draft(tx2).await;

    // C2: completed
    let tx3 = room.make_tx(&customer_id, &who.name, Side::Sell, "USD", "30", "33", t3);
    let c2_ref = room.seed_completed(tx3).await;

    // CF1: confirmed
    let tx4 = room.make_tx(&customer_id, &who.name, Side::Buy, "EUR", "40", "35", t4);
    room.seed_confirmed(tx4).await;

    // X1: cancelled
    let tx5 = room.make_tx(&customer_id, &who.name, Side::Sell, "JPY", "50", "0.25", t5);
    room.seed_cancelled(tx5).await;

    // C3: completed
    let tx6 = room.make_tx(&customer_id, &who.name, Side::Buy, "USD", "60", "32.1", t6);
    let c3_ref = room.seed_completed(tx6).await;

    // Page 1: limit 2 should return [C3, C2]
    let page1 = room.page("branch=MAIN&limit=2").await;
    assert_eq!(page1.len(), 2, "page 1 must have 2 rows, got {page1:?}");
    assert_eq!(
        page1[0]["id"],
        json!(c3_ref),
        "page 1 first item is newest completed C3"
    );
    assert_eq!(
        page1[1]["id"],
        json!(c2_ref),
        "page 1 second item is completed C2"
    );

    // Page 2: before=C2, limit 2 should return [C1]
    let page2 = room
        .page(&format!("branch=MAIN&limit=2&before={c2_ref}"))
        .await;
    assert_eq!(page2.len(), 1, "page 2 must have 1 row (C1), got {page2:?}");
    assert_eq!(page2[0]["id"], json!(c1_ref), "page 2 item is completed C1");

    room.finish().await;
}

/// #1133 §9 case 3:
/// The compat-saved exchange's row equals today's (and ignores an unfinished exchange).
///
/// On base: crashes or includes the draft exchange.
#[actix_web::test]
async fn the_compat_saved_exchanges_row_equals_todays() {
    let room = Room::open().await;
    let who = Who::new("Compat Saved");

    // Compat-saved exchange: BUY 100 USD @ 32.1
    let compat_ref = room
        .save_compat(MAIN, &who, "BUY", "USD", "100", "32.1")
        .await;

    // Interleave an unfinished DRAFT exchange
    let draft_who = Who::new("Draft Customer");
    let draft_cust_id = room.reserve(MAIN, &draft_who).await;
    let t_draft = Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap();
    let draft_tx = room.make_tx(
        &draft_cust_id,
        &draft_who.name,
        Side::Sell,
        "EUR",
        "50",
        "35",
        t_draft,
    );
    room.seed_draft(draft_tx).await;

    let rows = room.page("branch=MAIN").await;

    assert_eq!(
        rows.len(),
        1,
        "only the completed compat-saved exchange should be returned, got {rows:?}"
    );
    let row = &rows[0];
    assert_eq!(
        keys(row),
        set_of(&TRANSACTION_KEYS),
        "a transaction row: {row}"
    );
    assert_eq!(row["id"], json!(compat_ref));
    assert_eq!(row["type"], json!("BUY"));
    assert_eq!(decimal(&row["amount"], "BUY"), Decimal::from(100));
    assert_eq!(row["currency"], json!("USD"));

    room.finish().await;
}

/// #1133 §3, §4 (done criterion 3), §8:
/// Figures (type, amount, currency) are read from the line (shop_side, quantity, currency),
/// not from the wide row (transactions.type, amount, code).
///
/// When wide columns in `transactions` differ from `transaction_lines`, the reader
/// must return the line's figures.
///
/// On base: reads `transactions.type`, `transactions.amount`, `transactions.code`.
#[actix_web::test]
async fn device_transactions_reads_type_amount_and_currency_from_line_not_wide_row() {
    let room = Room::open().await;
    let who = Who::new("Wide Columns Test");
    let customer_id = room.reserve(MAIN, &who).await;

    let t1 = Utc.with_ymd_and_hms(2026, 10, 6, 10, 0, 0).unwrap();
    // Line has: SELL, 150 USD
    let tx1 = room.make_tx(&customer_id, &who.name, Side::Sell, "USD", "150", "33", t1);
    room.seed_completed(tx1).await;

    // TX-9 (#677): the wide figures are gone; nothing left to make differ from the line.

    let rows = room.page("branch=MAIN").await;
    assert_eq!(rows.len(), 1, "exactly 1 row: {rows:?}");
    let row = &rows[0];

    assert_eq!(
        row["type"],
        json!("SELL"),
        "type must come from transaction_lines.shop_side (SELL), not wide transactions.type (got: {})",
        row["type"]
    );
    assert_eq!(
        decimal(&row["amount"], "SELL"),
        Decimal::from(150),
        "amount must come from transaction_lines.quantity (150), not wide transactions.amount (got: {})",
        row["amount"]
    );
    assert_eq!(
        row["currency"],
        json!("USD"),
        "currency must come from transaction_lines.currency (USD), not wide transactions.code (got: {})",
        row["currency"]
    );

    room.finish().await;
}
