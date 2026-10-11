//! Ticket #672 (TX-4) — the harness shared by `tx4_confirm_cancel_test.rs` and
//! `tx4_approval_test.rs`, on top of TX-1's `quote_common` (a schema per test, publications seeded
//! by SQL).
//! Written before the implementation (claude/14cc24bd, on the owner's order, 2026-10-10). Do not
//! edit, delete or weaken; add helpers in a file of your own.
//!
//! An exchange under test is a DEMO one (no shift): a quote is asked of TX-1's `QuoteService`, and
//! the DRAFT is stored through #670's `TransactionStore::insert_draft_in` with the quote's figures
//! and the branch's seeded passport scan (`PASSPORT-<branch>`, #672 Q2),
//! so that confirm, cancel and the approval are tested apart from create (`tx4_create_test.rs`).
//!
//! Every service call runs in a transaction the harness opens and **commits whatever the answer**,
//! so a refusal that wrote half of its work would be seen in the rows read back.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_approval::application::approval_service::{ApprovalError, ApprovalService};
use xc_approval::domain::request::{Outcome, RequestId};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel};
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::repository::TransactionStore;
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Side};
use xchange::application::workflows::exchange::cancel::CancelTransaction;
use xchange::application::workflows::exchange::confirm::{ConfirmOutcome, ConfirmTransaction};
use xchange::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService,
};
use xchange::application::workflows::save_step::{
    SaveActor, SaveContext, SaveExtras, SaveStep, StepFuture,
};
use xchange::presentation::http::v3::commands::make_approval_service;

use crate::quote_common::*;

/// The second branch of the first organisation.
pub const BR2: &str = "BR2";
/// The DEMO customer of the first organisation.
pub const CUSTOMER: &str = "CU-TX4";
/// The teller who asks for the quote, creates and confirms: the approval's proposer.
pub const TELLER: &str = "employee-teller";

pub fn money(text: &str) -> Money {
    Money::parse(text).unwrap_or_else(|e| panic!("{text} is an amount: {e:?}"))
}

/// The code of a refusal: `Err(AppError::Domain(m))` with `m` starting with its code, as TX-1's
/// `QuoteService` and #670's store answer.
pub fn code<T: std::fmt::Debug>(got: AppResult<T>) -> String {
    match got {
        Err(AppError::Domain(m)) => m
            .split(|c: char| c == ':' || c.is_whitespace())
            .next()
            .unwrap_or("")
            .to_owned(),
        other => panic!("expected Err(AppError::Domain(\"<code>: …\")), got {other:?}"),
    }
}

/// The code of an approval core refusal (`ApprovalError::code`).
pub fn approval_code(got: Result<(), ApprovalError>) -> String {
    match got {
        Err(e) => e.code().to_owned(),
        Ok(()) => panic!("expected the decision to be refused"),
    }
}

/// The teller as the exchange services are told who acts (`SaveActor`, `save_step.rs:26`).
pub fn teller() -> SaveActor {
    SaveActor {
        id: TELLER.to_owned(),
        username: "teller".to_owned(),
        employee_id: Some(TELLER.to_owned()),
        role: Role::Front,
    }
}

/// A staff member as the approval core is told who decides (`access` actor of AP-1).
pub fn staff(id: &str, role: Role, branch: &str) -> Actor {
    Actor::staff(id, role, BranchId::new(branch)).expect("a staff actor")
}

/// A recorded call of a step's `before_in` / `after_in`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepCall {
    pub hook: &'static str,
    pub branch: String,
    pub extras: SaveExtras,
    pub actor: SaveActor,
    pub now: DateTime<Utc>,
    pub had_prior: bool,
}

/// A `SaveStep` that records every call and, when `refuse` is set, refuses `before_in` with it.
pub struct Recorder {
    pub calls: Arc<Mutex<Vec<StepCall>>>,
    pub refuse: Option<&'static str>,
}

impl Recorder {
    pub fn step(refuse: Option<&'static str>) -> (Arc<dyn SaveStep>, Arc<Mutex<Vec<StepCall>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            Arc::new(Self {
                calls: calls.clone(),
                refuse,
            }),
            calls,
        )
    }

    fn record(&self, hook: &'static str, ctx: &SaveContext<'_>) {
        self.calls.lock().unwrap().push(StepCall {
            hook,
            branch: ctx.branch.as_str().to_owned(),
            extras: ctx.extras.clone(),
            actor: ctx.actor.clone(),
            now: ctx.now,
            had_prior: ctx.prior.is_some(),
        });
    }
}

impl SaveStep for Recorder {
    fn before_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
    ) -> StepFuture<'a> {
        self.record("before_in", ctx);
        let refuse = self.refuse;
        Box::pin(async move {
            match refuse {
                Some(sentence) => Err(AppError::Domain(sentence.to_owned())),
                None => Ok(()),
            }
        })
    }

    fn after_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
        _saved: &'a xc_exchange::domain::exchange::entities::transaction::Transaction,
    ) -> StepFuture<'a> {
        self.record("after_in", ctx);
        Box::pin(async { Ok(()) })
    }
}

pub struct Desk {
    pub db: TestDb,
    pub store: PgTransactionStore,
    pub approvals: Arc<ApprovalService>,
}

impl Desk {
    /// MAIN and BR2 of the first organisation, each publishing USD at 32.10 / 32.45, and the DEMO
    /// customer `CUSTOMER`. No branch has a limit.
    pub async fn start() -> Self {
        let db = TestDb::start().await;
        db.branch(BR2).await;
        db.publish("MAIN", &[USD]).await;
        db.publish(BR2, &[USD]).await;
        db.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('{CUSTOMER}', '{ORG_1}', '{CUSTOMER}', 'Test Customer', 'DEMO', now())"
        ))
        .await;
        // #672 Q2: a DRAFT names its passport scan, a row of `files` (`transaction_files` PASSPORT)
        for branch in ["MAIN", BR2] {
            db.exec(&format!(
                "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ('PASSPORT-{branch}', '{branch}', '{TELLER}', 'image/jpeg', 'sha-{branch}', now())"
            ))
            .await;
        }
        Self {
            store: PgTransactionStore::new(db.pool.clone()),
            // [required] the real registry: TX-4 registers EXCHANGE_TRANSACTION in
            // `commands/mod.rs::make_approval_service`, beside RATE_PUBLICATION
            approvals: make_approval_service(db.pool.clone()),
            db,
        }
    }

    /// The approval limit of `branch` (`branch_exchange_limits`, §3.6), written by SQL so these
    /// tests do not lean on G2's writer.
    pub async fn limit(&self, branch: &str, thb: &str) {
        self.db
            .exec(&format!(
                "INSERT INTO branch_exchange_limits (organization_id, branch_id, approval_limit_thb, \
                     updated_by, updated_at) \
                 VALUES ('{ORG_1}', '{branch}', {thb}, 'admin', now()) \
                 ON CONFLICT (organization_id, branch_id) \
                 DO UPDATE SET approval_limit_thb = EXCLUDED.approval_limit_thb"
            ))
            .await;
    }

    /// An ACTIVE desk quote of `quantity` USD at `branch`, asked at `now`.
    pub async fn quote(
        &self,
        branch: &str,
        side: Side,
        quantity: &str,
        fee: Option<&str>,
        now: DateTime<Utc>,
    ) -> Quote {
        let mut tx = self.db.pool.begin().await.unwrap();
        let q = QuoteService::new(self.db.pool.clone())
            .quote_in(
                &mut tx,
                QuoteRequest {
                    branch_id: BranchId::new(branch),
                    customer_id: None,
                    channel: QuoteChannel::Desk,
                    lines: vec![QuoteRequestLine {
                        currency: CurrencyCode::parse("USD").unwrap(),
                        shop_side: side,
                        quantity: ForeignAmount::from_decimal(quantity.parse::<Decimal>().unwrap()),
                        fee_thb: fee.map(money),
                        denomination_group:
                            xc_rate::domain::currency::denomination_groups::GroupCode::all(),
                    }],
                },
                &teller(),
                now,
            )
            .await
            .unwrap_or_else(|e| panic!("quote_in: {e}"));
        tx.commit().await.unwrap();
        q
    }

    /// A DRAFT of `q` for `CUSTOMER`, stored through #670's store with the quote's figures, as
    /// create (§4.4 step 4) would store it.
    pub async fn draft(&self, q: &Quote, now: DateTime<Utc>) -> ExchangeTransaction {
        let line = &q.lines[0];
        let id = TransactionId(Uuid::new_v4());
        let t = ExchangeTransaction {
            id,
            organization_id: q.organization_id,
            branch_id: q.branch_id.clone(),
            mode: Mode::Demo,
            status: TransactionStatus::Draft,
            document_ref: None,
            quote_id: Some(q.id),
            booking_id: None,
            shift_id: None,
            submission_id: format!("sub-{}", id.0),
            payload_hash: "hash".to_owned(),
            customer_id: CUSTOMER.to_owned(),
            customer_name: "Test Customer".to_owned(),
            nationality: "THA".to_owned(),
            staff_id: TELLER.to_owned(),
            booking_no: "WALK-IN".to_owned(),
            notes: String::new(),
            value_date: NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
            recorded_at: now,
            recorded_by: TELLER.to_owned(),
            completed_at: None,
            cancelled_at: None,
            cancel_reason: None,
            approval_request_id: None,
            lines: vec![TransactionLine {
                id: Uuid::new_v4(),
                quote_line_id: Some(line.id),
                shop_side: line.shop_side,
                currency: line.currency,
                quantity: line.quantity,
                unit_size: line.unit_size,
                rate_snapshot: line.final_rate,
                reference_rate: None,
                thb_amount: line.thb_amount,
                fee_thb: line.fee_thb,
                settle_thb: line.settle_thb,
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
                amount_thb: line.settle_thb,
                bank_fee_thb: money("0.00"),
                paid_at: now,
            },
            receipts: vec![],
            // #672 Q2/Q3 (lane controller, issuecomment-6094343896): the passport scan and the save
            // extras are stored at create
            passport_file_key: format!("PASSPORT-{}", q.branch_id.as_str()),
            kiosk_request_id: None,
            staff_draft_id: None,
            count_confirmed: false,
        };
        let mut tx = self.db.pool.begin().await.unwrap();
        self.store
            .insert_draft_in(&mut tx, &t)
            .await
            .unwrap_or_else(|e| panic!("insert_draft_in: {e}"));
        tx.commit().await.unwrap();
        t
    }

    /// A quote and its DRAFT at MAIN: 100 USD bought at 32.10, no fee ⇒ settle 3210.00.
    pub async fn main_draft(&self, now: DateTime<Utc>) -> (Quote, ExchangeTransaction) {
        let q = self.quote("MAIN", Side::Buy, "100", None, now).await;
        let t = self.draft(&q, now).await;
        (q, t)
    }

    pub async fn confirm_with(
        &self,
        steps: Vec<Arc<dyn SaveStep>>,
        id: TransactionId,
        extras: &SaveExtras,
        now: DateTime<Utc>,
    ) -> AppResult<ConfirmOutcome> {
        let service = ConfirmTransaction::new(self.db.pool.clone(), self.approvals.clone(), steps);
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = service.run_in(&mut tx, id, &teller(), extras, now).await;
        tx.commit().await.unwrap();
        got
    }

    /// Confirm with no step registered.
    pub async fn confirm(
        &self,
        id: TransactionId,
        now: DateTime<Utc>,
    ) -> AppResult<ConfirmOutcome> {
        self.confirm_with(Vec::new(), id, &SaveExtras::default(), now)
            .await
    }

    pub async fn cancel(
        &self,
        id: TransactionId,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<ExchangeTransaction> {
        let service = CancelTransaction::new(self.db.pool.clone());
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = service.run_in(&mut tx, id, reason, &teller(), now).await;
        tx.commit().await.unwrap();
        got
    }

    /// AP-1's generic decision (`ApprovalService::decide`), the service
    /// `POST /api/approvals/:id/decide` calls.
    pub async fn decide(
        &self,
        request: &RequestId,
        who: &Actor,
        outcome: Outcome,
        reason: &str,
    ) -> Result<(), ApprovalError> {
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = self
            .approvals
            .decide(&mut tx, request, who, outcome, reason, "decide-key")
            .await;
        tx.commit().await.unwrap();
        got
    }

    /// The stored exchange, read back through #670's store.
    pub async fn stored(&self, id: TransactionId) -> ExchangeTransaction {
        let mut tx = self.db.pool.begin().await.unwrap();
        let t = self
            .store
            .find_in(&mut tx, id)
            .await
            .unwrap_or_else(|e| panic!("find_in: {e}"))
            .unwrap_or_else(|| panic!("{:?} is stored", id));
        tx.commit().await.unwrap();
        t
    }

    /// `<status>|<accepted_by_transaction_id or empty>` of the stored quote.
    pub async fn quote_row(&self, q: &Quote) -> String {
        self.db
            .text(&format!(
                "SELECT status || '|' || COALESCE(accepted_by_transaction_id::text, '') \
                 FROM quotes WHERE id = '{}'",
                q.id
            ))
            .await
            .expect("a quote row")
    }

    /// Rows of `approval_requests` of the exchange's type.
    pub async fn requests(&self) -> i64 {
        self.db
            .count("SELECT count(*) FROM approval_requests WHERE type = 'EXCHANGE_TRANSACTION'")
            .await
    }

    /// `<type>|<object_id>|<branch_id>|<requested_by>|<status>|<idempotency_key>` of a request.
    pub async fn request_row(&self, request: &RequestId) -> String {
        self.db
            .text(&format!(
                "SELECT type || '|' || object_id || '|' || COALESCE(branch_id, '') || '|' || \
                        requested_by || '|' || status || '|' || idempotency_key \
                 FROM approval_requests WHERE id = '{}'",
                request.as_str()
            ))
            .await
            .unwrap_or_else(|| panic!("request {} is a row", request.as_str()))
    }
}

/// `now` a whole second, and `now + s`.
pub fn later(now: DateTime<Utc>, s: i64) -> DateTime<Utc> {
    now + Duration::seconds(s)
}
