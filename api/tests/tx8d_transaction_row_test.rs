//! Ticket #1129 (TX-8d) — Up-front tests (rule A)
//!
//! Shared transaction row (`dto/v3/transaction_row.rs`) and the four list views built on it
//! (`admin_transactions`, `admin_slips`, `admin_overview`, `admin_customer_history`).
//!
//! Rule A verification:
//! 1. Seed through TX-2's store:
//!    - one COMPLETED exchange (with payment slip, without transaction_files SLIP)
//!    - one DRAFT exchange
//!    - one CONFIRMED exchange (with payment slip)
//!    - one CANCELLED exchange (with cancel_reason)
//!    - one compat-saved COMPLETED exchange (wide and child figures equal, with slip)
//!    - one CANCELLED exchange on a separate date to verify overview day with only cancelled rows.
//!    - mutate wide row columns of store-COMPLETED exchange to ensure figures are read from children.
//! 2. Red on main for wrong result (not compile error).
//! 3. Green on throwaway fix.
//! 4. Red again on two mutants: status filter removed, and reading wide figures instead of children.

mod common;
mod compat_common;
mod kiosk_http_common;
mod rpc_common;
mod snapshot_common;

use actix_web::http::Method;
use chrono::{NaiveDate, TimeZone, Utc};
use kiosk_http_common::Counter;
use rpc_common::{Answer, Call};
use rust_decimal::Decimal;
use serde_json::Value;
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

const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F'];

fn get(path: &str) -> Call {
    let mut call = Call::post(kiosk_http_common::ORIGIN, path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

async fn get_as_owner(counter: &Counter, path: &str) -> Answer {
    let call = get(path).cookie(
        counter
            .cookies
            .get("owner")
            .expect("the owner is signed in"),
    );
    counter.app.send(call).await
}

async fn view(counter: &Counter, path: &str) -> Value {
    let answer = get_as_owner(counter, path).await;
    assert_eq!(answer.status, 200, "GET {path}: {:.400}", answer.text);
    let body = answer.json();
    body.get("data")
        .cloned()
        .unwrap_or_else(|| panic!("GET {path}: data in {body}"))
}

async fn reserve_customer(
    counter: &Counter,
    branch: &str,
    name: &str,
    at: chrono::DateTime<Utc>,
) -> (String, ReserveCustomer) {
    let p = ReserveCustomer {
        name: name.to_owned(),
        passport_no: "AA1234567".to_owned(),
        nationality: "THA".to_owned(),
        birth_date: String::new(),
        passport_expiry: String::new(),
        mode: "LIVE".to_owned(),
    };
    let customer_id = CustomerService::new(counter.desk.db.branch_tx())
        .reserve(&BranchId::new(branch), p.clone(), at)
        .await
        .unwrap_or_else(|e| panic!("reserve {name} in {branch}: {e:?}"))
        .customer_id;
    (customer_id, p)
}

#[allow(dead_code)]
#[derive(Clone)]
struct SeededIds {
    store_completed_ref: String,
    store_completed_id: Uuid,
    draft_id: Uuid,
    confirmed_id: Uuid,
    cancelled_id: Uuid,
    cancelled_day2_id: Uuid,
    compat_ref: String,
    compat_slip_url: String,
    customer_id: String,
    shift_id: String,
}

struct Harness {
    counter: Counter,
    ids: SeededIds,
}

impl Harness {
    async fn start() -> Self {
        let counter = Counter::open(&[]).await;
        let pool = counter.desk.db.pool.clone();

        let org_id: Uuid =
            sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
                .fetch_one(&pool)
                .await
                .expect("fetch org id");

        let shift_id: String = sqlx::query_scalar(
            "SELECT id FROM shifts WHERE branch_id = 'MAIN' AND status = 'OPEN'",
        )
        .fetch_one(&pool)
        .await
        .expect("fetch shift id");

        let val_date = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
        let t0 = Utc.with_ymd_and_hms(2026, 10, 6, 5, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 10, 6, 5, 1, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 10, 6, 5, 2, 0).unwrap();
        let t3 = Utc.with_ymd_and_hms(2026, 10, 6, 5, 3, 0).unwrap();
        let t4 = Utc.with_ymd_and_hms(2026, 10, 6, 5, 4, 0).unwrap();

        let (customer_id, person) = reserve_customer(&counter, "MAIN", "Alice Test", t0).await;

        for key in [
            "PASSPORT-MAIN",
            "SLIP-1",
            "SLIP-CONFIRMED",
            "RECEIPT-1",
            "SLIP-MUTATED",
            "SLIP-COMPAT",
        ] {
            sqlx::query(
                "INSERT INTO files (key, organization_id, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ($1, $2, 'MAIN', 'owner', 'application/pdf', 'sha', now()) \
                 ON CONFLICT (key) DO NOTHING",
            )
            .bind(key)
            .bind(org_id)
            .execute(&pool)
            .await
            .expect("insert file");
        }

        let store = PgTransactionStore::new(pool.clone());

        // 1. COMPLETED via TX-2 store at t0: BUY 100 USD at 32.10, fee 10.00, settle 3200.00
        // Payment has slip_file_key = "SLIP-1".
        let store_completed_id = Uuid::new_v4();
        let tx1 = ExchangeTransaction {
            id: TransactionId(store_completed_id),
            organization_id: org_id,
            branch_id: BranchId::new("MAIN".to_string()),
            mode: Mode::Live,
            status: TransactionStatus::Draft,
            document_ref: None,
            quote_id: None,
            booking_id: None,
            shift_id: Some(shift_id.clone()),
            submission_id: format!("sub-{}", store_completed_id),
            payload_hash: format!("hash-{}", store_completed_id),
            customer_id: customer_id.clone(),
            customer_name: "Alice Test".to_owned(),
            nationality: "THA".to_owned(),
            staff_id: "EMP-001".to_owned(),
            booking_no: "WALK-IN".to_owned(),
            notes: "store completed tx".to_owned(),
            value_date: val_date,
            recorded_at: t0,
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
                rate_snapshot: Rate::parse("32.10").unwrap(),
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
                method: PaymentMethod::BankTransfer,
                reference: "TRX-STORE-COMPLETED".to_owned(),
                account: "KBank 1".to_owned(),
                slip_file_key: Some("SLIP-1".to_owned()),
                amount_thb: Money::parse("3200.00").unwrap(),
                bank_fee_thb: Money::parse("0.00").unwrap(),
                paid_at: t0,
            },
            receipts: vec![],
        };
        let mut sqlx_tx = pool.begin().await.unwrap();
        store.insert_draft_in(&mut sqlx_tx, &tx1).await.unwrap();
        store
            .set_status_in(
                &mut sqlx_tx,
                tx1.id,
                StatusMove {
                    from: TransactionStatus::Draft,
                    to: TransactionStatus::Confirmed,
                    reason: None,
                    approval_request_id: None,
                },
                t0,
            )
            .await
            .unwrap();
        let (store_completed_ref, _) = store
            .complete_in(&mut sqlx_tx, tx1.id, "RECEIPT-1", t0)
            .await
            .unwrap();
        sqlx_tx.commit().await.unwrap();

        // Mutate the wide row figures of the store-COMPLETED exchange to different/wrong values (§9, #1154 pattern).
        sqlx::query(
            "UPDATE transactions \
             SET type = 'SELL', code = 'EUR', amount = 9999.00, rate = 99.000000, \
                 reference_rate = 98.000000, principal = 99999.00, fee = 888.00, \
                 bank_fee = 777.00, received = 66666.00, paid = 55555.00, \
                 method = 'PromptPay', payment_ref = 'MUTATED_REF', account = 'MUTATED_ACC' \
             WHERE id = $1",
        )
        .bind(store_completed_id)
        .execute(&pool)
        .await
        .expect("mutate wide columns");

        sqlx::query(
            "INSERT INTO transaction_files (transaction_id, kind, file_key) \
             VALUES ($1, 'SLIP', 'SLIP-MUTATED') \
             ON CONFLICT (transaction_id, kind) DO UPDATE SET file_key = 'SLIP-MUTATED'",
        )
        .bind(store_completed_id)
        .execute(&pool)
        .await
        .expect("insert mutated slip into transaction_files");

        // 2. DRAFT via TX-2 store at t1: BUY 100 USD at 32.10, fee 0.00, settle 3210.00
        let draft_id = Uuid::new_v4();
        let tx2 = ExchangeTransaction {
            id: TransactionId(draft_id),
            organization_id: org_id,
            branch_id: BranchId::new("MAIN".to_string()),
            mode: Mode::Live,
            status: TransactionStatus::Draft,
            document_ref: None,
            quote_id: None,
            booking_id: None,
            shift_id: Some(shift_id.clone()),
            submission_id: format!("sub-{}", draft_id),
            payload_hash: format!("hash-{}", draft_id),
            customer_id: customer_id.clone(),
            customer_name: "Alice Test".to_owned(),
            nationality: "THA".to_owned(),
            staff_id: "EMP-001".to_owned(),
            booking_no: "WALK-IN".to_owned(),
            notes: "draft tx".to_owned(),
            value_date: val_date,
            recorded_at: t1,
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
                rate_snapshot: Rate::parse("32.10").unwrap(),
                reference_rate: Some(Rate::parse("32.05").unwrap()),
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
                reference: "".to_owned(),
                account: "Cash drawer 1".to_owned(),
                slip_file_key: None,
                amount_thb: Money::parse("3210.00").unwrap(),
                bank_fee_thb: Money::parse("0.00").unwrap(),
                paid_at: t1,
            },
            receipts: vec![],
        };
        let mut sqlx_tx = pool.begin().await.unwrap();
        store.insert_draft_in(&mut sqlx_tx, &tx2).await.unwrap();
        sqlx_tx.commit().await.unwrap();

        // 3. CONFIRMED via TX-2 store at t2: SELL 200 USD at 32.60, fee 0.00, settle 6520.00
        // Payment has slip_file_key = "SLIP-CONFIRMED".
        let confirmed_id = Uuid::new_v4();
        let tx3 = ExchangeTransaction {
            id: TransactionId(confirmed_id),
            organization_id: org_id,
            branch_id: BranchId::new("MAIN".to_string()),
            mode: Mode::Live,
            status: TransactionStatus::Draft,
            document_ref: None,
            quote_id: None,
            booking_id: None,
            shift_id: Some(shift_id.clone()),
            submission_id: format!("sub-{}", confirmed_id),
            payload_hash: format!("hash-{}", confirmed_id),
            customer_id: customer_id.clone(),
            customer_name: "Alice Test".to_owned(),
            nationality: "THA".to_owned(),
            staff_id: "EMP-001".to_owned(),
            booking_no: "WALK-IN".to_owned(),
            notes: "confirmed tx".to_owned(),
            value_date: val_date,
            recorded_at: t2,
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
                shop_side: Side::Sell,
                currency: CurrencyCode::parse("USD").unwrap(),
                quantity: ForeignAmount::from_decimal(Decimal::from(200)),
                unit_size: UnitSize::parse("1").unwrap(),
                rate_snapshot: Rate::parse("32.60").unwrap(),
                reference_rate: Some(Rate::parse("32.65").unwrap()),
                thb_amount: Money::parse("6520.00").unwrap(),
                fee_thb: Money::parse("0.00").unwrap(),
                settle_thb: Money::parse("6520.00").unwrap(),
                cost_basis_thb: None,
                realized_margin_thb: None,
                line_no: 1,
            }],
            payment: TransactionPayment {
                id: Uuid::new_v4(),
                payment_no: 1,
                method: PaymentMethod::BankTransfer,
                reference: "TRX-CONFIRMED".to_owned(),
                account: "KBank 1".to_owned(),
                slip_file_key: Some("SLIP-CONFIRMED".to_owned()),
                amount_thb: Money::parse("6520.00").unwrap(),
                bank_fee_thb: Money::parse("0.00").unwrap(),
                paid_at: t2,
            },
            receipts: vec![],
        };
        let mut sqlx_tx = pool.begin().await.unwrap();
        store.insert_draft_in(&mut sqlx_tx, &tx3).await.unwrap();
        store
            .set_status_in(
                &mut sqlx_tx,
                tx3.id,
                StatusMove {
                    from: TransactionStatus::Draft,
                    to: TransactionStatus::Confirmed,
                    reason: None,
                    approval_request_id: None,
                },
                t2,
            )
            .await
            .unwrap();
        sqlx_tx.commit().await.unwrap();

        // 4. CANCELLED via TX-2 store at t3: BUY 50 USD at 32.10, fee 0.00, settle 1605.00
        let cancelled_id = Uuid::new_v4();
        let tx4 = ExchangeTransaction {
            id: TransactionId(cancelled_id),
            organization_id: org_id,
            branch_id: BranchId::new("MAIN".to_string()),
            mode: Mode::Live,
            status: TransactionStatus::Draft,
            document_ref: None,
            quote_id: None,
            booking_id: None,
            shift_id: Some(shift_id.clone()),
            submission_id: format!("sub-{}", cancelled_id),
            payload_hash: format!("hash-{}", cancelled_id),
            customer_id: customer_id.clone(),
            customer_name: "Alice Test".to_owned(),
            nationality: "THA".to_owned(),
            staff_id: "EMP-001".to_owned(),
            booking_no: "WALK-IN".to_owned(),
            notes: "cancelled tx".to_owned(),
            value_date: val_date,
            recorded_at: t3,
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
                quantity: ForeignAmount::from_decimal(Decimal::from(50)),
                unit_size: UnitSize::parse("1").unwrap(),
                rate_snapshot: Rate::parse("32.10").unwrap(),
                reference_rate: Some(Rate::parse("32.05").unwrap()),
                thb_amount: Money::parse("1605.00").unwrap(),
                fee_thb: Money::parse("0.00").unwrap(),
                settle_thb: Money::parse("1605.00").unwrap(),
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
                amount_thb: Money::parse("1605.00").unwrap(),
                bank_fee_thb: Money::parse("0.00").unwrap(),
                paid_at: t3,
            },
            receipts: vec![],
        };
        let mut sqlx_tx = pool.begin().await.unwrap();
        store.insert_draft_in(&mut sqlx_tx, &tx4).await.unwrap();
        store
            .set_status_in(
                &mut sqlx_tx,
                tx4.id,
                StatusMove {
                    from: TransactionStatus::Draft,
                    to: TransactionStatus::Cancelled,
                    reason: Some("Customer changed mind".to_owned()),
                    approval_request_id: None,
                },
                t3,
            )
            .await
            .unwrap();
        sqlx_tx.commit().await.unwrap();

        // 5. COMPAT-SAVED COMPLETED exchange at t4: SELL 100 USD at 32.60, fee 0.00, settle 3260.00
        let here = BranchId::new("MAIN".to_string());
        let passport_key = counter
            .desk
            .files
            .upload(&here, "owner", "image/jpeg", JPEG, t4)
            .await
            .expect("upload passport");
        let slip_key = counter
            .desk
            .files
            .upload(&here, "owner", "image/jpeg", JPEG, t4)
            .await
            .expect("upload slip");

        let cmd = SaveTransaction {
            customer: person,
            submission_id: format!("sub-compat-{}", Uuid::new_v4()),
            customer_id: customer_id.clone(),
            side: "SELL".to_owned(),
            currency: "USD".to_owned(),
            amount: "100".to_owned(),
            rate: "32.6".to_owned(),
            fee: "0.00".to_owned(),
            bank_fee: "0.00".to_owned(),
            method: "Cash".to_owned(),
            payment_ref: "".to_owned(),
            account: "Cash drawer 1".to_owned(),
            staff: "EMP-001".to_owned(),
            booking: "WALK-IN".to_owned(),
            notes: "compat saved tx".to_owned(),
            value_date: "2026-10-06".to_owned(),
            paid_at: "2026-10-06T05:04:00.000Z".to_owned(),
            payment_confirmed: true,
            identity_confirmed: true,
            passport: Some(passport_key),
            slip: Some(slip_key.clone()),
            reference_rate: None,
        };
        let compat_saved = counter
            .desk
            .workflow
            .save(&here, cmd, "owner", t4)
            .await
            .expect("save compat");

        // 6. CANCELLED exchange on day 2 (2026-10-07) to test overview day with only cancelled rows.
        let val_date_2 = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
        let t_day2 = Utc.with_ymd_and_hms(2026, 10, 7, 5, 0, 0).unwrap();
        let cancelled_day2_id = Uuid::new_v4();
        let tx_day2 = ExchangeTransaction {
            id: TransactionId(cancelled_day2_id),
            organization_id: org_id,
            branch_id: BranchId::new("MAIN".to_string()),
            mode: Mode::Live,
            status: TransactionStatus::Draft,
            document_ref: None,
            quote_id: None,
            booking_id: None,
            shift_id: Some(shift_id.clone()),
            submission_id: format!("sub-{}", cancelled_day2_id),
            payload_hash: format!("hash-{}", cancelled_day2_id),
            customer_id: customer_id.clone(),
            customer_name: "Alice Test".to_owned(),
            nationality: "THA".to_owned(),
            staff_id: "EMP-001".to_owned(),
            booking_no: "WALK-IN".to_owned(),
            notes: "cancelled day 2 tx".to_owned(),
            value_date: val_date_2,
            recorded_at: t_day2,
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
                quantity: ForeignAmount::from_decimal(Decimal::from(50)),
                unit_size: UnitSize::parse("1").unwrap(),
                rate_snapshot: Rate::parse("32.10").unwrap(),
                reference_rate: None,
                thb_amount: Money::parse("1605.00").unwrap(),
                fee_thb: Money::parse("0.00").unwrap(),
                settle_thb: Money::parse("1605.00").unwrap(),
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
                amount_thb: Money::parse("1605.00").unwrap(),
                bank_fee_thb: Money::parse("0.00").unwrap(),
                paid_at: t_day2,
            },
            receipts: vec![],
        };
        let mut sqlx_tx = pool.begin().await.unwrap();
        store.insert_draft_in(&mut sqlx_tx, &tx_day2).await.unwrap();
        store
            .set_status_in(
                &mut sqlx_tx,
                tx_day2.id,
                StatusMove {
                    from: TransactionStatus::Draft,
                    to: TransactionStatus::Cancelled,
                    reason: Some("Cancelled on day 2".to_owned()),
                    approval_request_id: None,
                },
                t_day2,
            )
            .await
            .unwrap();
        sqlx_tx.commit().await.unwrap();

        Harness {
            counter,
            ids: SeededIds {
                store_completed_ref,
                store_completed_id,
                draft_id,
                confirmed_id,
                cancelled_id,
                cancelled_day2_id,
                compat_ref: compat_saved.document_ref,
                compat_slip_url: format!("/files/{}", slip_key),
                customer_id,
                shift_id,
            },
        }
    }
}

/// 1. `admin_transactions`: with no status, gives COMPLETED only (never DRAFT, CONFIRMED, CANCELLED).
/// Asserts that store-completed row reads figures from child tables (not mutated wide row figures).
/// With `status=PENDING`, gives the unfinished set (DRAFT and CONFIRMED), never CANCELLED or COMPLETED.
#[tokio::test]
async fn admin_transactions_no_status_returns_completed_only_and_reads_child_figures() {
    let h = Harness::start().await;

    // (a) No status -> COMPLETED only (the 2 completed rows: store_completed and compat_saved).
    let rows = view(
        &h.counter,
        "/api/v3/views/admin_transactions?branch=MAIN&from=2026-10-06&to=2026-10-06",
    )
    .await;
    let list = rows.as_array().expect("array of rows");
    assert_eq!(
        list.len(),
        2,
        "no status must return exactly 2 COMPLETED rows, but got: {list:?}"
    );
    for row in list {
        assert_eq!(row["status"], "COMPLETED", "every row must be COMPLETED");
    }

    let store_row = list
        .iter()
        .find(|r| r["id"] == h.ids.store_completed_ref)
        .expect("must contain store completed row");

    // Figures MUST come from child tables, not the mutated wide row columns:
    assert_eq!(store_row["principal"], "3210.00");
    assert_eq!(store_row["fee"], "10.00");
    assert_eq!(store_row["settlement"], "3200.00");
    assert_eq!(store_row["bankFee"], "0.00");
    assert_eq!(store_row["method"], "Bank Transfer");
    assert_eq!(store_row["paymentRef"], "TRX-STORE-COMPLETED");
    assert_eq!(store_row["type"], "BUY");
    assert_eq!(store_row["currency"], "USD");
    assert_eq!(store_row["amount"], "100.00");
    assert_eq!(store_row["rate"], "32.10000000");
    assert_eq!(
        store_row["slipUrl"], "/files/SLIP-1",
        "slipUrl must come from payment slip_file_key, not transaction_files SLIP-MUTATED"
    );

    // (b) status=PENDING -> DRAFT and CONFIRMED only.
    let pending = view(
        &h.counter,
        "/api/v3/views/admin_transactions?branch=MAIN&status=PENDING&from=2026-10-06&to=2026-10-06",
    )
    .await;
    let pending_list = pending.as_array().expect("array of pending rows");
    assert_eq!(
        pending_list.len(),
        2,
        "status=PENDING must return exactly 2 unfinished rows (DRAFT and CONFIRMED), but got: {pending_list:?}"
    );
    let pending_statuses: Vec<&str> = pending_list
        .iter()
        .map(|r| r["status"].as_str().unwrap())
        .collect();
    assert!(
        pending_statuses.contains(&"DRAFT"),
        "must contain DRAFT status"
    );
    assert!(
        pending_statuses.contains(&"CONFIRMED"),
        "must contain CONFIRMED status"
    );
    assert!(
        !pending_statuses.contains(&"COMPLETED"),
        "must not contain COMPLETED"
    );
    assert!(
        !pending_statuses.contains(&"CANCELLED"),
        "must not contain CANCELLED"
    );
}

/// 2. `admin_transactions`: `type` and `q` filter through the line (not wide row columns).
#[tokio::test]
async fn admin_transactions_type_and_q_filter_through_line() {
    let h = Harness::start().await;

    // Line of store_completed is BUY, while its wide row type was mutated to SELL.
    let buy_rows = view(
        &h.counter,
        "/api/v3/views/admin_transactions?branch=MAIN&type=BUY&from=2026-10-06&to=2026-10-06",
    )
    .await;
    let buy_list = buy_rows.as_array().expect("array of buy rows");
    assert_eq!(
        buy_list.len(),
        1,
        "type=BUY must match store_completed through transaction_lines.shop_side"
    );
    assert_eq!(buy_list[0]["id"], h.ids.store_completed_ref);

    // Compat saved row has line shop_side = SELL.
    let sell_rows = view(
        &h.counter,
        "/api/v3/views/admin_transactions?branch=MAIN&type=SELL&from=2026-10-06&to=2026-10-06",
    )
    .await;
    let sell_list = sell_rows.as_array().expect("array of sell rows");
    assert_eq!(
        sell_list.len(),
        1,
        "type=SELL must match compat_saved through transaction_lines.shop_side"
    );
    assert_eq!(sell_list[0]["id"], h.ids.compat_ref);

    // Line currency is USD for both completed rows, while store_completed wide code was mutated to EUR.
    let usd_rows = view(
        &h.counter,
        "/api/v3/views/admin_transactions?branch=MAIN&q=usd&from=2026-10-06&to=2026-10-06",
    )
    .await;
    let usd_list = usd_rows.as_array().expect("array of usd rows");
    assert_eq!(
        usd_list.len(),
        2,
        "q=usd must match both completed rows through transaction_lines.currency"
    );

    // q=eur must return 0 completed rows (even though store_completed wide row code is EUR).
    let eur_rows = view(
        &h.counter,
        "/api/v3/views/admin_transactions?branch=MAIN&q=eur&from=2026-10-06&to=2026-10-06",
    )
    .await;
    assert_eq!(
        eur_rows.as_array().unwrap().len(),
        0,
        "q=eur must not match mutated wide row code"
    );
}

/// 3. `admin_transactions`: with `status=CANCELLED`, gives the cancelled row with its `cancel_reason`.
#[tokio::test]
async fn admin_transactions_cancelled_status_and_cancel_reason() {
    let h = Harness::start().await;

    let rows = view(
        &h.counter,
        "/api/v3/views/admin_transactions?branch=MAIN&status=CANCELLED&from=2026-10-06&to=2026-10-06",
    )
    .await;
    let list = rows.as_array().expect("array of cancelled rows");
    assert_eq!(
        list.len(),
        1,
        "status=CANCELLED must return exactly 1 row on 2026-10-06, but got: {list:?}"
    );
    let row = &list[0];
    assert_eq!(row["status"], "CANCELLED");
    assert_eq!(row["cancelReason"], "Customer changed mind");
    assert_eq!(
        row["id"],
        h.ids.cancelled_id.to_string(),
        "cancelled row id must be its transaction uuid"
    );
}

/// 4. `admin_transactions`: DRAFT row carries `status = "DRAFT"`, `id` = uuid, and paging cursor
/// accepts both kinds of id: document_ref (from completed row) and uuid (from unfinished row).
#[tokio::test]
async fn admin_transactions_draft_row_fields_and_paging_by_both_kinds_of_id() {
    let h = Harness::start().await;

    let pending = view(
        &h.counter,
        "/api/v3/views/admin_transactions?branch=MAIN&status=PENDING&from=2026-10-06&to=2026-10-06",
    )
    .await;
    let list = pending.as_array().expect("array of pending rows");
    assert_eq!(list.len(), 2);

    let draft_row = list
        .iter()
        .find(|r| r["status"] == "DRAFT")
        .expect("must find DRAFT row");
    assert_eq!(
        draft_row["id"],
        h.ids.draft_id.to_string(),
        "draft row id must be its uuid"
    );
    assert_eq!(draft_row["date"], "2026-10-06");
    assert!(
        draft_row["cancelReason"].is_null(),
        "draft cancelReason must be null"
    );

    // Keyset paging 1: `before` set to COMPLETED row's document_ref (compat_ref recorded at t4)
    // inside status=PENDING should return both unfinished rows (recorded at t2 and t1 < t4)
    let paged_by_doc_ref = view(
        &h.counter,
        &format!(
            "/api/v3/views/admin_transactions?branch=MAIN&status=PENDING&before={}",
            h.ids.compat_ref
        ),
    )
    .await;
    let paged_doc_list = paged_by_doc_ref.as_array().unwrap();
    assert_eq!(
        paged_doc_list.len(),
        2,
        "paging before completed document_ref must return both unfinished rows"
    );

    // Keyset paging 2: `before` set to CONFIRMED row's uuid (recorded at t2 > t1)
    // inside status=PENDING should return the older draft row (recorded at t1)
    let paged_by_uuid = view(
        &h.counter,
        &format!(
            "/api/v3/views/admin_transactions?branch=MAIN&status=PENDING&before={}",
            h.ids.confirmed_id
        ),
    )
    .await;
    let paged_uuid_list = paged_by_uuid.as_array().unwrap();
    assert_eq!(
        paged_uuid_list.len(),
        1,
        "paging before confirmed_id must return the older draft row"
    );
    assert_eq!(paged_uuid_list[0]["id"], h.ids.draft_id.to_string());

    // Keyset paging 3: `before` set to UNFINISHED row's uuid (confirmed_id at t2)
    // over COMPLETED rows (default status) should return store_completed (t0 < t2) and NOT compat_saved (t4 > t2)
    let paged_completed_by_uuid = view(
        &h.counter,
        &format!(
            "/api/v3/views/admin_transactions?branch=MAIN&before={}",
            h.ids.confirmed_id
        ),
    )
    .await;
    let paged_completed_list = paged_completed_by_uuid.as_array().unwrap();
    assert_eq!(
        paged_completed_list.len(),
        1,
        "paging completed rows before confirmed uuid must return only store_completed"
    );
    assert_eq!(paged_completed_list[0]["id"], h.ids.store_completed_ref);
}

/// 5. `admin_slips`: lists COMPLETED exchanges only whose payment has a `slip_file_key`.
/// A CONFIRMED exchange with a slip is NOT listed.
/// `q` matches by line currency and by payment reference.
#[tokio::test]
async fn admin_slips_completed_only_and_q_by_currency_and_payment_ref() {
    let h = Harness::start().await;

    let rows = view(
        &h.counter,
        "/api/v3/views/admin_slips?branch=MAIN&from=2026-10-06&to=2026-10-06",
    )
    .await;
    let list = rows.as_array().expect("array of slip rows");
    assert_eq!(
        list.len(),
        2,
        "admin_slips must list exactly 2 COMPLETED exchanges with slips, but got: {list:?}"
    );

    let store_slip = list
        .iter()
        .find(|r| r["id"] == h.ids.store_completed_ref)
        .expect("must contain store completed row");
    assert_eq!(
        store_slip["slipUrl"], "/files/SLIP-1",
        "slipUrl must come from payment's slip_file_key, not transaction_files SLIP-MUTATED"
    );

    // Filter q matches by payment reference
    let matched_ref = view(
        &h.counter,
        "/api/v3/views/admin_slips?branch=MAIN&q=TRX-STORE-COMPLETED&from=2026-10-06&to=2026-10-06",
    )
    .await;
    assert_eq!(
        matched_ref.as_array().unwrap().len(),
        1,
        "q must match payment reference"
    );

    // Filter q matches by line currency (USD)
    let matched_curr = view(
        &h.counter,
        "/api/v3/views/admin_slips?branch=MAIN&q=USD&from=2026-10-06&to=2026-10-06",
    )
    .await;
    assert_eq!(
        matched_curr.as_array().unwrap().len(),
        2,
        "q must match by line currency"
    );

    // Filter q=EUR must match 0 (even though store_completed wide code was mutated to EUR)
    let matched_eur = view(
        &h.counter,
        "/api/v3/views/admin_slips?branch=MAIN&q=EUR&from=2026-10-06&to=2026-10-06",
    )
    .await;
    assert_eq!(
        matched_eur.as_array().unwrap().len(),
        0,
        "q=EUR must return empty"
    );
}

/// 6. `admin_overview`: totals and recent list are over COMPLETED rows only.
/// `completed_count` = 2, `pending_count` = 2, `cancelled_count` = 1.
/// Totals equal the 2 completed rows' child figures (not mutated wide row figures).
#[tokio::test]
async fn admin_overview_completed_pending_cancelled_counts_and_totals() {
    let h = Harness::start().await;

    let overview = view(
        &h.counter,
        "/api/v3/views/admin_overview?branch=MAIN&from=2026-10-06&to=2026-10-06",
    )
    .await;
    let totals = &overview["totals"];

    assert_eq!(
        totals["completedCount"], 2,
        "completedCount must be 2 (store completed + compat completed)"
    );
    assert_eq!(
        totals["pendingCount"], 2,
        "pendingCount must be 2 (1 DRAFT + 1 CONFIRMED)"
    );
    assert_eq!(
        totals["cancelledCount"], 1,
        "cancelledCount must be 1 (1 CANCELLED on 2026-10-06)"
    );

    // Sum over completed rows only from child lines:
    // Row 1 (BUY 100 USD @ 32.10, fee 10.00, settle 3200.00): principal 3210.00, paid 3200.00
    // Row 5 (SELL 100 USD @ 32.60, fee 0.00, settle 3260.00): principal 3260.00, received 3260.00
    // Total volume: 3210 + 3260 = 6470.00
    assert_eq!(totals["volume"], "6470.00");
    assert_eq!(totals["buyPrincipal"], "3210.00");
    assert_eq!(totals["sellPrincipal"], "3260.00");
    assert_eq!(totals["paid"], "3200.00");
    assert_eq!(totals["received"], "3260.00");
}

/// 7. `admin_overview`: a day that has ONLY a CANCELLED row returns zero totals and `cancelled_count = 1`.
#[tokio::test]
async fn admin_overview_day_with_only_cancelled_row_totals_zero() {
    let h = Harness::start().await;

    let overview = view(
        &h.counter,
        "/api/v3/views/admin_overview?branch=MAIN&from=2026-10-07&to=2026-10-07",
    )
    .await;
    let totals = &overview["totals"];

    assert_eq!(totals["cancelledCount"], 1);
    assert_eq!(totals["completedCount"], 0);
    assert_eq!(totals["pendingCount"], 0);

    assert_eq!(totals["volume"], "0.00");
    assert_eq!(totals["buyPrincipal"], "0.00");
    assert_eq!(totals["sellPrincipal"], "0.00");
    assert_eq!(totals["paid"], "0.00");
    assert_eq!(totals["received"], "0.00");
    assert_eq!(totals["net"], "0.00");
    assert_eq!(totals["fees"], "0.00");
    assert_eq!(totals["buyShare"], "0");
    assert_eq!(totals["sellShare"], "0");
    assert_eq!(totals["buyCount"], 0);
    assert_eq!(totals["sellCount"], 0);
}

/// 8. `admin_customer_history`: lists COMPLETED exchanges of the customer only (never DRAFT, CONFIRMED, CANCELLED).
/// Figures come from child rows (not mutated wide row figures).
/// Another customer's completed row is not listed.
#[tokio::test]
async fn admin_customer_history_completed_only_and_reads_child_figures() {
    let h = Harness::start().await;

    let history = view(
        &h.counter,
        &format!(
            "/api/v3/views/admin_customer_history?branch=MAIN&customer={}",
            h.ids.customer_id
        ),
    )
    .await;
    let list = history.as_array().expect("array of customer history rows");
    assert_eq!(
        list.len(),
        2,
        "customer history must list completed rows only (never draft, confirmed, cancelled), but got: {list:?}"
    );
    for row in list {
        assert_eq!(row["status"], "COMPLETED");
    }

    let store_row = list
        .iter()
        .find(|r| r["id"] == h.ids.store_completed_ref)
        .expect("must contain store completed row");
    assert_eq!(store_row["principal"], "3210.00");
    assert_eq!(store_row["fee"], "10.00");
    assert_eq!(store_row["settlement"], "3200.00");
    assert_eq!(store_row["method"], "Bank Transfer");
    assert_eq!(store_row["paymentRef"], "TRX-STORE-COMPLETED");
    assert_eq!(store_row["type"], "BUY");
    assert_eq!(store_row["currency"], "USD");

    let other_history = view(
        &h.counter,
        "/api/v3/views/admin_customer_history?branch=MAIN&customer=CU-OTHER",
    )
    .await;
    assert_eq!(
        other_history.as_array().unwrap().len(),
        0,
        "other customer history must be empty"
    );
}

/// 9. Compat-saved row's JSON equals today's ("byte-identical") literal snapshot across
/// `admin_transactions`, `admin_slips`, and `admin_customer_history`.
#[tokio::test]
async fn compat_saved_completed_row_json_matches_snapshot() {
    let h = Harness::start().await;

    let expected = serde_json::json!({
        "id": h.ids.compat_ref,
        "date": "2026-10-06",
        "created": "2026-10-06T05:04:00.000Z",
        "type": "SELL",
        "currency": "USD",
        "amount": "100.00",
        "rate": "32.60000000",
        "unitSize": "1",
        "principal": "3260.00",
        "fee": "0.00",
        "bankFee": "0.00",
        "settlement": "3260.00",
        "method": "Cash",
        "customer": "Alice Test",
        "customerId": h.ids.customer_id,
        "staff": "EMP-001",
        "status": "COMPLETED",
        "notes": "compat saved tx",
        "cancelReason": null,
        "shiftId": h.ids.shift_id,
        "referenceRate": null,
        "referenceValue": null,
        "referenceSpread": null,
        "referencePercent": null,
        "paymentRef": null,
        "slipUrl": h.ids.compat_slip_url
    });

    // (a) in admin_transactions
    let tx_rows = view(
        &h.counter,
        "/api/v3/views/admin_transactions?branch=MAIN&from=2026-10-06&to=2026-10-06",
    )
    .await;
    let tx_compat = tx_rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == h.ids.compat_ref)
        .expect("must find compat row in admin_transactions");
    assert_eq!(
        *tx_compat, expected,
        "admin_transactions compat row must match snapshot"
    );

    // (b) in admin_slips
    let slip_rows = view(
        &h.counter,
        "/api/v3/views/admin_slips?branch=MAIN&from=2026-10-06&to=2026-10-06",
    )
    .await;
    let slip_compat = slip_rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == h.ids.compat_ref)
        .expect("must find compat row in admin_slips");
    assert_eq!(
        *slip_compat, expected,
        "admin_slips compat row must match snapshot"
    );

    // (c) in admin_customer_history
    let history_rows = view(
        &h.counter,
        &format!(
            "/api/v3/views/admin_customer_history?branch=MAIN&customer={}",
            h.ids.customer_id
        ),
    )
    .await;
    let history_compat = history_rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == h.ids.compat_ref)
        .expect("must find compat row in admin_customer_history");
    assert_eq!(
        *history_compat, expected,
        "admin_customer_history compat row must match snapshot"
    );
}
