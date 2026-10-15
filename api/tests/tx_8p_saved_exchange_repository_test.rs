//! Ticket #1154 (TX-8p) — `PgTransactionRepository` against a real PostgreSQL:
//! reads figures from `transaction_lines` and `transaction_payments` rather than
//! the wide columns of `transactions`, and answers `COMPLETED` rows only.
//! Written before the implementation (rule A, ticket #1154).
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §3.2–§3.4, §4.1, §9 row TX-8;
//! ticket #1154 §1–§10.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod save_common;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use save_common::*;
use uuid::Uuid;
use xc_exchange::domain::exchange::entities::transaction::Transaction;
use xc_exchange::domain::exchange::repositories::transaction_repository::TransactionRepository;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::repository::{StatusMove, TransactionStore};
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};

struct SeededExchanges {
    branch: BranchId,
    compat_doc_ref: String,
    compat_sub_id: String,
    completed_doc_ref: String,
    completed_sub_id: String,
    draft_sub_id: String,
    confirmed_sub_id: String,
    cancelled_sub_id: String,
    slip_key: String,
}

fn make_draft(
    org_id: Uuid,
    branch_id: &BranchId,
    shift_id: &str,
    customer_id: &str,
    passport_key: &str,
    slip_key: Option<&str>,
    submission_id: &str,
) -> ExchangeTransaction {
    let id = TransactionId(Uuid::new_v4());
    ExchangeTransaction {
        id,
        organization_id: org_id,
        branch_id: branch_id.clone(),
        mode: Mode::Live,
        status: TransactionStatus::Draft,
        document_ref: None,
        quote_id: None,
        booking_id: None,
        shift_id: Some(shift_id.to_owned()),
        submission_id: submission_id.to_owned(),
        payload_hash: format!("hash-{}", id.0),
        customer_id: customer_id.to_owned(),
        customer_name: "Somchai".to_owned(),
        nationality: "THA".to_owned(),
        staff_id: "EMP-001".to_owned(),
        booking_no: "WALK-IN".to_owned(),
        notes: "a note".to_owned(),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
        recorded_at: at(0),
        recorded_by: "owner".to_owned(),
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
            method: PaymentMethod::BankTransfer,
            reference: "TRX-9".to_owned(),
            account: "KBank 1".to_owned(),
            slip_file_key: slip_key.map(str::to_owned),
            amount_thb: Money::parse("3200.00").unwrap(),
            bank_fee_thb: Money::parse("1.50").unwrap(),
            paid_at: at(0),
        },
        receipts: vec![],
    }
}

async fn seed_five_exchanges(shop: &Shop) -> SeededExchanges {
    let branch = BranchId::new(MAIN);
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let slip_key = shop.upload(MAIN, OWNER, "application/pdf", PDF).await;
    let receipt_key = shop.upload(MAIN, OWNER, "application/pdf", PDF).await;

    // 1. One compat-saved exchange (wide and child figures are equal)
    let compat_sub_id = format!("sub-compat-{}", Uuid::new_v4());
    let compat = shop
        .save(MAIN, form(&compat_sub_id, &customer, &passport), at(10))
        .await
        .expect("compat save");

    let mut tx = shop.pool.begin().await.unwrap();
    let org_id = PgTransactionRepository::organization_of_in(&mut tx, &branch)
        .await
        .expect("organization_of_in");
    tx.commit().await.unwrap();

    let shift_id: String =
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = $1 AND status = 'OPEN'")
            .bind(branch.as_str())
            .fetch_one(&shop.pool)
            .await
            .expect("open shift");

    let store = PgTransactionStore::new(shop.pool.clone());

    // 2. One COMPLETED exchange through TX-2 store
    let completed_sub_id = format!("sub-compl-{}", Uuid::new_v4());
    let completed_tx = make_draft(
        org_id,
        &branch,
        &shift_id,
        &customer,
        &passport,
        Some(&slip_key),
        &completed_sub_id,
    );
    let completed_id = completed_tx.id;

    let mut tx = shop.pool.begin().await.unwrap();
    store
        .insert_draft_in(&mut tx, &completed_tx)
        .await
        .expect("insert completed draft");
    store
        .set_status_in(
            &mut tx,
            completed_id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Confirmed,
                reason: None,
                approval_request_id: None,
            },
            at(20),
        )
        .await
        .expect("confirm draft");
    let (completed_doc_ref, _) = store
        .complete_in(&mut tx, completed_id, &receipt_key, at(30))
        .await
        .expect("complete draft");
    tx.commit().await.unwrap();

    sqlx::query(
        "INSERT INTO transaction_files (transaction_id, kind, file_key) VALUES ($1, 'RECEIPT', $2)",
    )
    .bind(completed_id.0)
    .bind(&receipt_key)
    .execute(&shop.pool)
    .await
    .expect("insert receipt in transaction_files");

    // Mutate the wide row figures of the COMPLETED exchange to different values (§9)
    sqlx::query(
        "UPDATE transactions \
         SET type = 'SELL', code = 'EUR', amount = 9999.00, rate = 99.000000, \
             reference_rate = 98.000000, principal = 99999.00, fee = 888.00, \
             bank_fee = 777.00, received = 66666.00, paid = 55555.00, \
             method = 'PromptPay', payment_ref = 'MUTATED_REF', account = 'MUTATED_ACC' \
         WHERE id = $1",
    )
    .bind(completed_id.0)
    .execute(&shop.pool)
    .await
    .expect("mutate wide columns");

    let mutated_slip = shop.upload(MAIN, OWNER, "application/pdf", PDF).await;
    // A stale SLIP row (legacy data): inserted if the writer no longer writes one (TX-9 #677), so it exists
    // before and after the writers stop, and the reader must still answer the payment's slip.
    sqlx::query(
        "INSERT INTO transaction_files (transaction_id, kind, file_key) VALUES ($1, 'SLIP', $2) \
         ON CONFLICT (transaction_id, kind) DO UPDATE SET file_key = EXCLUDED.file_key",
    )
    .bind(completed_id.0)
    .bind(&mutated_slip)
    .execute(&shop.pool)
    .await
    .expect("mutate slip in transaction_files");

    // 3. One DRAFT exchange through TX-2 store (document_ref is NULL)
    let draft_sub_id = format!("sub-draft-{}", Uuid::new_v4());
    let draft_tx = make_draft(
        org_id,
        &branch,
        &shift_id,
        &customer,
        &passport,
        None,
        &draft_sub_id,
    );
    let mut tx = shop.pool.begin().await.unwrap();
    store
        .insert_draft_in(&mut tx, &draft_tx)
        .await
        .expect("insert draft");
    tx.commit().await.unwrap();

    // 4. One CONFIRMED exchange through TX-2 store (document_ref is NULL)
    let confirmed_sub_id = format!("sub-conf-{}", Uuid::new_v4());
    let confirmed_tx = make_draft(
        org_id,
        &branch,
        &shift_id,
        &customer,
        &passport,
        None,
        &confirmed_sub_id,
    );
    let mut tx = shop.pool.begin().await.unwrap();
    store
        .insert_draft_in(&mut tx, &confirmed_tx)
        .await
        .expect("insert confirmed draft");
    store
        .set_status_in(
            &mut tx,
            confirmed_tx.id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Confirmed,
                reason: None,
                approval_request_id: None,
            },
            at(40),
        )
        .await
        .expect("confirm draft");
    tx.commit().await.unwrap();

    // 5. One CANCELLED exchange through TX-2 store (document_ref is NULL)
    let cancelled_sub_id = format!("sub-canc-{}", Uuid::new_v4());
    let cancelled_tx = make_draft(
        org_id,
        &branch,
        &shift_id,
        &customer,
        &passport,
        None,
        &cancelled_sub_id,
    );
    let cancelled_id = cancelled_tx.id;
    let mut tx = shop.pool.begin().await.unwrap();
    store
        .insert_draft_in(&mut tx, &cancelled_tx)
        .await
        .expect("insert cancelled draft");
    store
        .set_status_in(
            &mut tx,
            cancelled_id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Cancelled,
                reason: Some("customer cancelled".to_owned()),
                approval_request_id: None,
            },
            at(50),
        )
        .await
        .expect("cancel draft");
    tx.commit().await.unwrap();

    SeededExchanges {
        branch,
        compat_doc_ref: compat.document_ref,
        compat_sub_id,
        completed_doc_ref,
        completed_sub_id,
        draft_sub_id,
        confirmed_sub_id,
        cancelled_sub_id,
        slip_key,
    }
}

fn assert_completed_figures_match_children(tx: &Transaction, expected_slip: &str) {
    assert_eq!(tx.side, Side::Buy, "side must come from line shop_side");
    assert_eq!(tx.currency, "USD", "currency must come from line currency");
    assert_eq!(
        tx.amount,
        Money::parse("100.00").unwrap(),
        "amount must come from line quantity"
    );
    assert_eq!(
        tx.rate,
        Rate::parse("32.1").unwrap(),
        "rate must come from line rate_snapshot"
    );
    assert_eq!(
        tx.reference_rate,
        Some(Rate::parse("32.05").unwrap()),
        "reference_rate must come from line reference_rate"
    );
    assert_eq!(
        tx.principal,
        Money::parse("3210.00").unwrap(),
        "principal must come from line thb_amount"
    );
    assert_eq!(
        tx.fee,
        Money::parse("10.00").unwrap(),
        "fee must come from line fee_thb"
    );
    assert_eq!(
        tx.bank_fee,
        Money::parse("1.50").unwrap(),
        "bank_fee must come from payment bank_fee_thb"
    );
    assert_eq!(
        tx.received,
        Money::parse("0.00").unwrap(),
        "received must be 0.00 for BUY"
    );
    assert_eq!(
        tx.paid,
        Money::parse("3200.00").unwrap(),
        "paid must come from line settle_thb for BUY"
    );
    assert_eq!(
        tx.method,
        PaymentMethod::BankTransfer,
        "method must come from payment method"
    );
    assert_eq!(
        tx.payment_ref, "TRX-9",
        "payment_ref must come from payment reference"
    );
    assert_eq!(
        tx.account, "KBank 1",
        "account must come from payment account"
    );
    assert_eq!(
        tx.slip_key.as_deref(),
        Some(expected_slip),
        "slip_key must come from payment slip_file_key"
    );
}

#[tokio::test]
async fn list_answers_only_completed_and_compat_save_with_child_figures() {
    let shop = Shop::start().await;
    let seeded = seed_five_exchanges(&shop).await;

    let repo = PgTransactionRepository::new(shop.pool.clone());
    let list = repo
        .list(&seeded.branch, Mode::Live)
        .await
        .expect("list must succeed and not fail on NULL document_ref rows");

    assert_eq!(
        list.len(),
        2,
        "list must answer exactly the COMPLETED exchange and the compat save"
    );

    let completed_tx = list
        .iter()
        .find(|t| t.document_ref == seeded.completed_doc_ref)
        .expect("COMPLETED exchange in list");
    assert_completed_figures_match_children(completed_tx, &seeded.slip_key);

    let compat_tx = list
        .iter()
        .find(|t| t.document_ref == seeded.compat_doc_ref)
        .expect("compat-saved exchange in list");
    assert_eq!(compat_tx.side, Side::Buy);
    assert_eq!(compat_tx.currency, "USD");
    assert_eq!(compat_tx.amount, Money::parse("100.00").unwrap());
    assert_eq!(compat_tx.rate, Rate::parse("32.1").unwrap());
    assert_eq!(compat_tx.principal, Money::parse("3210.00").unwrap());
    assert_eq!(compat_tx.paid, Money::parse("3210.00").unwrap());
    assert_eq!(compat_tx.method, PaymentMethod::Cash);
    assert_eq!(compat_tx.slip_key, None);

    // Verify DRAFT, CONFIRMED and CANCELLED are never answered
    assert!(
        !list.iter().any(|t| t.submission_id == seeded.draft_sub_id),
        "DRAFT must not be listed"
    );
    assert!(
        !list
            .iter()
            .any(|t| t.submission_id == seeded.confirmed_sub_id),
        "CONFIRMED must not be listed"
    );
    assert!(
        !list
            .iter()
            .any(|t| t.submission_id == seeded.cancelled_sub_id),
        "CANCELLED must not be listed"
    );

    shop.finish().await;
}

#[tokio::test]
async fn find_by_number_answers_only_completed_with_child_figures() {
    let shop = Shop::start().await;
    let seeded = seed_five_exchanges(&shop).await;

    let repo = PgTransactionRepository::new(shop.pool.clone());

    // 1. Finding the COMPLETED exchange answers child figures
    let found = repo
        .find(&seeded.branch, &seeded.completed_doc_ref)
        .await
        .expect("find query succeeds")
        .expect("COMPLETED exchange found by document_ref");
    assert_completed_figures_match_children(&found, &seeded.slip_key);

    // 2. Finding the compat save answers wide = children
    let compat_found = repo
        .find(&seeded.branch, &seeded.compat_doc_ref)
        .await
        .expect("find query succeeds")
        .expect("compat exchange found by document_ref");
    assert_eq!(compat_found.document_ref, seeded.compat_doc_ref);
    assert_eq!(compat_found.currency, "USD");
    assert_eq!(compat_found.amount, Money::parse("100.00").unwrap());

    // 3. Finding an unknown number answers None
    let unknown = repo
        .find(&seeded.branch, "XC-NONEXISTENT-999999")
        .await
        .expect("find query succeeds");
    assert_eq!(unknown, None, "unknown document_ref answers None");

    shop.finish().await;
}

#[tokio::test]
async fn find_submission_in_answers_only_completed_and_reads_from_children() {
    let shop = Shop::start().await;
    let seeded = seed_five_exchanges(&shop).await;

    let repo = PgTransactionRepository::new(shop.pool.clone());

    // 1. DRAFT, CONFIRMED and CANCELLED submissions must answer None
    let mut tx = shop.pool.begin().await.unwrap();
    let draft_res = repo
        .find_submission_in(&mut tx, &seeded.branch, Mode::Live, &seeded.draft_sub_id)
        .await
        .expect("find_submission_in query succeeds");
    assert!(draft_res.is_none(), "DRAFT submission must answer None");

    let confirmed_res = repo
        .find_submission_in(
            &mut tx,
            &seeded.branch,
            Mode::Live,
            &seeded.confirmed_sub_id,
        )
        .await
        .expect("find_submission_in query succeeds");
    assert!(
        confirmed_res.is_none(),
        "CONFIRMED submission must answer None"
    );

    let cancelled_res = repo
        .find_submission_in(
            &mut tx,
            &seeded.branch,
            Mode::Live,
            &seeded.cancelled_sub_id,
        )
        .await
        .expect("find_submission_in query succeeds");
    assert!(
        cancelled_res.is_none(),
        "CANCELLED submission must answer None"
    );

    // 2. COMPLETED submission answers child figures
    let (completed_found, _) = repo
        .find_submission_in(
            &mut tx,
            &seeded.branch,
            Mode::Live,
            &seeded.completed_sub_id,
        )
        .await
        .expect("find_submission_in query succeeds")
        .expect("COMPLETED submission found");
    assert_completed_figures_match_children(&completed_found, &seeded.slip_key);

    // 3. Compat save answers as today
    let (compat_found, _) = repo
        .find_submission_in(&mut tx, &seeded.branch, Mode::Live, &seeded.compat_sub_id)
        .await
        .expect("find_submission_in query succeeds")
        .expect("compat submission found");
    assert_eq!(compat_found.document_ref, seeded.compat_doc_ref);
    assert_eq!(compat_found.currency, "USD");
    assert_eq!(compat_found.amount, Money::parse("100.00").unwrap());

    tx.commit().await.unwrap();
    shop.finish().await;
}
