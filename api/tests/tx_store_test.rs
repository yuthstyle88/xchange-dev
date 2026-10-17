//! Ticket #670 (TX-2) — `PgTransactionStore` against a real PostgreSQL: a DRAFT with its line and
//! payment, the guarded status moves, and the completion that numbers the exchange per organisation
//! and issues receipt revision 1.
//! Written before the implementation by the ticket author (claude/f03345fe). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §2 D-2 (the wide columns are still written,
//! from the line and the payment), D-3 (`document_ref` is NULL until COMPLETED, assigned at
//! completion; a cancelled draft burns no number), D-4 (the counter is per organisation), D-9 (one
//! payment now), §3.2–§3.5, §4.1 (the states), §7 (`ExchangeTransaction`, `TransactionLine`,
//! `TransactionPayment`, `Receipt`, the store port, `PgTransactionStore`; every `_in` takes the
//! caller's `&mut sqlx::Transaction<'_, Postgres>` first). Lane controller's ruling on #670
//! (2026-10-10): the counter key is `document-<organization_id>-<LIVE|DEMO>-<YYYYMMDD>`.
//! Lane controller's rulings on #672 (2026-10-10, claude/5edf7bb5, issuecomment-6094343896), which
//! make the aggregate a whole record of what create receives:
//! - Q2: `ExchangeTransaction.passport_file_key`; the store writes the PASSPORT `transaction_files`
//!   row at insert and reads it back, as it does SLIP (one writer, D-1);
//! - Q3: `kiosk_request_id`, `staff_draft_id` and `count_confirmed` are stored on `transactions` at
//!   insert and read back, so confirm and complete rebuild `SaveExtras` from the stored row (D-7).
//!
//! Customers are seeded in CI-4's shape (#651: a customer belongs to the organisation).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips. Every store call runs in a transaction the test opens and commits; a refused call is
//! followed by a COMMIT too, and the tables are counted, so "nothing written" is measured, not
//! assumed. The exchanges are DEMO (no shift needed); `now` is a whole second.
//!
//! Signatures this test assumes — [required] in ticket §7 (they cross tickets: TX-4, TX-5, TX-6 and
//! TX-10 call them):
//! - `xc_exchange::domain::transaction::transaction::{ExchangeTransaction, TransactionId,
//!   TransactionStatus}`:
//!   - `TransactionId(pub Uuid)`: `Copy`, `Eq`;
//!   - `TransactionStatus { Draft, PendingApproval, Confirmed, Completed, Cancelled }`, `Copy`, `Eq`;
//!   - `ExchangeTransaction`: `Clone`, `PartialEq`, every field below public, including
//!     `passport_file_key: String`, `kiosk_request_id: Option<String>`,
//!     `staff_draft_id: Option<String>` and `count_confirmed: bool` (#672 Q2/Q3).
//! - `…::transaction::line::TransactionLine`, `…::transaction::payment::TransactionPayment`,
//!   `…::transaction::receipt_record::Receipt`: `Clone`, `PartialEq`, fields below public.
//! - `…::transaction::repository::{TransactionStore, StatusMove}` — the port (the design's working
//!   name `TransactionRepository2`; named here). `StatusMove { from, to, reason: Option<String>,
//!   approval_request_id: Option<Uuid> }`.
//! - `xc_exchange::infrastructure::transaction::store::PgTransactionStore::new(pool: PgPool)`.
//! - `TransactionStore` methods (async, `AppResult`):
//!   - `insert_draft_in(tx, &ExchangeTransaction) -> ()`: only a `Draft` with no number, no
//!     receipt and exactly one line; else `validationFailed`, nothing written;
//!   - `find_in(tx, TransactionId) -> Option<ExchangeTransaction>`, with its lines, payment and
//!     receipts;
//!   - `set_status_in(tx, TransactionId, StatusMove, now) -> ()`: refuses `staleVersion` when the
//!     stored status is not `from`; a move to `Cancelled` needs a reason (else `validationFailed`)
//!     and stamps `cancelled_at = now`; a move to `PendingApproval` stores the request id; a move to
//!     `Completed` is refused `validationFailed` (only `complete_in` completes);
//!   - `complete_in(tx, TransactionId, receipt_file_key: &str, now) -> (String, Receipt)`: only from
//!     `Confirmed` (else `staleVersion`, no number consumed); takes the next value of
//!     `DocumentRef::counter_key(organization_id, mode, now)`, numbers the exchange
//!     `DocumentRef::format(mode, now, n)`, sets `completed_at = now`, writes receipt revision 1.
//! - A refusal is `Err(AppError::Domain(m))` with `m` starting with its code (`staleVersion`,
//!   `validationFailed`), as TX-1's `QuoteService` answers (`workflows/exchange/quote.rs:90`).

mod quote_common;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use quote_common::*;
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::receipt_record::Receipt;
use xc_exchange::domain::transaction::repository::{StatusMove, TransactionStore};
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};

const ORG_2: &str = "00000000-0000-0000-0000-0000000000b2";

/// 2026-10-06 12:00 in Bangkok.
fn day_1() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 6, 5, 0, 0).unwrap()
}

/// 2026-10-07 00:00 in Bangkok: the next day.
fn day_2() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 6, 17, 0, 0).unwrap()
}

fn org(text: &str) -> Uuid {
    Uuid::parse_str(text).unwrap()
}

fn money(text: &str) -> Money {
    Money::parse(text).unwrap()
}

/// A DEMO BUY of 100 USD at 32.1, fee 10, by bank transfer with a slip, at `branch` of `organization`,
/// with the branch's passport scan (`PASSPORT-<branch>`) and no save extras.
fn draft(
    organization: &str,
    branch: &str,
    customer: &str,
    slip: Option<&str>,
) -> ExchangeTransaction {
    let id = TransactionId(Uuid::new_v4());
    ExchangeTransaction {
        id,
        organization_id: org(organization),
        branch_id: BranchId::new(branch.to_owned()),
        mode: Mode::Demo,
        status: TransactionStatus::Draft,
        document_ref: None,
        quote_id: None,
        booking_id: None,
        shift_id: None,
        submission_id: format!("sub-{}", id.0),
        payload_hash: "hash".to_owned(),
        customer_id: customer.to_owned(),
        customer_name: "Somchai".to_owned(),
        nationality: "THA".to_owned(),
        staff_id: "EMP-001".to_owned(),
        booking_no: "WALK-IN".to_owned(),
        notes: "a note".to_owned(),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
        recorded_at: day_1(),
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
            shop_side: Side::Buy,
            currency: CurrencyCode::parse("USD").unwrap(),
            quantity: ForeignAmount::from_decimal(Decimal::from(100)),
            unit_size: UnitSize::parse("1").unwrap(),
            rate_snapshot: Rate::parse("32.1").unwrap(),
            reference_rate: Some(Rate::parse("32.05").unwrap()),
            thb_amount: money("3210.00"),
            fee_thb: money("10.00"),
            settle_thb: money("3200.00"),
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
            slip_file_key: slip.map(str::to_owned),
            amount_thb: money("3200.00"),
            bank_fee_thb: money("1.50"),
            paid_at: day_1(),
        },
        receipts: vec![],
    }
}

struct Shop {
    db: TestDb,
    store: PgTransactionStore,
}

impl Shop {
    async fn start() -> Self {
        let db = TestDb::start().await;
        db.exec(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT 'CU-1', organization_id, 'CU-1', 'Somchai', 'DEMO', now() FROM branches WHERE id = 'MAIN'",
        )
        .await;
        for key in [
            "PASSPORT-MAIN",
            "SLIP-1",
            "RECEIPT-1",
            "RECEIPT-2",
            "RECEIPT-3",
        ] {
            db.exec(&format!(
                "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ('{key}', 'MAIN', 'owner', 'application/pdf', 'sha-{key}', now())"
            ))
            .await;
        }
        let store = PgTransactionStore::new(db.pool.clone());
        Self { db, store }
    }

    async fn org_2(&self) {
        self.db.second_organisation(ORG_2, "ORG2", "B2").await;
        self.db
            .exec(&format!(
                "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
                 VALUES ('CU-2', '{ORG_2}', 'CU-2', 'Anan', 'DEMO', now())"
            ))
            .await;
        self.db
            .exec(&format!(
                "INSERT INTO files (key, organization_id, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ('RECEIPT-B2', '{ORG_2}', 'B2', 'owner', 'application/pdf', 'sha', now()), \
                        ('PASSPORT-B2', '{ORG_2}', 'B2', 'owner', 'application/pdf', 'sha-p', now())"
            ))
            .await;
    }

    async fn insert(&self, t: &ExchangeTransaction) -> AppResult<()> {
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = self.store.insert_draft_in(&mut tx, t).await;
        tx.commit().await.unwrap();
        got
    }

    async fn find(&self, id: TransactionId) -> Option<ExchangeTransaction> {
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = self.store.find_in(&mut tx, id).await.expect("find_in");
        tx.commit().await.unwrap();
        got
    }

    async fn move_to(
        &self,
        id: TransactionId,
        from: TransactionStatus,
        to: TransactionStatus,
        reason: Option<&str>,
        request: Option<Uuid>,
    ) -> AppResult<()> {
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = self
            .store
            .set_status_in(
                &mut tx,
                id,
                StatusMove {
                    from,
                    to,
                    reason: reason.map(str::to_owned),
                    approval_request_id: request,
                },
                day_1(),
            )
            .await;
        tx.commit().await.unwrap();
        got
    }

    async fn complete(
        &self,
        id: TransactionId,
        receipt: &str,
        now: DateTime<Utc>,
    ) -> AppResult<(String, Receipt)> {
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = self.store.complete_in(&mut tx, id, receipt, now).await;
        tx.commit().await.unwrap();
        got
    }

    /// Insert `t` and move it to CONFIRMED.
    async fn confirmed(&self, t: &ExchangeTransaction) {
        self.insert(t).await.expect("insert the draft");
        self.move_to(
            t.id,
            TransactionStatus::Draft,
            TransactionStatus::Confirmed,
            None,
            None,
        )
        .await
        .expect("confirm it");
    }

    /// (transactions, transaction_lines, transaction_payments, receipts, document counters' sum)
    async fn written(&self) -> (i64, i64, i64, i64, i64) {
        (
            self.db.count("SELECT count(*) FROM transactions").await,
            self.db.count("SELECT count(*) FROM transaction_lines").await,
            self.db
                .count("SELECT count(*) FROM transaction_payments")
                .await,
            self.db.count("SELECT count(*) FROM receipts").await,
            self.db
                .count("SELECT coalesce(sum(value), 0)::bigint FROM counters WHERE key LIKE 'document-%'")
                .await,
        )
    }

    async fn counter(&self, key: &str) -> Option<String> {
        self.db
            .text(&format!(
                "SELECT (SELECT value::text FROM counters WHERE key = '{key}')"
            ))
            .await
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

fn code<T: std::fmt::Debug>(got: AppResult<T>) -> String {
    match got {
        Err(AppError::Domain(m)) => m.split(':').next().unwrap_or("").trim().to_owned(),
        other => panic!("expected Err(AppError::Domain(\"<code>: …\")), got {other:?}"),
    }
}

// ── the draft ────────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_draft_is_stored_with_its_line_and_payment_and_read_back_whole() {
    let shop = Shop::start().await;
    let t = draft(ORG_1, "MAIN", "CU-1", Some("SLIP-1"));
    shop.insert(&t).await.expect("a draft is stored");

    assert_eq!(shop.find(t.id).await, Some(t.clone()), "read back whole");
    assert_eq!(
        shop.written().await,
        (1, 1, 1, 0, 0),
        "no number, no receipt"
    );
    assert_eq!(
        shop.db
            .text(&format!(
                "SELECT status || '|' || coalesce(document_ref, 'NULL') FROM transactions \
                 WHERE id = '{}'",
                t.id.0
            ))
            .await
            .as_deref(),
        Some("DRAFT|NULL"),
        "D-3: no number before completion"
    );
    // D-2's wide row retired with TX-9 (#677): the line and the payment are the only copy.
    assert_eq!(
        shop.db
            .count(&format!(
                "SELECT count(*) FROM transaction_files WHERE transaction_id = '{}' AND kind = 'SLIP'",
                t.id.0
            ))
            .await,
        0,
        "TX-9 #677 ruling: the slip lives on the payment only"
    );
    assert_eq!(
        shop.db
            .text(&format!(
                "SELECT file_key FROM transaction_files WHERE transaction_id = '{}' AND kind = 'PASSPORT'",
                t.id.0
            ))
            .await
            .as_deref(),
        Some("PASSPORT-MAIN"),
        "#672 Q2: the PASSPORT row, written by the store"
    );
    shop.finish().await;
}

#[tokio::test]
async fn only_an_unnumbered_draft_with_one_line_is_stored() {
    let shop = Shop::start().await;
    let base = draft(ORG_1, "MAIN", "CU-1", None);

    let mut confirmed = base.clone();
    confirmed.status = TransactionStatus::Confirmed;
    let mut numbered = draft(ORG_1, "MAIN", "CU-1", None);
    numbered.document_ref = Some("TEST-XC-20261006-000001".to_owned());
    let mut lineless = draft(ORG_1, "MAIN", "CU-1", None);
    lineless.lines.clear();
    let mut two_lines = draft(ORG_1, "MAIN", "CU-1", None);
    let mut second = two_lines.lines[0].clone();
    second.id = Uuid::new_v4();
    second.line_no = 2;
    two_lines.lines.push(second);
    let mut with_receipt = draft(ORG_1, "MAIN", "CU-1", None);
    with_receipt.receipts.push(Receipt {
        id: Uuid::new_v4(),
        number: "TEST-XC-20261006-000001".to_owned(),
        revision: 1,
        issued_at: day_1(),
        document_ref: "RECEIPT-1".to_owned(),
    });

    for (what, t) in [
        ("a CONFIRMED", &confirmed),
        ("a numbered draft", &numbered),
        ("a draft without a line", &lineless),
        ("a draft with two lines", &two_lines),
        ("a draft with a receipt", &with_receipt),
    ] {
        assert_eq!(code(shop.insert(t).await), "validationFailed", "{what}");
    }
    assert_eq!(shop.written().await, (0, 0, 0, 0, 0), "nothing written");
    assert_eq!(
        shop.db
            .count("SELECT count(*) FROM transaction_files")
            .await,
        0,
        "no file link written"
    );
    shop.finish().await;
}

/// #672 Q2/Q3: the passport scan and the three save extras are stored at insert and read back.
#[tokio::test]
async fn the_passport_file_and_the_save_extras_are_stored_and_read_back() {
    let shop = Shop::start().await;
    let mut t = draft(ORG_1, "MAIN", "CU-1", None);
    t.kiosk_request_id = Some("KR-1".to_owned());
    t.staff_draft_id = Some("SD-1".to_owned());
    t.count_confirmed = true;
    shop.insert(&t).await.expect("a draft is stored");

    let back = shop.find(t.id).await.expect("found");
    assert_eq!(back.passport_file_key, "PASSPORT-MAIN", "the passport scan");
    assert_eq!(back.kiosk_request_id.as_deref(), Some("KR-1"));
    assert_eq!(back.staff_draft_id.as_deref(), Some("SD-1"));
    assert!(back.count_confirmed, "the cash was counted");
    assert_eq!(back, t, "read back whole");
    assert_eq!(
        shop.db
            .text(&format!(
                "SELECT concat_ws('|', kiosk_request_id, staff_draft_id, count_confirmed::text) \
                 FROM transactions WHERE id = '{}'",
                t.id.0
            ))
            .await
            .as_deref(),
        Some("KR-1|SD-1|true"),
        "Q3: the columns on transactions"
    );
    assert_eq!(
        shop.db
            .text(&format!(
                "SELECT string_agg(kind || '=' || file_key, ',' ORDER BY kind) FROM transaction_files \
                 WHERE transaction_id = '{}'",
                t.id.0
            ))
            .await
            .as_deref(),
        Some("PASSPORT=PASSPORT-MAIN"),
        "Q2: the PASSPORT row, and no SLIP row without a slip"
    );
    shop.finish().await;
}

// ── the moves ────────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_status_moves_only_from_the_status_that_is_stored() {
    use TransactionStatus::*;
    let shop = Shop::start().await;
    let t = draft(ORG_1, "MAIN", "CU-1", None);
    shop.insert(&t).await.unwrap();

    shop.move_to(t.id, Draft, Confirmed, None, None)
        .await
        .expect("DRAFT → CONFIRMED");
    assert_eq!(
        code(shop.move_to(t.id, Draft, Confirmed, None, None).await),
        "staleVersion",
        "the stored status is no longer DRAFT"
    );
    assert_eq!(shop.find(t.id).await.unwrap().status, Confirmed);

    assert_eq!(
        code(shop.move_to(t.id, Confirmed, Cancelled, None, None).await),
        "validationFailed",
        "a cancel without a reason"
    );
    assert_eq!(
        code(shop.move_to(t.id, Confirmed, Completed, None, None).await),
        "validationFailed",
        "only complete_in completes"
    );
    assert_eq!(
        shop.find(t.id).await.unwrap().status,
        Confirmed,
        "unchanged"
    );

    shop.move_to(t.id, Confirmed, Cancelled, Some("customer left"), None)
        .await
        .expect("CONFIRMED → CANCELLED with a reason");
    let cancelled = shop.find(t.id).await.unwrap();
    assert_eq!(
        (
            cancelled.status,
            cancelled.cancelled_at,
            cancelled.cancel_reason.as_deref(),
            cancelled.document_ref.as_deref()
        ),
        (Cancelled, Some(day_1()), Some("customer left"), None),
        "D-3: a cancelled exchange burns no number"
    );
    assert_eq!(
        shop.written().await,
        (1, 1, 1, 0, 0),
        "no number and no receipt"
    );
    shop.finish().await;
}

#[tokio::test]
async fn a_move_to_pending_approval_keeps_the_request() {
    use TransactionStatus::*;
    let shop = Shop::start().await;
    let request = Uuid::new_v4();
    shop.db
        .exec(&format!(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
                object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
                status, idempotency_key) \
             VALUES ('{request}', '{ORG_1}', 'EXCHANGE_TRANSACTION', 'MAIN', '{request}', 'h', '1', \
                '{{}}'::jsonb, 'owner', now(), 'PENDING', 'k-{request}')"
        ))
        .await;
    let t = draft(ORG_1, "MAIN", "CU-1", None);
    shop.insert(&t).await.unwrap();
    shop.move_to(t.id, Draft, PendingApproval, None, Some(request))
        .await
        .expect("DRAFT → PENDING_APPROVAL");
    let pending = shop.find(t.id).await.unwrap();
    assert_eq!(
        (pending.status, pending.approval_request_id),
        (PendingApproval, Some(request))
    );
    shop.finish().await;
}

#[tokio::test]
async fn an_unknown_transaction_is_not_found_and_cannot_move() {
    use TransactionStatus::*;
    let shop = Shop::start().await;
    let nobody = TransactionId(Uuid::new_v4());
    assert_eq!(shop.find(nobody).await, None);
    assert!(
        shop.move_to(nobody, Draft, Confirmed, None, None)
            .await
            .is_err(),
        "no row moves"
    );
    assert!(
        shop.complete(nobody, "RECEIPT-1", day_1()).await.is_err(),
        "no row completes"
    );
    assert_eq!(shop.written().await, (0, 0, 0, 0, 0));
    shop.finish().await;
}

// ── the completion ───────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn completing_numbers_the_exchange_and_issues_receipt_revision_1() {
    let shop = Shop::start().await;
    let t = draft(ORG_1, "MAIN", "CU-1", None);
    shop.confirmed(&t).await;

    let (number, receipt) = shop
        .complete(t.id, "RECEIPT-1", day_1())
        .await
        .expect("a CONFIRMED exchange completes");
    assert_eq!(number, "TEST-XC-20261006-000001");
    assert_eq!(
        (
            receipt.number.as_str(),
            receipt.revision,
            receipt.issued_at,
            receipt.document_ref.as_str()
        ),
        ("TEST-XC-20261006-000001", 1, day_1(), "RECEIPT-1"),
        "§3.5: revision 1 carries the transaction's number and the stored receipt file"
    );

    let done = shop.find(t.id).await.unwrap();
    assert_eq!(done.status, TransactionStatus::Completed);
    assert_eq!(done.document_ref.as_deref(), Some(number.as_str()));
    assert_eq!(done.completed_at, Some(day_1()));
    assert_eq!(done.receipts, vec![receipt]);
    assert_eq!(
        shop.counter(&format!("document-{ORG_1}-DEMO-20261006"))
            .await
            .as_deref(),
        Some("1"),
        "D-4: the organisation's counter"
    );
    assert_eq!(shop.written().await, (1, 1, 1, 1, 1));
    shop.finish().await;
}

#[tokio::test]
async fn only_a_confirmed_exchange_completes_and_a_refusal_consumes_no_number() {
    use TransactionStatus::*;
    let shop = Shop::start().await;
    let t = draft(ORG_1, "MAIN", "CU-1", None);
    shop.insert(&t).await.unwrap();
    assert_eq!(
        code(shop.complete(t.id, "RECEIPT-1", day_1()).await),
        "staleVersion",
        "a DRAFT"
    );
    shop.move_to(t.id, Draft, Confirmed, None, None)
        .await
        .unwrap();
    shop.complete(t.id, "RECEIPT-1", day_1()).await.unwrap();
    assert_eq!(
        code(shop.complete(t.id, "RECEIPT-2", day_1()).await),
        "staleVersion",
        "a COMPLETED exchange is not completed twice"
    );
    assert_eq!(
        shop.written().await,
        (1, 1, 1, 1, 1),
        "one number, one receipt"
    );
    shop.finish().await;
}

#[tokio::test]
async fn numbers_run_per_organisation_and_restart_each_day() {
    let shop = Shop::start().await;
    shop.org_2().await;
    let a = draft(ORG_1, "MAIN", "CU-1", None);
    let b = draft(ORG_1, "MAIN", "CU-1", None);
    let c = draft(ORG_1, "MAIN", "CU-1", None);
    let other = draft(ORG_2, "B2", "CU-2", None);
    for t in [&a, &b, &c, &other] {
        shop.confirmed(t).await;
    }

    let number = |got: AppResult<(String, Receipt)>| got.expect("completes").0;
    assert_eq!(
        number(shop.complete(a.id, "RECEIPT-1", day_1()).await),
        "TEST-XC-20261006-000001"
    );
    assert_eq!(
        number(shop.complete(b.id, "RECEIPT-2", day_1()).await),
        "TEST-XC-20261006-000002"
    );
    assert_eq!(
        number(shop.complete(other.id, "RECEIPT-B2", day_1()).await),
        "TEST-XC-20261006-000001",
        "D-4: another organisation numbers from 1"
    );
    assert_eq!(
        number(shop.complete(c.id, "RECEIPT-3", day_2()).await),
        "TEST-XC-20261007-000001",
        "a new Bangkok day starts again"
    );
    for (key, value) in [
        (format!("document-{ORG_1}-DEMO-20261006"), "2"),
        (format!("document-{ORG_2}-DEMO-20261006"), "1"),
        (format!("document-{ORG_1}-DEMO-20261007"), "1"),
    ] {
        assert_eq!(shop.counter(&key).await.as_deref(), Some(value), "{key}");
    }
    shop.finish().await;
}

#[tokio::test]
async fn completion_continues_a_days_sequence_already_in_the_counter() {
    let shop = Shop::start().await;
    shop.db
        .exec(&format!(
            "INSERT INTO counters (key, value) VALUES ('document-{ORG_1}-DEMO-20261006', 7)"
        ))
        .await;
    let t = draft(ORG_1, "MAIN", "CU-1", None);
    shop.confirmed(&t).await;
    let (number, _) = shop.complete(t.id, "RECEIPT-1", day_1()).await.unwrap();
    assert_eq!(number, "TEST-XC-20261006-000008");
    shop.finish().await;
}
