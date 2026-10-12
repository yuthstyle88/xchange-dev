//! Ticket #873 (TX-5) — the harness shared by `tx5_complete_test.rs` and `tx5_routes_test.rs`: the posting double
//! (#924's `CashPostings`, ruling Q1), the effect double (§6.2's `ExchangeEffect`, rulings Q3/Q4), and a counter
//! with MAIN's open shift and CONFIRMED LIVE exchanges on it, on top of `tx4_common::Desk`.
//! Written before the implementation by the up-front test writer (claude/07d3765f, on the xchange BE lane
//! controller claude/ad05bf8d's order, 2026-10-11). Do not edit, delete or weaken; add helpers in a file of your
//! own.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use crate::quote_common::*;
use crate::tx4_common::*;
use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{
    ApprovalRequestId, CashPlace, CashPostings, LineCost, PostingId, PostingRequest, PostingResult,
};
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::quote::quote::Quote;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::repository::TransactionStore;
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta, Side};
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::shift::commands::open_shift::OpenShift;
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;
use xchange::application::workflows::exchange::complete::{
    BoxFuture, CompleteExchange, CompletedExchange, EffectPhase, ExchangeCompletedFacts,
    ExchangeEffect, ExchangeEffects, PostedReversal,
};
use xchange::application::workflows::exchange::confirm::ConfirmOutcome;
use xchange::application::workflows::save_step::SaveStep;
use xchange::infrastructure::files::file_service::FileService;

/// Today's sentences: no open shift (`save_common`, as `tx4_create_test.rs`), and the cash count
/// (`device_service.rs` `COUNT_FIRST`).
pub const NO_OPEN_SHIFT: &str = "กรุณาเปิดกะจากหน้าแอดมินก่อน";
pub const COUNT_FIRST: &str = "ยืนยันการนับเงินก่อนบันทึกรายการเงินสด";
pub const MANAGER: &str = "employee-manager";

// ------------------------------------------------------------ the posting double (Q1)

#[derive(Debug, Clone)]
pub struct Posted {
    pub reqs: Vec<PostingRequest>,
    pub allow_negative: Option<ApprovalRequestId>,
}

/// #924's `CashPostings`, recording every call. It writes one row of `tx5_posted` on the caller's transaction,
/// so a rollback is seen; `refuse` answers `stockInsufficient` and writes nothing. Every leg with a `line` gets
/// `cost` and `margin`.
#[derive(Clone)]
pub struct Ledger {
    pub calls: Arc<Mutex<Vec<Posted>>>,
    pub refuse: bool,
    pub cost: Money,
    pub margin: Option<MoneyDelta>,
}

impl Ledger {
    pub fn new(cost: &str, margin: Option<MoneyDelta>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            refuse: false,
            cost: money(cost),
            margin,
        }
    }

    pub fn refusing() -> Self {
        Self {
            refuse: true,
            ..Self::new("0.00", None)
        }
    }

    pub fn calls(&self) -> Vec<Posted> {
        self.calls.lock().unwrap().clone()
    }
}

impl CashPostings for Ledger {
    fn post_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        req: &'a [PostingRequest],
        allow_negative: Option<ApprovalRequestId>,
    ) -> BoxFuture<'a, AppResult<PostingResult>> {
        self.calls.lock().unwrap().push(Posted {
            reqs: req.to_vec(),
            allow_negative: allow_negative.clone(),
        });
        Box::pin(async move {
            if self.refuse {
                return Err(AppError::Domain(
                    "stockInsufficient: the test double refuses".to_owned(),
                ));
            }
            let source = req
                .first()
                .map(|r| r.source.source_id.clone())
                .unwrap_or_default();
            sqlx::query("INSERT INTO tx5_posted (source_id) VALUES ($1)")
                .bind(source)
                .execute(&mut **tx)
                .await
                .map_err(AppError::Database)?;
            Ok(PostingResult {
                posting_id: PostingId(Uuid::new_v4()),
                costs: req
                    .iter()
                    .filter_map(|r| r.line)
                    .map(|line| LineCost {
                        line,
                        cost_basis_thb: self.cost,
                        realized_margin_thb: self.margin,
                    })
                    .collect(),
                reversed: Vec::new(),
            })
        })
    }

    fn expected_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _place: &'a CashPlace,
    ) -> BoxFuture<'a, AppResult<Balances>> {
        Box::pin(async { Ok(Balances::of(&[])) })
    }
}

// ------------------------------------------------------------ the effect double (Q3/Q4)

/// An effect, the facts it saw, and the exchange statuses it saw (`Effect::watching`).
pub type Watched = (
    Arc<dyn ExchangeEffect>,
    Arc<Mutex<Vec<ExchangeCompletedFacts>>>,
    Arc<Mutex<Vec<String>>>,
);

/// §6.2's `ExchangeEffect`: records the facts it is told and the exchange's `status` as the caller's transaction
/// shows it when the effect runs (#873 ruling issuecomment-6108290812, item 3), and writes one row of `tx5_effects`
/// on that transaction; `refuse` answers an `Err` before writing.
pub struct Effect {
    pub name: &'static str,
    pub phase: EffectPhase,
    pub seen: Arc<Mutex<Vec<ExchangeCompletedFacts>>>,
    pub statuses: Arc<Mutex<Vec<String>>>,
    pub refuse: bool,
}

impl Effect {
    pub fn of(
        name: &'static str,
        phase: EffectPhase,
        refuse: bool,
    ) -> (
        Arc<dyn ExchangeEffect>,
        Arc<Mutex<Vec<ExchangeCompletedFacts>>>,
    ) {
        let (effect, seen, _) = Self::watching(name, phase, refuse);
        (effect, seen)
    }

    /// As [`Effect::of`], also answering the exchange statuses the effect saw when it ran.
    pub fn watching(name: &'static str, phase: EffectPhase, refuse: bool) -> Watched {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let statuses = Arc::new(Mutex::new(Vec::new()));
        (
            Arc::new(Self {
                name,
                phase,
                seen: seen.clone(),
                statuses: statuses.clone(),
                refuse,
            }),
            seen,
            statuses,
        )
    }
}

impl ExchangeEffect for Effect {
    fn name(&self) -> &'static str {
        self.name
    }

    fn phase(&self) -> EffectPhase {
        self.phase
    }

    fn on_completed_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        e: &'a ExchangeCompletedFacts,
    ) -> BoxFuture<'a, AppResult<()>> {
        self.seen.lock().unwrap().push(e.clone());
        Box::pin(async move {
            let status: Option<String> =
                sqlx::query_scalar("SELECT status FROM transactions WHERE id = $1")
                    .bind(e.transaction_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;
            self.statuses
                .lock()
                .unwrap()
                .push(status.unwrap_or_default());
            if self.refuse {
                return Err(AppError::Domain(format!("refused: {}", self.name)));
            }
            sqlx::query("INSERT INTO tx5_effects (name, transaction_id) VALUES ($1, $2)")
                .bind(self.name)
                .bind(e.transaction_id)
                .execute(&mut **tx)
                .await
                .map_err(AppError::Database)?;
            Ok(())
        })
    }

    fn on_reversed_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _r: &'a PostedReversal,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

// ------------------------------------------------------------ the counter

pub struct Counter {
    pub desk: Desk,
    pub files: FileService,
    /// MAIN's open shift.
    pub shift: String,
    pub scratch: std::path::PathBuf,
}

impl Counter {
    /// `tx4_common::Desk` plus MAIN's open shift (THB 10,000.00 and USD 1,000.00 at opening) and the two scratch
    /// tables the doubles write.
    pub async fn start() -> Self {
        let desk = Desk::start().await;
        let pool = desk.db.pool.clone();
        desk.db
            .exec("CREATE TABLE tx5_posted (source_id text NOT NULL)")
            .await;
        desk.db
            .exec("CREATE TABLE tx5_effects (name text NOT NULL, transaction_id uuid NOT NULL)")
            .await;
        sqlx::query(
            "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
             SELECT id, 'test', 'PENDING', 'test', 'test' FROM organizations WHERE code = 'XC' \
             ON CONFLICT (legacy_employee_id) DO NOTHING",
        )
        .execute(&pool)
        .await
        .expect("seed the opener");
        let shifts = ShiftService::new(
            Arc::new(PgShiftRepository::new(pool.clone())),
            BranchTx::new(Transactor::new(pool.clone())),
        );
        let shift = shifts
            .open(
                &BranchId::new("MAIN"),
                OpenShift {
                    request_id: unique("open-shift").replace('_', "-"),
                    opening: Balances::of(&[
                        (CurrencyCode::THB, money("10000.00")),
                        (CurrencyCode::parse("USD").unwrap(), money("1000.00")),
                    ]),
                },
                "test",
                whole_second_now(),
            )
            .await
            .unwrap_or_else(|e| panic!("open a shift: {e}"))
            .id;
        // LG-4 (#929, 1:1): the same opening in the ledger's drawer, which the route's postings now read.
        for (code, amount) in [("THB", "10000.00"), ("USD", "1000.00")] {
            lg4_fund(
                &pool,
                xc_cash::domain::posting::CashPlace::Bank,
                xc_cash::domain::posting::CashPlace::Drawer(shift.clone()),
                code,
                amount,
            )
            .await;
        }
        let scratch = std::env::temp_dir().join(unique("xc-tx5"));
        std::fs::create_dir_all(&scratch).unwrap();
        Self {
            files: FileService::new(pool, scratch.join("uploads")),
            desk,
            shift,
            scratch,
        }
    }

    pub async fn finish(self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
        self.desk.db.finish().await;
    }

    /// A CONFIRMED LIVE exchange of 100 USD at MAIN on its open shift, confirmed with no step.
    pub async fn live(
        &self,
        side: Side,
        method: PaymentMethod,
        count_confirmed: bool,
        now: DateTime<Utc>,
    ) -> ExchangeTransaction {
        let q = self.desk.quote("MAIN", side, "100", None, now).await;
        self.live_of(&q, method, count_confirmed, now).await
    }

    pub async fn live_of(
        &self,
        q: &Quote,
        method: PaymentMethod,
        count_confirmed: bool,
        now: DateTime<Utc>,
    ) -> ExchangeTransaction {
        let t = self.live_draft(q, method, count_confirmed, now).await;
        assert_eq!(
            self.desk
                .confirm(t.id, later(now, 1))
                .await
                .expect("confirm"),
            ConfirmOutcome::Confirmed
        );
        self.desk.stored(t.id).await
    }

    /// A LIVE DRAFT of `q` on MAIN's open shift, stored through #670's store (as `Desk::draft` stores a DEMO one).
    pub async fn live_draft(
        &self,
        q: &Quote,
        method: PaymentMethod,
        count_confirmed: bool,
        now: DateTime<Utc>,
    ) -> ExchangeTransaction {
        let line = &q.lines[0];
        let id = TransactionId(Uuid::new_v4());
        let t = ExchangeTransaction {
            id,
            organization_id: q.organization_id,
            branch_id: q.branch_id.clone(),
            mode: Mode::Live,
            status: TransactionStatus::Draft,
            document_ref: None,
            quote_id: Some(q.id),
            booking_id: None,
            shift_id: Some(self.shift.clone()),
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
                method,
                reference: String::new(),
                account: "Cash drawer 1".to_owned(),
                slip_file_key: None,
                amount_thb: line.settle_thb,
                bank_fee_thb: money("0.00"),
                paid_at: now,
            },
            receipts: vec![],
            passport_file_key: format!("PASSPORT-{}", q.branch_id.as_str()),
            kiosk_request_id: None,
            staff_draft_id: None,
            count_confirmed,
        };
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        self.desk
            .store
            .insert_draft_in(&mut tx, &t)
            .await
            .unwrap_or_else(|e| panic!("insert_draft_in: {e}"));
        tx.commit().await.unwrap();
        t
    }

    pub fn service(
        &self,
        ledger: &Ledger,
        steps: Vec<Arc<dyn SaveStep>>,
        effects: Vec<Arc<dyn ExchangeEffect>>,
    ) -> CompleteExchange {
        CompleteExchange::new(
            self.desk.db.pool.clone(),
            self.files.clone(),
            Arc::new(ledger.clone()),
            steps,
            Arc::new(ExchangeEffects::new(effects)),
        )
    }

    /// `run_in` in a transaction that is **committed whatever the answer**.
    pub async fn complete_with(
        &self,
        service: &CompleteExchange,
        id: TransactionId,
        now: DateTime<Utc>,
    ) -> AppResult<CompletedExchange> {
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        let got = service.run_in(&mut tx, id, &teller(), now).await;
        tx.commit().await.unwrap();
        got
    }

    pub async fn complete(
        &self,
        ledger: &Ledger,
        id: TransactionId,
        now: DateTime<Utc>,
    ) -> AppResult<CompletedExchange> {
        let service = self.service(ledger, Vec::new(), Vec::new());
        self.complete_with(&service, id, now).await
    }

    pub async fn status(&self, id: TransactionId) -> String {
        self.desk
            .db
            .text(&format!(
                "SELECT status FROM transactions WHERE id = '{}'",
                id.0
            ))
            .await
            .expect("a transaction row")
    }

    /// Rows a completion of `id` leaves: postings (the double's), receipts, effects, and costs on its lines.
    pub async fn written(&self, id: TransactionId) -> (i64, i64, i64, i64) {
        let db = &self.desk.db;
        (
            db.count(&format!(
                "SELECT count(*) FROM tx5_posted WHERE source_id = '{}'",
                id.0
            ))
            .await,
            db.count(&format!(
                "SELECT count(*) FROM receipts WHERE transaction_id = '{}'",
                id.0
            ))
            .await,
            db.count(&format!(
                "SELECT count(*) FROM tx5_effects WHERE transaction_id = '{}'",
                id.0
            ))
            .await,
            db.count(&format!(
                "SELECT count(*) FROM transaction_lines WHERE transaction_id = '{}' \
                 AND (cost_basis_thb IS NOT NULL OR realized_margin_thb IS NOT NULL)",
                id.0
            ))
            .await,
        )
    }

    /// `"<cost_basis_thb>|<realized_margin_thb>"` of the exchange's line, `NULL` spelled out.
    pub async fn line_costs(&self, id: TransactionId) -> String {
        self.desk
            .db
            .text(&format!(
                "SELECT COALESCE(cost_basis_thb::text, 'NULL') || '|' || \
                        COALESCE(realized_margin_thb::text, 'NULL') \
                 FROM transaction_lines WHERE transaction_id = '{}'",
                id.0
            ))
            .await
            .expect("a line row")
    }
}

pub fn nothing_written() -> (i64, i64, i64, i64) {
    (0, 0, 0, 0)
}

pub fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
}

pub fn source_of(t: &ExchangeTransaction) -> SourceRef {
    SourceRef {
        source_type: SourceType::Exchange,
        source_id: t.id.0.to_string(),
        source_key: "complete".to_owned(),
    }
}

/// The THB leg's quantity: the line's `settle_thb` as a `ForeignAmount` (Q6; `posting.rs`: THB carried with minor
/// unit 2), compared by value.
pub fn thb_quantity(m: Money) -> Decimal {
    Decimal::from_i128_with_scale(m.minor_i128(), 2)
}

pub fn leg(posted: &Posted, currency: CurrencyCode) -> &PostingRequest {
    let legs: Vec<&PostingRequest> = posted
        .reqs
        .iter()
        .filter(|r| r.currency == currency)
        .collect();
    assert_eq!(
        legs.len(),
        1,
        "one {} leg: {:?}",
        currency.as_str(),
        posted.reqs
    );
    legs[0]
}

static LG4_FUND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// LG-4 (#929, lane controller's 1:1 ruling): moves `currency amount` from `from` into `to` through the ledger
/// (`OPENING`), the store the cutover reads; before it, this file seeded `ops_state.safe` / `shift_balances`.
pub async fn lg4_fund(
    pool: &sqlx::PgPool,
    from: xc_cash::domain::posting::CashPlace,
    to: xc_cash::domain::posting::CashPlace,
    currency: &str,
    amount: &str,
) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPostings, PostingRequest};
    let mut tx = pool.begin().await.expect("begin");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from,
                to,
                currency: xc_kernel::money::CurrencyCode::parse(currency).expect("a currency"),
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: format!(
                        "lg4-fund:{currency}:{}",
                        LG4_FUND.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    ),
                    source_key: "fund".to_string(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("fund through the ledger");
    tx.commit().await.expect("commit");
}
