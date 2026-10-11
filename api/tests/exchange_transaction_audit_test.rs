//! Ticket #1262 (TX-11) — the audit of the transaction commands: each of create, confirm (under and over the
//! limit), cancel (from DRAFT and from CONFIRMED) and complete appends **exactly one** `audit_events` row through
//! AP-1's `AuditLog`, in the command's own transaction (design `docs/design/blueprint/quote-transaction.md` §4.7,
//! §9.1 TX-11; Blueprint §10 *"ทุก mutation ใช้ authenticated actor และ audit"*).
//! Written before the implementation by the up-front test writer (claude/709ec749, on the xchange BE lane
//! controller claude/ad05bf8d's order, 2026-10-11). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! What §4.7 makes `[required]` and these tests pin:
//! - `action`: `transaction.created` | `transaction.confirmed` | `transaction.pendingApproval` |
//!   `transaction.cancelled` | `transaction.completed`;
//! - `actor_id` / `actor_role` = the caller's `SaveActor.id` / `SaveActor.role.as_str()`. Every command here is run
//!   by a **different** caller from the one who made the exchange, so an actor read off the stored row (its
//!   `staff_id` / `recorded_by`) is not the caller's;
//! - `scope` `"branch"`, `branch_id` = the transaction's branch, `object_ref` = the transaction id;
//! - `before` / `after` exactly §4.7's table. The ticket author reads the table as the whole object (the status plus
//!   the fields that command changes, and nothing else: *"No customer, passport or payment field"*), so the
//!   redacted columns are compared whole. Object on #1262 if that reading is wrong.
//!
//! The rest of the §9.1 row:
//! - a refused command (`quoteExpired` at create, `staleVersion` on cancel from PENDING_APPROVAL,
//!   `stockInsufficient` at complete) leaves no row. Each of these tests then runs the same command successfully,
//!   so it is red where no audit is written at all;
//! - a `run_in` that succeeds in a transaction the test rolls back leaves no row, for each of the four commands
//!   (an append on a transaction of its own would survive the rollback);
//! - an Idempotency-Key replay through the routes adds no second row;
//! - an approve or a reject leaves AP-1's one `approval.*` row and no further `transaction.*` row;
//! - no redacted column holds the customer's passport number.
//!
//! Not tested here, on purpose (§9.1): the compat save. It reaches these workflows only through TX-6's adapter,
//! which may merge before or after TX-11, so a TX-11 test of it would assert zero rows or three by merge order.
//!
//! Each service call runs in a transaction the test opens and **commits whatever the answer** (as `tx4_common`),
//! except the rollback test, which rolls back. Needs `TEST_DATABASE_URL` naming an existing database whose name
//! ends in `_test`; nothing here skips. Each test works in a schema of its own.

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Once};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, NaiveDate, Utc};
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use quote_common::*;
use serde_json::{Value, json};
use tx4_common::*;
use tx5_common::*;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::application::customer::service::CustomerService;
use xc_exchange::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::quote::quote::Quote;
use xc_exchange::domain::transaction::transaction::{ExchangeTransaction, TransactionId};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xchange::application::workflows::exchange::cancel::CancelTransaction;
use xchange::application::workflows::exchange::confirm::{ConfirmOutcome, ConfirmTransaction};
use xchange::application::workflows::exchange::create::{
    CreatePayment, CreateRequest, CreateTransaction,
};
use xchange::application::workflows::save_step::{SaveActor, SaveExtras};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

/// CI-4 (#651): the test PII key, as `tx4_create_test.rs`.
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
/// The customer's passport number: no redacted column may hold it.
const PASSPORT_NO: &str = "XC7654321";
/// The passport scan of `creator()` at MAIN (create checks the scan is the caller's, #672 Q2).
const CREATOR_SCAN: &str = "PASSPORT-CREATOR";

static PII: Once = Once::new();

// ------------------------------------------------------------ callers

fn actor(id: &str, role: Role) -> SaveActor {
    SaveActor {
        id: id.to_owned(),
        username: id.trim_start_matches("employee-").to_owned(),
        employee_id: Some(id.to_owned()),
        role,
    }
}

/// Makes the exchanges in the create tests.
fn creator() -> SaveActor {
    actor("employee-creator", Role::Backend)
}

/// Confirms: neither the exchange's maker nor its approver.
fn confirmer() -> SaveActor {
    actor("employee-confirmer", Role::Front)
}

/// Cancels and completes.
fn closer() -> SaveActor {
    actor("employee-closer", Role::Finance)
}

// ------------------------------------------------------------ the counter

/// `tx5_common::Counter` (MAIN's open shift, the posting double) plus a DEMO customer with a passport number and
/// the creator's passport scan at MAIN.
struct Audited {
    c: Counter,
    customer: String,
}

impl Audited {
    async fn start() -> Self {
        PII.call_once(|| {
            let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
            xc_customer::install_pii_keys(keys).expect("one test PII key per process");
        });
        let c = Counter::start().await;
        let pool = c.desk.db.pool.clone();
        let customer = CustomerService::new(BranchTx::new(Transactor::new(pool)))
            .reserve(
                &BranchId::new("MAIN"),
                ReserveCustomer {
                    name: "Somchai Jaidee".to_owned(),
                    passport_no: PASSPORT_NO.to_owned(),
                    nationality: "THA".to_owned(),
                    birth_date: String::new(),
                    passport_expiry: String::new(),
                    mode: "DEMO".to_owned(),
                },
                whole_second_now(),
            )
            .await
            .unwrap_or_else(|e| panic!("reserve: {e}"))
            .customer_id;
        c.desk
            .db
            .exec(&format!(
                "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ('{CREATOR_SCAN}', 'MAIN', '{}', 'image/jpeg', 'sha-creator', now())",
                creator().id
            ))
            .await;
        Self { c, customer }
    }

    async fn finish(self) {
        self.c.finish().await;
    }

    fn desk(&self) -> &Desk {
        &self.c.desk
    }

    fn request(&self, q: &Quote) -> CreateRequest {
        CreateRequest {
            branch_id: BranchId::new("MAIN"),
            quote_id: q.id,
            customer_id: self.customer.clone(),
            booking_id: None,
            kiosk_request_id: None,
            staff_draft_id: None,
            passport_key: CREATOR_SCAN.to_owned(),
            payment: CreatePayment {
                method: PaymentMethod::Cash,
                reference: "R-1".to_owned(),
                account: "Cash drawer 1".to_owned(),
                slip_key: None,
                bank_fee_thb: money("0.00"),
                paid_at: q.created_at,
            },
            value_date: NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
            notes: "a note".to_owned(),
            count_confirmed: true,
        }
    }

    /// `CreateTransaction::run_in` as `creator()`, committed whatever the answer.
    async fn create(&self, q: &Quote, now: DateTime<Utc>) -> AppResult<ExchangeTransaction> {
        let service = CreateTransaction::new(self.desk().db.pool.clone(), self.c.files.clone());
        let mut tx = self.desk().db.pool.begin().await.unwrap();
        let got = service
            .run_in(&mut tx, self.request(q), &creator(), now)
            .await;
        tx.commit().await.unwrap();
        got
    }

    /// A DRAFT made through create at MAIN: 100 USD bought at 32.10 ⇒ settle 3210.00.
    async fn draft(&self, now: DateTime<Utc>) -> ExchangeTransaction {
        let q = self.desk().quote("MAIN", Side::Buy, "100", None, now).await;
        self.create(&q, now).await.expect("create")
    }

    /// `ConfirmTransaction::run_in` as `confirmer()`, committed whatever the answer.
    async fn confirm(&self, id: TransactionId, now: DateTime<Utc>) -> AppResult<ConfirmOutcome> {
        let service = ConfirmTransaction::new(
            self.desk().db.pool.clone(),
            self.desk().approvals.clone(),
            Vec::new(),
        );
        let mut tx = self.desk().db.pool.begin().await.unwrap();
        let got = service
            .run_in(&mut tx, id, &confirmer(), &SaveExtras::default(), now)
            .await;
        tx.commit().await.unwrap();
        got
    }

    /// `CancelTransaction::run_in` as `closer()`, committed whatever the answer.
    async fn cancel(
        &self,
        id: TransactionId,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<ExchangeTransaction> {
        let service = CancelTransaction::new(self.desk().db.pool.clone());
        let mut tx = self.desk().db.pool.begin().await.unwrap();
        let got = service.run_in(&mut tx, id, reason, &closer(), now).await;
        tx.commit().await.unwrap();
        got
    }

    /// `CompleteExchange::run_in` as `closer()` with `ledger`, committed whatever the answer.
    async fn complete(
        &self,
        ledger: &Ledger,
        id: TransactionId,
        now: DateTime<Utc>,
    ) -> AppResult<ExchangeTransaction> {
        let service = self.c.service(ledger, Vec::new(), Vec::new());
        let mut tx = self.desk().db.pool.begin().await.unwrap();
        let got = service.run_in(&mut tx, id, &closer(), now).await;
        tx.commit().await.unwrap();
        got.map(|done| done.transaction)
    }

    /// The `audit_events` rows whose `object_ref` is `object`, oldest first.
    async fn audits(&self, object: &str) -> Vec<Value> {
        audits_of(self.desk(), object).await
    }
}

/// The `audit_events` rows whose `object_ref` is `object`, oldest first, as JSON objects.
async fn audits_of(desk: &Desk, object: &str) -> Vec<Value> {
    let text = desk
        .db
        .text(&format!(
            "SELECT COALESCE(json_agg(json_build_object( \
                 'actorId', actor_id, 'actorRole', actor_role, 'scope', scope, 'branchId', branch_id, \
                 'action', action, 'objectRef', object_ref, \
                 'before', before_redacted, 'after', after_redacted) ORDER BY id), '[]'::json)::text \
             FROM audit_events WHERE object_ref = '{object}'"
        ))
        .await
        .expect("a JSON array");
    match serde_json::from_str(&text).expect("JSON") {
        Value::Array(rows) => rows,
        other => panic!("an array, got {other}"),
    }
}

fn id_of(t: &ExchangeTransaction) -> String {
    t.id.0.to_string()
}

/// The rows of `rows` whose action is `action`.
fn with_action<'a>(rows: &'a [Value], action: &str) -> Vec<&'a Value> {
    rows.iter().filter(|r| r["action"] == action).collect()
}

/// The rows of `rows` whose action starts with `transaction.`.
fn transaction_rows(rows: &[Value]) -> Vec<&Value> {
    rows.iter()
        .filter(|r| {
            r["action"]
                .as_str()
                .is_some_and(|a| a.starts_with("transaction."))
        })
        .collect()
}

/// `rows` is exactly one row, §4.7's for `action` by `who` on `t` at MAIN, with `before` / `after` (`Value::Null`
/// for none).
fn assert_one(
    rows: &[Value],
    t: &ExchangeTransaction,
    action: &str,
    who: &SaveActor,
    before: Value,
    after: Value,
) {
    assert_eq!(
        rows.len(),
        1,
        "exactly one audit row for {action}: {rows:#?}"
    );
    let row = &rows[0];
    assert_eq!(row["action"], json!(action), "§4.7 action: {row:#}");
    assert_eq!(
        row["objectRef"],
        json!(id_of(t)),
        "object_ref = the transaction id"
    );
    assert_eq!(
        (row["actorId"].clone(), row["actorRole"].clone()),
        (json!(who.id), json!(who.role.as_str())),
        "actor = the caller of {action}"
    );
    assert_eq!(row["scope"], json!("branch"), "scope");
    assert_eq!(
        row["branchId"],
        json!(t.branch_id.as_str()),
        "branch_id = the transaction's branch"
    );
    assert_eq!(row["before"], before, "before of {action}");
    assert_eq!(row["after"], after, "after of {action}");
}

// ------------------------------------------------------------ one row per command (§4.7 table)

#[tokio::test]
async fn create_writes_one_created_row_with_the_quote_and_no_before() {
    let a = Audited::start().await;
    let now = whole_second_now();
    let q = a.desk().quote("MAIN", Side::Buy, "100", None, now).await;
    let t = a.create(&q, now).await.expect("create");

    assert_one(
        &a.audits(&id_of(&t)).await,
        &t,
        "transaction.created",
        &creator(),
        Value::Null,
        json!({ "status": "DRAFT", "quoteId": q.id.to_string() }),
    );
    a.finish().await;
}

#[tokio::test]
async fn confirm_under_the_limit_writes_one_confirmed_row() {
    let a = Audited::start().await;
    let now = whole_second_now();
    let t = a.draft(now).await;
    assert_eq!(
        a.confirm(t.id, later(now, 1)).await.expect("confirm"),
        ConfirmOutcome::Confirmed
    );

    let rows = a.audits(&id_of(&t)).await;
    let confirmed: Vec<Value> = with_action(&rows, "transaction.confirmed")
        .into_iter()
        .cloned()
        .collect();
    assert_one(
        &confirmed,
        &t,
        "transaction.confirmed",
        &confirmer(),
        json!({ "status": "DRAFT" }),
        json!({ "status": "CONFIRMED", "quoteId": t.quote_id.unwrap().to_string() }),
    );
    assert_eq!(
        transaction_rows(&rows).len(),
        2,
        "create's row and confirm's, nothing else: {rows:#?}"
    );
    a.finish().await;
}

#[tokio::test]
async fn confirm_over_the_limit_writes_one_pending_approval_row_naming_the_request() {
    let a = Audited::start().await;
    let now = whole_second_now();
    let t = a.draft(now).await;
    a.desk().limit("MAIN", "1000.00").await;
    let request = match a.confirm(t.id, later(now, 1)).await {
        Ok(ConfirmOutcome::PendingApproval(request)) => request,
        other => panic!("3210.00 > 1000.00 waits for approval: {other:?}"),
    };

    let rows = a.audits(&id_of(&t)).await;
    let pending: Vec<Value> = with_action(&rows, "transaction.pendingApproval")
        .into_iter()
        .cloned()
        .collect();
    assert_one(
        &pending,
        &t,
        "transaction.pendingApproval",
        &confirmer(),
        json!({ "status": "DRAFT" }),
        json!({
            "status": "PENDING_APPROVAL",
            "quoteId": t.quote_id.unwrap().to_string(),
            "approvalRequestId": request.as_str(),
        }),
    );
    assert_eq!(
        transaction_rows(&rows).len(),
        2,
        "create's row and confirm's, nothing else: {rows:#?}"
    );
    a.finish().await;
}

#[tokio::test]
async fn cancel_of_a_draft_writes_one_cancelled_row_with_the_reason() {
    let a = Audited::start().await;
    let now = whole_second_now();
    let t = a.draft(now).await;
    a.cancel(t.id, "customer left", later(now, 1))
        .await
        .expect("cancel a DRAFT");

    let rows = a.audits(&id_of(&t)).await;
    let cancelled: Vec<Value> = with_action(&rows, "transaction.cancelled")
        .into_iter()
        .cloned()
        .collect();
    assert_one(
        &cancelled,
        &t,
        "transaction.cancelled",
        &closer(),
        json!({ "status": "DRAFT" }),
        json!({ "status": "CANCELLED", "reason": "customer left" }),
    );
    a.finish().await;
}

#[tokio::test]
async fn cancel_of_a_confirmed_exchange_writes_one_cancelled_row_from_confirmed() {
    let a = Audited::start().await;
    let now = whole_second_now();
    let t = a.draft(now).await;
    a.confirm(t.id, later(now, 1)).await.expect("confirm");
    a.cancel(t.id, "wrong currency", later(now, 2))
        .await
        .expect("cancel a CONFIRMED");

    let rows = a.audits(&id_of(&t)).await;
    let cancelled: Vec<Value> = with_action(&rows, "transaction.cancelled")
        .into_iter()
        .cloned()
        .collect();
    assert_one(
        &cancelled,
        &t,
        "transaction.cancelled",
        &closer(),
        json!({ "status": "CONFIRMED" }),
        json!({ "status": "CANCELLED", "reason": "wrong currency" }),
    );
    assert_eq!(
        transaction_rows(&rows).len(),
        3,
        "created, confirmed, cancelled: {rows:#?}"
    );
    a.finish().await;
}

/// A LIVE exchange on MAIN's open shift (`tx5_common`), so complete posts; the double answers its costs.
#[tokio::test]
async fn complete_writes_one_completed_row_with_the_document_ref_and_completed_at() {
    let a = Audited::start().await;
    let now = whole_second_now();
    let t = a.c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    let done = a
        .complete(&Ledger::new("3200.00", None), t.id, later(now, 2))
        .await
        .expect("complete");
    let stored = a.desk().stored(t.id).await;
    let document_ref = stored
        .document_ref
        .clone()
        .expect("a COMPLETED exchange has its number");
    let completed_at = stored
        .completed_at
        .expect("a COMPLETED exchange has completed_at");
    assert_eq!(done.document_ref.as_deref(), Some(document_ref.as_str()));

    let rows = a.audits(&id_of(&t)).await;
    let completed: Vec<Value> = with_action(&rows, "transaction.completed")
        .into_iter()
        .cloned()
        .collect();
    assert_eq!(completed.len(), 1, "exactly one completed row: {rows:#?}");
    // `completedAt` is compared as an instant: the design names the value, not its spelling.
    let after = &completed[0]["after"];
    let written: DateTime<Utc> = after["completedAt"]
        .as_str()
        .unwrap_or_else(|| panic!("after.completedAt is a timestamp string: {after}"))
        .parse()
        .unwrap_or_else(|e| panic!("after.completedAt {after}: {e}"));
    assert_eq!(
        written, completed_at,
        "completedAt = the row's completed_at"
    );
    let mut spelled = after.clone();
    spelled["completedAt"] = json!(completed_at.to_rfc3339());
    let mut row = completed[0].clone();
    row["after"] = spelled;
    assert_one(
        &[row],
        &t,
        "transaction.completed",
        &closer(),
        json!({ "status": "CONFIRMED" }),
        json!({
            "status": "COMPLETED",
            "documentRef": document_ref,
            "completedAt": completed_at.to_rfc3339(),
        }),
    );
    a.finish().await;
}

// ------------------------------------------------------------ a refusal writes nothing

#[tokio::test]
async fn a_create_refused_as_quote_expired_writes_no_row_and_the_next_create_writes_one() {
    let a = Audited::start().await;
    let now = whole_second_now();
    let expired = a.desk().quote("MAIN", Side::Buy, "100", None, now).await;
    assert_eq!(
        code(a.create(&expired, expired.expires_at).await),
        "quoteExpired"
    );
    assert_eq!(
        a.desk()
            .db
            .count("SELECT count(*) FROM audit_events WHERE action LIKE 'transaction.%'")
            .await,
        0,
        "the refused create wrote no row"
    );

    let t = a.draft(later(now, 1)).await;
    assert_eq!(
        with_action(&a.audits(&id_of(&t)).await, "transaction.created").len(),
        1,
        "the next create writes its one row"
    );
    assert_eq!(
        a.desk()
            .db
            .count("SELECT count(*) FROM audit_events WHERE action LIKE 'transaction.%'")
            .await,
        1,
        "and only that row"
    );
    a.finish().await;
}

#[tokio::test]
async fn a_cancel_of_a_pending_approval_is_stale_and_writes_no_row() {
    let a = Audited::start().await;
    let now = whole_second_now();
    let t = a.draft(now).await;
    a.desk().limit("MAIN", "1000.00").await;
    assert!(matches!(
        a.confirm(t.id, later(now, 1)).await,
        Ok(ConfirmOutcome::PendingApproval(_))
    ));
    let before = a.audits(&id_of(&t)).await;

    assert_eq!(
        code(a.cancel(t.id, "too late", later(now, 2)).await),
        "staleVersion"
    );
    let after = a.audits(&id_of(&t)).await;
    assert_eq!(after, before, "the refused cancel wrote no row");
    assert_eq!(
        transaction_rows(&after)
            .iter()
            .map(|r| r["action"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>(),
        ["transaction.created", "transaction.pendingApproval"],
        "create's and confirm's rows, in order"
    );
    a.finish().await;
}

#[tokio::test]
async fn a_complete_refused_as_stock_insufficient_writes_no_row_and_the_retry_writes_one() {
    let a = Audited::start().await;
    let now = whole_second_now();
    let t = a.c.live(Side::Sell, PaymentMethod::Cash, true, now).await;
    let before = a.audits(&id_of(&t)).await;

    assert_eq!(
        code(a.complete(&Ledger::refusing(), t.id, later(now, 2)).await),
        "stockInsufficient"
    );
    assert_eq!(
        a.audits(&id_of(&t)).await,
        before,
        "the refused complete wrote no row"
    );

    a.complete(&Ledger::new("3200.00", None), t.id, later(now, 3))
        .await
        .expect("the retry completes");
    assert_eq!(
        with_action(&a.audits(&id_of(&t)).await, "transaction.completed").len(),
        1,
        "the retry writes its one row"
    );
    a.finish().await;
}

// ------------------------------------------------------------ the row is in the command's transaction

/// Each command's `run_in` succeeds in a transaction the test rolls back: no row survives. The same command,
/// committed, then writes its one row. An append on a transaction of its own would survive the rollback.
#[tokio::test]
async fn a_command_rolled_back_by_its_caller_leaves_no_row_and_committed_leaves_one() {
    let a = Audited::start().await;
    let now = whole_second_now();
    let pool = a.desk().db.pool.clone();
    let transaction_rows_in_table = || async {
        a.desk()
            .db
            .count("SELECT count(*) FROM audit_events WHERE action LIKE 'transaction.%'")
            .await
    };

    // create
    let q = a.desk().quote("MAIN", Side::Buy, "100", None, now).await;
    let service = CreateTransaction::new(pool.clone(), a.c.files.clone());
    let mut tx = pool.begin().await.unwrap();
    let rolled = service
        .run_in(&mut tx, a.request(&q), &creator(), now)
        .await
        .expect("create inside the caller's transaction");
    tx.rollback().await.unwrap();
    assert_eq!(
        a.audits(&id_of(&rolled)).await,
        Vec::<Value>::new(),
        "create rolled back"
    );
    assert_eq!(transaction_rows_in_table().await, 0, "create rolled back");
    let t = a.create(&q, now).await.expect("create, committed");
    assert_eq!(transaction_rows_in_table().await, 1, "create committed");

    // confirm
    let service = ConfirmTransaction::new(pool.clone(), a.desk().approvals.clone(), Vec::new());
    let mut tx = pool.begin().await.unwrap();
    service
        .run_in(
            &mut tx,
            t.id,
            &confirmer(),
            &SaveExtras::default(),
            later(now, 1),
        )
        .await
        .expect("confirm inside the caller's transaction");
    tx.rollback().await.unwrap();
    assert_eq!(transaction_rows_in_table().await, 1, "confirm rolled back");
    a.confirm(t.id, later(now, 1))
        .await
        .expect("confirm, committed");
    assert_eq!(transaction_rows_in_table().await, 2, "confirm committed");

    // cancel (of another DRAFT)
    let other = a.draft(later(now, 2)).await;
    assert_eq!(transaction_rows_in_table().await, 3, "the second create");
    let service = CancelTransaction::new(pool.clone());
    let mut tx = pool.begin().await.unwrap();
    service
        .run_in(&mut tx, other.id, "rolled back", &closer(), later(now, 3))
        .await
        .expect("cancel inside the caller's transaction");
    tx.rollback().await.unwrap();
    assert_eq!(transaction_rows_in_table().await, 3, "cancel rolled back");
    a.cancel(other.id, "committed", later(now, 3))
        .await
        .expect("cancel, committed");
    assert_eq!(transaction_rows_in_table().await, 4, "cancel committed");

    // complete (a LIVE exchange; its create and confirm went through `Desk`, which writes through the store
    // directly for the draft but through `ConfirmTransaction` for the confirm)
    let live =
        a.c.live(Side::Buy, PaymentMethod::Cash, true, later(now, 4))
            .await;
    let settled = transaction_rows_in_table().await;
    let service =
        a.c.service(&Ledger::new("3200.00", None), Vec::new(), Vec::new());
    let mut tx = pool.begin().await.unwrap();
    service
        .run_in(&mut tx, live.id, &closer(), later(now, 5))
        .await
        .expect("complete inside the caller's transaction");
    tx.rollback().await.unwrap();
    assert_eq!(
        with_action(&a.audits(&id_of(&live)).await, "transaction.completed").len(),
        0,
        "complete rolled back"
    );
    assert_eq!(
        transaction_rows_in_table().await,
        settled,
        "complete rolled back"
    );
    a.complete(&Ledger::new("3200.00", None), live.id, later(now, 5))
        .await
        .expect("complete, committed");
    assert_eq!(
        transaction_rows_in_table().await,
        settled + 1,
        "complete committed"
    );
    a.finish().await;
}

// ------------------------------------------------------------ the approval's decision is AP-1's row only

async fn decided_rows(outcome: Outcome, reason: &str) {
    let a = Audited::start().await;
    let now = whole_second_now();
    let t = a.draft(now).await;
    a.desk().limit("MAIN", "1000.00").await;
    let request: RequestId = match a.confirm(t.id, later(now, 1)).await {
        Ok(ConfirmOutcome::PendingApproval(request)) => request,
        other => panic!("3210.00 > 1000.00 waits for approval: {other:?}"),
    };
    a.desk()
        .decide(
            &request,
            &staff(MANAGER, Role::Manager, "MAIN"),
            outcome,
            reason,
        )
        .await
        .expect("the branch's manager decides");

    let rows = a.audits(&id_of(&t)).await;
    let decision = format!("approval.{}", outcome.as_str().to_lowercase());
    assert_eq!(
        with_action(&rows, &decision).len(),
        1,
        "AP-1's one {decision} row: {rows:#?}"
    );
    assert_eq!(
        transaction_rows(&rows)
            .iter()
            .map(|r| r["action"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>(),
        ["transaction.created", "transaction.pendingApproval"],
        "on_decided writes no transaction.* row of its own"
    );
    assert_eq!(
        rows.len(),
        3,
        "created, pendingApproval, {decision}: {rows:#?}"
    );
    a.finish().await;
}

#[tokio::test]
async fn an_approve_leaves_ap1s_row_and_no_transaction_row() {
    decided_rows(Outcome::Approved, "known customer").await;
}

#[tokio::test]
async fn a_reject_leaves_ap1s_row_and_no_transaction_row() {
    decided_rows(Outcome::Rejected, "passport photo unclear").await;
}

// ------------------------------------------------------------ no passport number

/// Every command of the exchange's life writes its row, and no redacted column holds the passport number or the
/// customer id.
#[tokio::test]
async fn no_audit_row_holds_the_passport_number() {
    let a = Audited::start().await;
    let now = whole_second_now();
    let t = a.draft(now).await;
    a.confirm(t.id, later(now, 1)).await.expect("confirm");
    let other = a.draft(later(now, 2)).await;
    a.cancel(other.id, "customer left", later(now, 3))
        .await
        .expect("cancel");

    assert_eq!(
        a.desk()
            .db
            .count("SELECT count(*) FROM audit_events WHERE action LIKE 'transaction.%'")
            .await,
        4,
        "two creates, a confirm and a cancel"
    );
    assert_eq!(
        a.desk()
            .db
            .count(&format!(
                "SELECT count(*) FROM audit_events \
                 WHERE COALESCE(before_redacted::text, '') LIKE '%{PASSPORT_NO}%' \
                    OR COALESCE(after_redacted::text, '') LIKE '%{PASSPORT_NO}%' \
                    OR COALESCE(before_redacted::text, '') LIKE '%{c}%' \
                    OR COALESCE(after_redacted::text, '') LIKE '%{c}%'",
                c = a.customer
            ))
            .await,
        0,
        "§4.7: no customer or passport field"
    );
    a.finish().await;
}

// ------------------------------------------------------------ a replay writes no second row

/// Who is signed in, from the header `x-test-as`: `teller` (FRONT of MAIN, id `TELLER`) or absent.
struct HeaderSessions;

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let signed_in = req.headers().get("x-test-as").is_some();
        Box::pin(async move {
            Sessions {
                owner: None,
                staff: if signed_in {
                    Actor::staff(TELLER, Role::Front, BranchId::new("MAIN"))
                } else {
                    None
                },
            }
        })
    }
}

/// `POST uri` as MAIN's teller with `body` and the `Idempotency-Key` `key`; answers the status and the body.
async fn post(a: &Audited, uri: &str, body: &Value, key: &str) -> (StatusCode, Value) {
    let dir = a.c.scratch.join("uploads");
    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            dir.to_str().expect("a UTF-8 path"),
        )])),
        a.desk().db.pool.clone(),
    );
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::post()
        .uri(uri)
        .insert_header((header::HOST, "xc.test"))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", "http://xc.test"))
        .insert_header(("content-type", "application/json"))
        .insert_header(("x-test-as", "teller"))
        .insert_header(("idempotency-key", key))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
    )
}

/// `post` twice with one key: both answer 200 with the same body.
async fn twice(a: &Audited, uri: &str, body: &Value) -> Value {
    let key = unique("key");
    let first = post(a, uri, body, &key).await;
    assert_eq!(first.0, StatusCode::OK, "{uri}: {}", first.1);
    let again = post(a, uri, body, &key).await;
    assert_eq!(again, first, "{uri}: the same key replays the answer");
    first.1
}

/// The create body of `tx4_routes_test.rs` for `q`, with the passport scan `Desk` seeds for `TELLER`.
fn create_body(q: &Quote, customer: &str) -> Value {
    json!({
        "quoteId": q.id.to_string(),
        "customerId": customer,
        "passport": { "uploadKey": "PASSPORT-MAIN" },
        "payment": {
            "method": "Cash",
            "reference": "R-1",
            "account": "Cash drawer 1",
            "bankFeeThb": "0.00",
            "paidAt": Utc::now().to_rfc3339(),
        },
        "valueDate": "2026-10-06",
        "notes": "",
        "countConfirmed": true,
    })
}

#[actix_web::test]
async fn a_replay_with_the_same_key_writes_no_second_row() {
    let a = Audited::start().await;
    let actions = |rows: &[Value]| -> Vec<String> {
        rows.iter()
            .map(|r| r["action"].as_str().unwrap_or("").to_owned())
            .collect()
    };

    // create, confirm and complete, each twice with one key
    let q = a
        .desk()
        .quote("MAIN", Side::Buy, "100", None, Utc::now())
        .await;
    let made = twice(&a, "/api/transactions", &create_body(&q, &a.customer)).await;
    let id = made["id"]
        .as_str()
        .expect("create answers an id")
        .to_owned();
    twice(&a, &format!("/api/transactions/{id}/confirm"), &json!({})).await;
    twice(&a, &format!("/api/transactions/{id}/complete"), &json!({})).await;
    assert_eq!(
        actions(&audits_of(a.desk(), &id).await),
        [
            "transaction.created",
            "transaction.confirmed",
            "transaction.completed"
        ],
        "one row per command, none for the replays"
    );

    // cancel, twice with one key
    let q = a
        .desk()
        .quote("MAIN", Side::Buy, "100", None, Utc::now())
        .await;
    let made = twice(&a, "/api/transactions", &create_body(&q, &a.customer)).await;
    let other = made["id"]
        .as_str()
        .expect("create answers an id")
        .to_owned();
    twice(
        &a,
        &format!("/api/transactions/{other}/cancel"),
        &json!({ "reason": "customer left" }),
    )
    .await;
    assert_eq!(
        actions(&audits_of(a.desk(), &other).await),
        ["transaction.created", "transaction.cancelled"],
        "one row per command, none for the replays"
    );
    a.finish().await;
}
