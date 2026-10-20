//! Ticket #1130 (TX-8e) — `admin_customers` view counts completed exchanges only,
//! sums `transaction_lines.thb_amount` (not wide `principal`), and ignores unfinished
//! transactions in customer stats and paging cursor.
//!
//! Up-front tests written by the test-writer (gemini/warut-2) under Rule A.
//! Based on the TX-2 branch (PR 1118, commit a6ceae2).
//!
//! Edge cases from #1130 §9:
//! 1. A customer with the four seeded rows (COMPLETED, DRAFT, CONFIRMED, CANCELLED):
//!    completed = 1, volume = the completed line's thb_amount, last_at = COMPLETED recorded_at.
//! 2. A customer with only unfinished exchanges shows completed = 0, volume = 0.00, last_at = null.
//! 3. The paging cursor (last_at) ignores an unfinished exchange recorded later.
//! 4. The compat-saved exchange gives today's figures (and ignores an unfinished exchange).
//! 5. Volume sums lines' thb_amount, not wide transactions.principal.

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
mod rpc_common;
mod shift_holder_common;
mod shift_setup_common;
mod snapshot_common;

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
const CUSTOMERS: &str = "admin_customers";
const MAIN: &str = "MAIN";

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

fn view_path(view: &str, query: &str) -> String {
    if query.is_empty() {
        format!("/api/v3/views/{view}")
    } else {
        format!("/api/v3/views/{view}?{query}")
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

fn instant(row: &Value, field: &str) -> DateTime<Utc> {
    let text = row[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} is a string: {row}"));
    DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("{field} is an RFC 3339 instant ({e}): {row}"))
        .with_timezone(&Utc)
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

#[derive(Clone)]
struct Reserved {
    id: String,
    #[allow(dead_code)]
    at: DateTime<Utc>,
}

fn make_tx(
    org_id: Uuid,
    branch_id: &str,
    shift_id: Option<String>,
    customer_id: &str,
    customer_name: &str,
    thb_amount: &str,
    recorded_at: DateTime<Utc>,
) -> ExchangeTransaction {
    let id = TransactionId(Uuid::new_v4());
    let amount = Money::parse(thb_amount).unwrap();
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
            shop_side: Side::Buy,
            currency: CurrencyCode::parse("USD").unwrap(),
            quantity: ForeignAmount::from_decimal(Decimal::from(100)),
            unit_size: UnitSize::parse("1").unwrap(),
            rate_snapshot: Rate::parse("32.1").unwrap(),
            reference_rate: Some(Rate::parse("32.05").unwrap()),
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

    fn make_tx(
        &self,
        customer_id: &str,
        customer_name: &str,
        thb_amount: &str,
        recorded_at: DateTime<Utc>,
    ) -> ExchangeTransaction {
        make_tx(
            self.org_id,
            MAIN,
            Some(self.shift_id.clone()),
            customer_id,
            customer_name,
            thb_amount,
            recorded_at,
        )
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
        let call = get(path);
        let call = if who.is_empty() {
            call
        } else {
            call.cookie(&self.cookie(who))
        };
        self.app.send(call).await
    }

    async fn view(&self, who: &str, view: &str, query: &str) -> Value {
        let path = view_path(view, query);
        let answer = self.get(who, &path).await;
        assert_eq!(
            answer.status, 200,
            "GET {path} as {who:?}: {:.400}",
            answer.text
        );
        answer.json()
    }

    async fn rows(&self, who: &str, view: &str, query: &str) -> Vec<Value> {
        let body = self.view(who, view, query).await;
        assert_eq!(
            body["changed"],
            json!(true),
            "GET {}: {body}",
            view_path(view, query)
        );
        body.get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| panic!("GET {}: a list in data: {body}", view_path(view, query)))
    }

    async fn customer(&self, id: &str) -> Value {
        let rows = self.rows("OWNER", CUSTOMERS, "branch=MAIN&limit=100").await;
        rows.iter()
            .find(|r| r["id"] == json!(id))
            .cloned()
            .unwrap_or_else(|| panic!("{id} among {rows:?}"))
    }

    async fn page(&self, query: &str) -> Vec<Value> {
        self.rows("OWNER", CUSTOMERS, query).await
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

    async fn reserve(&self, branch: &'static str, who: &Who) -> Reserved {
        let t = self.tick();
        let id = CustomerService::new(self.desk.db.branch_tx())
            .reserve(&BranchId::new(branch), who.command(), t)
            .await
            .unwrap_or_else(|e| panic!("reserve {} in {branch}: {e:?}", who.name))
            .customer_id;
        Reserved { id, at: t }
    }

    async fn save_compat(&self, branch: &'static str, who: &Who) -> (String, DateTime<Utc>) {
        let customer = self.reserve(branch, who).await;
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
            customer_id: customer.id.clone(),
            side: "BUY".to_owned(),
            currency: "USD".to_owned(),
            amount: "100".to_owned(),
            rate: "32.1".to_owned(),
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
        let _saved = self
            .desk
            .workflow
            .save(&here, cmd, &saver, t)
            .await
            .unwrap_or_else(|e| panic!("save an exchange of {} in {branch}: {e:?}", who.name));
        (customer.id, t)
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

    async fn seed_completed(&self, t: ExchangeTransaction) {
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
        self.store
            .complete_in(&mut tx, id, "RECEIPT-MAIN", at)
            .await
            .unwrap();
        tx.commit().await.unwrap();
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

/// #1130 §9 case 1:
/// A customer with the four seeded rows (COMPLETED, DRAFT, CONFIRMED, CANCELLED):
/// completed = 1, volume = the completed line's thb_amount, last_at = COMPLETED exchange's recorded_at.
///
/// On base: counts all rows as completed (completed = 4) and sums all principals (volume = 32100.00).
#[actix_web::test]
async fn customer_with_four_seeded_rows_counts_one_completed_and_sums_completed_line() {
    let room = Room::open().await;
    let who = Who::new("Somchai FourRows");
    let customer = room.reserve(MAIN, &who).await;

    let t1 = Utc.with_ymd_and_hms(2026, 10, 6, 10, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2026, 10, 6, 11, 0, 0).unwrap();
    let t3 = Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap();
    let t4 = Utc.with_ymd_and_hms(2026, 10, 6, 13, 0, 0).unwrap();

    // 1. One COMPLETED exchange: thb_amount 3210.00
    let tx1 = room.make_tx(&customer.id, &who.name, "3210.00", t1);
    room.seed_completed(tx1).await;

    // 2. One DRAFT exchange: thb_amount 6420.00
    let tx2 = room.make_tx(&customer.id, &who.name, "6420.00", t2);
    room.seed_draft(tx2).await;

    // 3. One CONFIRMED exchange: thb_amount 9630.00
    let tx3 = room.make_tx(&customer.id, &who.name, "9630.00", t3);
    room.seed_confirmed(tx3).await;

    // 4. One CANCELLED exchange: thb_amount 12840.00
    let tx4 = room.make_tx(&customer.id, &who.name, "12840.00", t4);
    room.seed_cancelled(tx4).await;

    let row = room.customer(&customer.id).await;

    assert_eq!(
        row["completed"],
        json!(1),
        "completed must count only COMPLETED exchanges (got: {})",
        row["completed"]
    );
    assert_eq!(
        row["volume"],
        json!("3210.00"),
        "volume must sum only COMPLETED lines' thb_amount (got: {})",
        row["volume"]
    );
    assert_eq!(
        instant(&row, "lastAt"),
        t1,
        "lastAt must be the COMPLETED exchange recorded_at (got: {})",
        row["lastAt"]
    );

    room.finish().await;
}

/// #1130 §3 & §8:
/// A customer with only unfinished exchanges shows completed = 0, volume = 0.00, last_at = null.
///
/// On base: counts draft and cancelled rows (completed = 3, volume > 0, lastAt != null).
#[actix_web::test]
async fn customer_with_only_unfinished_exchanges_shows_completed_0_volume_zero_and_null_last_at() {
    let room = Room::open().await;
    let who = Who::new("Anan Unfinished");
    let customer = room.reserve(MAIN, &who).await;

    let t1 = Utc.with_ymd_and_hms(2026, 10, 6, 10, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2026, 10, 6, 11, 0, 0).unwrap();
    let t3 = Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap();

    let tx1 = room.make_tx(&customer.id, &who.name, "6420.00", t1);
    room.seed_draft(tx1).await;

    let tx2 = room.make_tx(&customer.id, &who.name, "9630.00", t2);
    room.seed_confirmed(tx2).await;

    let tx3 = room.make_tx(&customer.id, &who.name, "12840.00", t3);
    room.seed_cancelled(tx3).await;

    let row = room.customer(&customer.id).await;

    assert_eq!(
        row["completed"],
        json!(0),
        "completed must be 0 for unfinished exchanges only (got: {})",
        row["completed"]
    );
    assert_eq!(
        row["volume"],
        json!("0.00"),
        "volume must be 0.00 for unfinished exchanges only (got: {})",
        row["volume"]
    );
    assert_eq!(
        row["lastAt"],
        Value::Null,
        "lastAt must be null for customer with no completed exchanges (got: {})",
        row["lastAt"]
    );

    room.finish().await;
}

/// #1130 §9 case 2:
/// The paging cursor (last_at) ignores an unfinished exchange recorded later.
///
/// Setup:
/// - Customer C has a COMPLETED exchange at t0 (09:00).
/// - Customer A has a COMPLETED exchange at t1 (10:00) and later a DRAFT exchange at t3 (12:00).
/// - Customer B has a COMPLETED exchange at t2 (11:00).
/// Since t2 > t1 > t0, Customer B ranks first, Customer A ranks second, Customer C ranks third.
/// Paging after Customer B (before=Customer B) must return Customer A with lastAt = t1.
/// Paging after Customer A (before=Customer A) must return exactly [Customer C].
///
/// Mutant killing:
/// If the cursor query at views/admin_customers.rs:69 keeps MAX(t.recorded_at) without status = 'COMPLETED',
/// Customer A's cursor evaluates to t3 (the draft time). Since t2 < t3, the page after Customer A
/// also returns Customer B (t2), causing Customer B to appear twice across pages.
/// Correct behavior: Customer A's cursor evaluates to t1, so the page after Customer A is exactly [Customer C].
#[actix_web::test]
async fn paging_cursor_last_at_ignores_an_unfinished_exchange_recorded_later() {
    let room = Room::open().await;

    let who_a = Who::new("Alice Paging");
    let cust_a = room.reserve(MAIN, &who_a).await;

    let who_b = Who::new("Bob Paging");
    let cust_b = room.reserve(MAIN, &who_b).await;

    let who_c = Who::new("Charlie Paging");
    let cust_c = room.reserve(MAIN, &who_c).await;

    let t0 = Utc.with_ymd_and_hms(2026, 10, 6, 9, 0, 0).unwrap();
    let t1 = Utc.with_ymd_and_hms(2026, 10, 6, 10, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2026, 10, 6, 11, 0, 0).unwrap();
    let t3 = Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap();

    // Customer C: COMPLETED at t0
    let tx_c_comp = room.make_tx(&cust_c.id, &who_c.name, "3210.00", t0);
    room.seed_completed(tx_c_comp).await;

    // Customer A: COMPLETED at t1 (> t0)
    let tx_a_comp = room.make_tx(&cust_a.id, &who_a.name, "3210.00", t1);
    room.seed_completed(tx_a_comp).await;

    // Customer B: COMPLETED at t2 (> t1 > t0)
    let tx_b_comp = room.make_tx(&cust_b.id, &who_b.name, "3210.00", t2);
    room.seed_completed(tx_b_comp).await;

    // Customer A: DRAFT recorded later at t3 (> t2 > t1 > t0)
    let tx_a_draft = room.make_tx(&cust_a.id, &who_a.name, "6420.00", t3);
    room.seed_draft(tx_a_draft).await;

    // Page 1: limit 1 must return Customer B (whose completed exchange at t2 is newest)
    let page1 = room.page("branch=MAIN&limit=1").await;
    assert_eq!(page1.len(), 1, "page 1 should return 1 row");
    assert_eq!(
        page1[0]["id"],
        json!(cust_b.id),
        "Customer B must rank first because t2 > t1 (got: {})",
        page1[0]["name"]
    );

    // Page 2: paged after Customer B (before=cust_b.id) must find Customer A
    let page2 = room
        .page(&format!("branch=MAIN&limit=10&before={}", cust_b.id))
        .await;
    let found_a = page2.iter().find(|r| r["id"] == json!(cust_a.id));
    assert!(
        found_a.is_some(),
        "Customer A must appear on page 2 after Customer B (page2: {page2:?})"
    );

    let row_a = found_a.unwrap();
    assert_eq!(
        instant(row_a, "lastAt"),
        t1,
        "Customer A's lastAt must be t1 (the completed exchange), not t3 (the draft)"
    );

    // Page after Customer A (before=cust_a.id): must return exactly [Customer C].
    // If the cursor lookup at views/admin_customers.rs:69 considers the draft at t3,
    // Customer A's cursor evaluates to t3 (> t2), causing Customer B (t2) to be returned again.
    let page_after_a = room
        .page(&format!("branch=MAIN&limit=10&before={}", cust_a.id))
        .await;
    assert_eq!(
        page_after_a.len(),
        1,
        "page after Customer A must return exactly [Customer C] (got: {page_after_a:?})"
    );
    assert_eq!(
        page_after_a[0]["id"],
        json!(cust_c.id),
        "page after Customer A must be Customer C (got: {})",
        page_after_a[0]["name"]
    );
    assert_eq!(
        instant(&page_after_a[0], "lastAt"),
        t0,
        "Customer C's lastAt must be t0"
    );

    room.finish().await;
}

/// #1130 §9 case 3:
/// The compat-saved exchange gives today's figures (and ignores an unfinished exchange).
///
/// On base: counts the unfinished draft row alongside the compat exchange (completed = 2).
#[actix_web::test]
async fn compat_saved_exchange_gives_todays_figures_and_ignores_unfinished_rows() {
    let room = Room::open().await;
    let who = Who::new("Kamonwan Compat");

    // Compat save: 100 USD @ 32.1 = 3210.00
    let (customer_id, compat_at) = room.save_compat(MAIN, &who).await;

    // Add an unfinished DRAFT exchange for the same customer
    let t_draft = compat_at + Duration::minutes(15);
    let draft_tx = room.make_tx(&customer_id, &who.name, "6420.00", t_draft);
    room.seed_draft(draft_tx).await;

    let row = room.customer(&customer_id).await;

    assert_eq!(
        row["completed"],
        json!(1),
        "completed must count only the compat exchange, ignoring draft (got: {})",
        row["completed"]
    );
    assert_eq!(
        row["volume"],
        json!("3210.00"),
        "volume must equal compat exchange's 3210.00, ignoring draft (got: {})",
        row["volume"]
    );
    assert_eq!(
        instant(&row, "lastAt"),
        compat_at,
        "lastAt must match compat exchange time (got: {})",
        row["lastAt"]
    );

    room.finish().await;
}

/// #1130 §3 & Done criterion 3:
/// Volume is the sum of lines' thb_amount and does not read wide `transactions.principal`.
///
/// When `transactions.principal` is zeroed (simulating wide column dropping in TX-9),
/// volume must still be correctly calculated from `transaction_lines.thb_amount`.
///
/// On base: reads `SUM(t.principal)`, resulting in volume = "0.00".
#[actix_web::test]
async fn customer_volume_sums_transaction_lines_thb_amount_not_wide_principal() {
    let room = Room::open().await;
    let who = Who::new("Vichai LineSum");
    let customer = room.reserve(MAIN, &who).await;

    let t1 = Utc.with_ymd_and_hms(2026, 10, 6, 10, 0, 0).unwrap();
    let tx1 = room.make_tx(&customer.id, &who.name, "2500.00", t1);
    room.seed_completed(tx1).await;

    // TX-9 (#677) dropped `transactions.principal`; the volume can only come from the lines.

    let row = room.customer(&customer.id).await;

    assert_eq!(row["completed"], json!(1), "{row}");
    assert_eq!(
        row["volume"],
        json!("2500.00"),
        "volume must sum transaction_lines.thb_amount (2500.00), not wide principal (got: {})",
        row["volume"]
    );

    room.finish().await;
}
