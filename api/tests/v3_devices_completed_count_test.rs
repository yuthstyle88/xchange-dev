//! Up-front tests for TX-8i (#1134): the devices overview's completed count
//! counts COMPLETED exchanges only (design `docs/design/blueprint/quote-transaction.md` §3.2–§3.4, §4.1; #676).
//!
//! Written before the implementation by the test-writer (gemini/studio-10, rule A).
//! Do not edit, delete or weaken.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database ending in `_test`.

mod common;
mod compat_common;
mod publication_seed;
mod rates_re4_common;
mod rates_re4e_publish;
mod rpc_common;
mod shift_setup_common;
mod snapshot_common;

use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use rpc_common::{App, Call, Cookies, Desk, one_of_each_role, sign_in};
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

const ORIGIN: &str = "http://desk.test";
const MAIN: &str = "MAIN";
#[allow(dead_code)]
const RPC: &str = "/api/rpc";
const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];

static NEXT: AtomicU64 = AtomicU64::new(100);
fn next_id() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn request_id() -> String {
    format!("00000000-0000-4000-8000-{:012x}", next_id())
}

struct DeviceTestRoom {
    desk: Desk,
    app: App,
    cookies: Cookies,
    store: PgTransactionStore,
    org: Uuid,
    shift_id: String,
    t0: DateTime<Utc>,
}

impl DeviceTestRoom {
    async fn open() -> Self {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let t0 = snapshot_common::time("2026-10-06T05:00:00.000Z");
        desk.clock.set(t0);

        let org: Uuid =
            sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
                .fetch_one(&desk.db.pool)
                .await
                .expect("read MAIN organization_id");

        // Seed file keys for passport and receipts
        for key in [
            "PASSPORT-MAIN",
            "RECEIPT-1",
            "RECEIPT-2",
            "RECEIPT-3",
            "RECEIPT-4",
        ] {
            sqlx::query(
                "INSERT INTO files (key, organization_id, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ($1, $2, 'MAIN', 'owner', 'application/pdf', $3, now()) \
                 ON CONFLICT (key) DO NOTHING",
            )
            .bind(key)
            .bind(org)
            .bind(format!("sha-{key}"))
            .execute(&desk.db.pool)
            .await
            .expect("seed files");
        }

        // Seed customer rows for LIVE and DEMO transactions
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('CU-MAIN-LIVE', $1, 'CU-MAIN-LIVE', 'Somchai Live', 'LIVE', now()) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(org)
        .execute(&desk.db.pool)
        .await
        .expect("seed live customer");

        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('CU-MAIN-DEMO', $1, 'CU-MAIN-DEMO', 'Somchai Demo', 'DEMO', now()) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(org)
        .execute(&desk.db.pool)
        .await
        .expect("seed demo customer");

        // Seed USD rate
        rates_re4e_publish::set_counter_rate(&desk.db.pool, MAIN, "USD", "32.1", "33.0").await;

        // Open a shift on MAIN for LIVE transactions (CHECK (mode = 'LIVE') = (shift_id IS NOT NULL))
        shift_setup_common::open_shift_by_count(
            &desk.db.pool,
            MAIN,
            json!({ "THB": 100000 }),
            chrono::Utc::now(),
        )
        .await;

        let shift_id: String = sqlx::query_scalar(
            "SELECT id FROM shifts WHERE branch_id = 'MAIN' AND status = 'OPEN'",
        )
        .fetch_one(&desk.db.pool)
        .await
        .expect("read open shift id");

        let store = PgTransactionStore::new(desk.db.pool.clone());

        Self {
            desk,
            app,
            cookies,
            store,
            org,
            shift_id,
            t0,
        }
    }

    fn tick(&self) -> DateTime<Utc> {
        let t = self.t0 + Duration::seconds(next_id() as i64);
        self.desk.clock.set(t);
        t
    }

    async fn completed_count(&self, branch: &str) -> i64 {
        let path = format!("/api/v3/views/devices?branch={branch}");
        let mut call = Call::post(ORIGIN, &path, Vec::<u8>::new())
            .without("x-xchange-request")
            .without("content-type")
            .without("origin")
            .cookie(&self.cookies["OWNER"]);
        call.method = Method::GET;
        let answer = self.app.send(call).await;
        assert_eq!(answer.status, 200, "GET {path}: {}", answer.text);
        let body = answer.json();
        body["data"]["completedCount"]
            .as_i64()
            .unwrap_or_else(|| panic!("completedCount in {body}"))
    }

    fn make_draft(&self, branch: &str, mode: Mode, customer_id: &str) -> ExchangeTransaction {
        let id = TransactionId(Uuid::new_v4());
        let t = self.tick();
        let shift_id = if mode == Mode::Live {
            Some(self.shift_id.clone())
        } else {
            None
        };
        ExchangeTransaction {
            id,
            organization_id: self.org,
            branch_id: BranchId::new(branch),
            mode,
            status: TransactionStatus::Draft,
            document_ref: None,
            quote_id: None,
            booking_id: None,
            shift_id,
            submission_id: format!("sub-{}", id.0),
            payload_hash: "hash".to_owned(),
            customer_id: customer_id.to_owned(),
            customer_name: "Somchai".to_owned(),
            nationality: "THA".to_owned(),
            staff_id: "EMP-001".to_owned(),
            booking_no: "WALK-IN".to_owned(),
            notes: "test note".to_owned(),
            value_date: NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
            recorded_at: t,
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
                thb_amount: Money::parse("3210.00").unwrap(),
                fee_thb: Money::parse("10.00").unwrap(),
                settle_thb: Money::parse("3200.00").unwrap(),
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
                amount_thb: Money::parse("3200.00").unwrap(),
                bank_fee_thb: Money::parse("0.00").unwrap(),
                paid_at: t,
            },
            receipts: vec![],
        }
    }

    async fn seed_draft(&self, branch: &str, mode: Mode) -> TransactionId {
        let cust = if mode == Mode::Live {
            "CU-MAIN-LIVE"
        } else {
            "CU-MAIN-DEMO"
        };
        let t = self.make_draft(branch, mode, cust);
        let id = t.id;
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        self.store.insert_draft_in(&mut tx, &t).await.unwrap();
        tx.commit().await.unwrap();
        id
    }

    async fn seed_confirmed(&self, branch: &str, mode: Mode) -> TransactionId {
        let id = self.seed_draft(branch, mode).await;
        let now = self.tick();
        let mut tx = self.desk.db.pool.begin().await.unwrap();
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
                now,
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
        id
    }

    async fn seed_cancelled(&self, branch: &str, mode: Mode) -> TransactionId {
        let id = self.seed_draft(branch, mode).await;
        let now = self.tick();
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        self.store
            .set_status_in(
                &mut tx,
                id,
                StatusMove {
                    from: TransactionStatus::Draft,
                    to: TransactionStatus::Cancelled,
                    reason: Some("customer cancelled".to_owned()),
                    approval_request_id: None,
                },
                now,
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
        id
    }

    async fn seed_completed(&self, branch: &str, mode: Mode, receipt_key: &str) -> TransactionId {
        let id = self.seed_confirmed(branch, mode).await;
        let now = self.tick();
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        self.store
            .complete_in(&mut tx, id, receipt_key, now)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        id
    }

    async fn save_compat(&self, branch: &str, mode: &str) -> String {
        let t = self.tick();
        let here = BranchId::new(branch);
        let person = ReserveCustomer {
            name: "Somchai Compat".to_owned(),
            passport_no: format!("AA{:07}", next_id()),
            nationality: "THA".to_owned(),
            birth_date: String::new(),
            passport_expiry: String::new(),
            mode: mode.to_owned(),
        };
        let customer = CustomerService::new(self.desk.db.branch_tx())
            .reserve(&here, person.clone(), t)
            .await
            .expect("reserve customer")
            .customer_id;
        let passport = self
            .desk
            .files
            .upload(&here, "owner", "image/jpeg", JPEG, t)
            .await
            .expect("upload passport");
        let cmd = SaveTransaction {
            customer: person,
            submission_id: format!("sub-compat-{:016x}", next_id()),
            customer_id: customer,
            side: "BUY".to_owned(),
            currency: "USD".to_owned(),
            amount: "100".to_owned(),
            rate: "32.1".to_owned(),
            fee: "0".to_owned(),
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
        self.desk
            .workflow
            .save(&here, cmd, "owner", t)
            .await
            .unwrap_or_else(|e| panic!("compat save: {e:?}"))
            .document_ref
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// §9 row 1: The four seeded rows (COMPLETED, DRAFT, CONFIRMED, CANCELLED) count 1.
/// An unfinished (DRAFT, CONFIRMED) or cancelled exchange is never counted as completed.
#[actix_web::test]
async fn the_four_seeded_rows_count_one_completed_exchange_only() {
    let room = DeviceTestRoom::open().await;

    // Seed the 4 LIVE rows through TX-2's store:
    room.seed_completed(MAIN, Mode::Live, "RECEIPT-1").await;
    room.seed_draft(MAIN, Mode::Live).await;
    room.seed_confirmed(MAIN, Mode::Live).await;
    room.seed_cancelled(MAIN, Mode::Live).await;

    let count = room.completed_count(MAIN).await;
    assert_eq!(
        count, 1,
        "only the COMPLETED row is counted; DRAFT, CONFIRMED, CANCELLED are not"
    );

    room.finish().await;
}

/// Compat-saved exchange (which is COMPLETED with wide and child figures equal)
/// alongside unfinished and cancelled exchanges counts only the completed exchanges.
#[actix_web::test]
async fn a_compat_saved_exchange_beside_unfinished_and_cancelled_rows_counts_completed_only() {
    let room = DeviceTestRoom::open().await;

    // One compat-saved exchange (COMPLETED)
    room.save_compat(MAIN, "LIVE").await;

    // Unfinished and cancelled exchanges through TX-2 store
    room.seed_draft(MAIN, Mode::Live).await;
    room.seed_confirmed(MAIN, Mode::Live).await;
    room.seed_cancelled(MAIN, Mode::Live).await;

    let count = room.completed_count(MAIN).await;
    assert_eq!(
        count, 1,
        "only the compat-saved COMPLETED exchange is counted"
    );

    room.finish().await;
}

/// §9 row 2: A DEMO completed row is not counted, as today.
/// Furthermore, unfinished LIVE rows (DRAFT, CONFIRMED, CANCELLED) are not counted.
#[actix_web::test]
async fn a_demo_completed_row_is_not_counted_and_unfinished_live_rows_are_ignored() {
    let room = DeviceTestRoom::open().await;

    // A completed DEMO exchange
    room.seed_completed(MAIN, Mode::Demo, "RECEIPT-2").await;

    // Unfinished LIVE exchanges
    room.seed_draft(MAIN, Mode::Live).await;
    room.seed_confirmed(MAIN, Mode::Live).await;
    room.seed_cancelled(MAIN, Mode::Live).await;

    let count = room.completed_count(MAIN).await;
    assert_eq!(
        count, 0,
        "DEMO completed is ignored, and LIVE unfinished rows are not counted"
    );

    room.finish().await;
}

/// PENDING_APPROVAL row (design §4.1: status IN ('DRAFT','PENDING_APPROVAL','CONFIRMED'))
/// is unfinished and is never counted as completed.
#[actix_web::test]
async fn pending_approval_exchange_is_not_counted_as_completed() {
    let room = DeviceTestRoom::open().await;

    // One COMPLETED exchange
    room.seed_completed(MAIN, Mode::Live, "RECEIPT-3").await;

    // One PENDING_APPROVAL exchange
    let id = room.seed_draft(MAIN, Mode::Live).await;
    let now = room.tick();

    let req_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES ($1, 'TRANSACTION_OVER_LIMIT', $2, $3, 'hash', 'v1', '{}'::jsonb, 'owner', now(), 'PENDING', $4)",
    )
    .bind(req_id)
    .bind(MAIN)
    .bind(id.0.to_string())
    .bind(request_id())
    .execute(&room.desk.db.pool)
    .await
    .unwrap();

    let mut tx = room.desk.db.pool.begin().await.unwrap();
    room.store
        .set_status_in(
            &mut tx,
            id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::PendingApproval,
                reason: None,
                approval_request_id: Some(req_id),
            },
            now,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let count = room.completed_count(MAIN).await;
    assert_eq!(
        count, 1,
        "PENDING_APPROVAL is not counted; only COMPLETED is counted"
    );

    room.finish().await;
}
