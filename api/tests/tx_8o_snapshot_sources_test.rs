//! Ticket #1139 (TX-8o) — `ServiceSnapshotSources::transactions`: read completed exchanges
//! only, and read `unit_size` from `transaction_lines`.
//! Written before the implementation by the test-writer (gemini/warut-8, rule A).
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §3.2–§3.4, §4.1 (states), §9 TX-8.
//! Ticket §9:
//! - The four seeded rows: one entry, the completed one's, with its line's unit size.
//! - The compat-saved exchange gives today's map.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod compat_common;
mod save_common;
mod snapshot_common;

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use save_common::{
    MAIN, Mode, OWNER, PDF, Shop, TransactionService, at, branch, form, millis, money,
};
use uuid::Uuid;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::repository::{StatusMove, TransactionStore};
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Rate, Side, UnitSize};
use xchange::application::compat::snapshot_sources::{SnapshotSources, SnapshotTransaction};
use xchange::infrastructure::compat::snapshot_sources::ServiceSnapshotSources;

fn branch_tx(shop: &Shop) -> BranchTx {
    BranchTx::new(pos_infra::transactions::Transactor::new(shop.pool.clone()))
}

fn sources(shop: &Shop) -> ServiceSnapshotSources {
    ServiceSnapshotSources::new(TransactionService::new(
        Arc::new(PgTransactionRepository::new(shop.pool.clone())),
        branch_tx(shop),
    ))
}

#[allow(clippy::too_many_arguments)]
fn make_draft(
    org_id: Uuid,
    branch_name: &str,
    shift_id: Option<String>,
    customer_id: &str,
    passport_key: &str,
    currency: &str,
    unit_size: &str,
    amount: &str,
    rate: &str,
    ref_rate: Option<&str>,
    fee: &str,
    bank_fee: &str,
    paid_thb: &str,
    recorded_at: DateTime<Utc>,
) -> ExchangeTransaction {
    let id = TransactionId(Uuid::new_v4());
    ExchangeTransaction {
        id,
        organization_id: org_id,
        branch_id: BranchId::new(branch_name.to_owned()),
        mode: Mode::Live,
        status: TransactionStatus::Draft,
        document_ref: None,
        quote_id: None,
        booking_id: None,
        shift_id,
        submission_id: format!("sub-{}", id.0),
        payload_hash: "hash".to_owned(),
        customer_id: customer_id.to_owned(),
        customer_name: "Somchai Jaidee".to_owned(),
        nationality: "THA".to_owned(),
        staff_id: OWNER.to_owned(),
        booking_no: "WALK-IN".to_owned(),
        notes: "seeded".to_owned(),
        value_date: recorded_at.date_naive(),
        recorded_at,
        recorded_by: OWNER.to_owned(),
        completed_at: None,
        cancelled_at: None,
        cancel_reason: None,
        approval_request_id: None,
        passport_file_key: passport_key.to_owned(),
        kiosk_request_id: None,
        staff_draft_id: None,
        count_confirmed: false,
        lines: vec![TransactionLine {
            id: Uuid::new_v4(),
            quote_line_id: None,
            shop_side: Side::Buy,
            currency: CurrencyCode::parse(currency).unwrap(),
            quantity: ForeignAmount::from_decimal(Decimal::from_str(amount).unwrap()),
            unit_size: UnitSize::parse(unit_size).unwrap(),
            rate_snapshot: Rate::parse(rate).unwrap(),
            reference_rate: ref_rate.map(|r| Rate::parse(r).unwrap()),
            thb_amount: money(paid_thb),
            fee_thb: money(fee),
            settle_thb: money(paid_thb),
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
            amount_thb: money(paid_thb),
            bank_fee_thb: money(bank_fee),
            paid_at: recorded_at,
        },
        receipts: vec![],
    }
}

/// §9 edge case 1: Seeding four rows through TX-2's store (DRAFT, CONFIRMED, CANCELLED,
/// and COMPLETED). `ServiceSnapshotSources::transactions` must answer exactly one entry
/// (the COMPLETED exchange), with its `unit_size` read from `transaction_lines` (not the
/// wide `transactions` row).
#[tokio::test]
async fn the_four_seeded_rows_answer_one_completed_entry_with_its_lines_unit_size() {
    let shop = Shop::start().await;
    let (customer, passport) = shop
        .ready(&[("THB", "100000"), ("USD", "1000"), ("JPY", "1000000")])
        .await;
    let shift = shop.open_shift_id(MAIN).await;
    let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
        .bind(MAIN)
        .fetch_one(&shop.pool)
        .await
        .expect("branch organization_id");

    let store = PgTransactionStore::new(shop.pool.clone());

    // 1. DRAFT
    let draft = make_draft(
        org_id,
        MAIN,
        Some(shift.clone()),
        &customer,
        &passport,
        "USD",
        "1",
        "100",
        "32.1",
        None,
        "0",
        "0",
        "3210.00",
        at(10),
    );
    let mut tx = shop.pool.begin().await.unwrap();
    store
        .insert_draft_in(&mut tx, &draft)
        .await
        .expect("insert draft");
    tx.commit().await.unwrap();

    // 2. CONFIRMED
    let confirmed = make_draft(
        org_id,
        MAIN,
        Some(shift.clone()),
        &customer,
        &passport,
        "USD",
        "1",
        "200",
        "32.1",
        None,
        "0",
        "0",
        "6420.00",
        at(20),
    );
    let mut tx = shop.pool.begin().await.unwrap();
    store
        .insert_draft_in(&mut tx, &confirmed)
        .await
        .expect("insert confirmed");
    store
        .set_status_in(
            &mut tx,
            confirmed.id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Confirmed,
                reason: None,
                approval_request_id: None,
            },
            at(20),
        )
        .await
        .expect("confirm exchange");
    tx.commit().await.unwrap();

    // 3. CANCELLED
    let cancelled = make_draft(
        org_id,
        MAIN,
        Some(shift.clone()),
        &customer,
        &passport,
        "USD",
        "1",
        "300",
        "32.1",
        None,
        "0",
        "0",
        "9630.00",
        at(30),
    );
    let mut tx = shop.pool.begin().await.unwrap();
    store
        .insert_draft_in(&mut tx, &cancelled)
        .await
        .expect("insert cancelled");
    store
        .set_status_in(
            &mut tx,
            cancelled.id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Cancelled,
                reason: Some("customer cancelled".to_owned()),
                approval_request_id: None,
            },
            at(30),
        )
        .await
        .expect("cancel exchange");
    tx.commit().await.unwrap();

    // 4. COMPLETED — JPY with unit_size 100 on the line
    let completed = make_draft(
        org_id,
        MAIN,
        Some(shift.clone()),
        &customer,
        &passport,
        "JPY",
        "100",
        "10000",
        "23.5",
        Some("23.4"),
        "0",
        "0",
        "2350.00",
        at(40),
    );
    let receipt = shop.upload(MAIN, OWNER, "application/pdf", PDF).await;
    let mut tx = shop.pool.begin().await.unwrap();
    store
        .insert_draft_in(&mut tx, &completed)
        .await
        .expect("insert completed draft");
    store
        .set_status_in(
            &mut tx,
            completed.id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Confirmed,
                reason: None,
                approval_request_id: None,
            },
            at(40),
        )
        .await
        .expect("confirm completed draft");
    let (completed_ref, _) = store
        .complete_in(&mut tx, completed.id, &receipt, at(40))
        .await
        .expect("complete exchange");
    tx.commit().await.unwrap();

    // TX-9 (#677) dropped `transactions.unit_size`: the reader can only read the line's 100.

    let sources = sources(&shop);
    let listed = sources
        .transactions(&branch(MAIN))
        .await
        .expect("the exchanges of MAIN");

    assert_eq!(
        listed.len(),
        1,
        "only COMPLETED exchanges are listed; DRAFT, CONFIRMED and CANCELLED must be excluded"
    );
    assert_eq!(listed[0].document_ref, completed_ref);
    assert_eq!(
        listed[0].unit_size,
        UnitSize::parse("100").unwrap(),
        "unit_size must be read from transaction_lines (100), not wide transactions (1)"
    );
    assert_eq!(listed[0].currency, "JPY");
    assert_eq!(listed[0].amount, money("10000"));
    assert_eq!(listed[0].rate, Rate::parse("23.5").unwrap());
    assert_eq!(listed[0].reference_rate, Some(Rate::parse("23.4").unwrap()));
}

/// §9 edge case 2: A compat-saved exchange (via `SaveTransactionWorkflow`) alongside
/// unfinished exchanges. The reader must ignore unfinished rows and answer today's map
/// with the compat-saved exchange's figures.
#[tokio::test]
async fn the_compat_saved_exchange_gives_todays_map() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let slip = shop.upload(MAIN, OWNER, "application/pdf", PDF).await;
    let shift = shop.open_shift_id(MAIN).await;

    let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
        .bind(MAIN)
        .fetch_one(&shop.pool)
        .await
        .expect("branch organization_id");

    // Seed an unfinished (DRAFT) exchange in the same branch
    let store = PgTransactionStore::new(shop.pool.clone());
    let draft = make_draft(
        org_id,
        MAIN,
        Some(shift.clone()),
        &customer,
        &passport,
        "USD",
        "1",
        "100",
        "32.1",
        None,
        "0",
        "0",
        "3210.00",
        at(10),
    );
    let mut tx = shop.pool.begin().await.unwrap();
    store
        .insert_draft_in(&mut tx, &draft)
        .await
        .expect("insert draft");
    tx.commit().await.unwrap();

    // The compat save: a SELL by transfer with a slip
    let mut cmd = form("compat-0001-aaaaaaaaaa", &customer, &passport);
    cmd.side = "SELL".to_owned();
    cmd.rate = "33".to_owned();
    cmd.amount = "250.5".to_owned();
    cmd.fee = "10".to_owned();
    cmd.bank_fee = "1.5".to_owned();
    cmd.method = "SCB Transfer".to_owned();
    cmd.payment_ref = "TRX-1".to_owned();
    cmd.slip = Some(slip.clone());
    cmd.staff = "owner".to_owned();
    cmd.reference_rate = Some("32.95".to_owned());
    cmd.value_date = "2026-11-01".to_owned();
    let saved = shop.save(MAIN, cmd, at(20)).await.expect("the compat save");

    let sources = sources(&shop);
    let listed = sources
        .transactions(&branch(MAIN))
        .await
        .expect("the exchanges of MAIN");

    assert_eq!(
        listed,
        vec![SnapshotTransaction {
            document_ref: saved.document_ref.clone(),
            recorded_at: millis(at(20)),
            value_date: NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
            side: Side::Sell,
            currency: "USD".to_owned(),
            amount: money("250.5"),
            rate: Rate::parse("33").unwrap(),
            unit_size: UnitSize::parse("1").unwrap(),
            reference_rate: Some(Rate::parse("32.95").unwrap()),
            fee: money("10"),
            bank_fee: money("1.5"),
            method: "SCB Transfer".to_owned(),
            slip_key: Some(slip.clone()),
            payment_ref: "TRX-1".to_owned(),
            customer_id: customer.clone(),
            customer_name: "Somchai Jaidee".to_owned(),
            staff: "owner".to_owned(),
            notes: String::new(),
            shift_id: Some(shift),
        }]
    );
}
