//! Ticket #684 (RV-3) — posting an APPROVED reversal at the counter: the opposite posting at the original's frozen
//! cost, the effects, the Broker feed, the reversal document and the mark, in one transaction.
//! Written before the implementation by the up-front test writer (claude/dell-claude-2, on the xchange BE lane
//! controller claude/ad05bf8d's order, 2026-10-12). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/reversal.md` §4.3 (§09's order), §8.1 RV-3, V-2, V-4, V-6, R-1, R-4; ledger
//! `ledger-inventory.md` §4.6 (the port's `reverses`). The effect seam: lane controller's ruling on #684
//! (issuecomment-6095137406) — RV-3 owns the `ExchangeEffects::run_reversed_in` call.
//!
//! It runs on LG-4 #929's ledger (`LedgerCashPostings`): the originals are completed through TX-5's
//! `CompleteExchange` with the real port, so each line's `cost_basis_thb` / `realized_margin_thb` is frozen as on a
//! live counter. The branch is TX-5's `Counter` (MAIN, its open shift with THB 10,000.00 and USD 1,000.00 in the
//! ledger drawer); the reversal is requested and FINANCE-approved through RV-2's own workflow and AP-1.
//!
//! Signatures this test assumes ([required] in ticket §7 unless marked):
//! - `xchange::application::workflows::exchange::reversal_post::PostReversal`:
//!   - `PostReversal::new(pool, files: FileService, postings: Arc<dyn CashPostings>, effects:
//!     Arc<ExchangeEffects>)` — [proposed], as `CompleteExchange::new`;
//!   - `PostReversal::run_in(&self, tx, id: ReversalId, who: &SaveActor, now) -> AppResult<PostedReversal>`, the
//!     answer being the facts its effects were told (`complete.rs`'s `PostedReversal`, XS-0's seam).
//! - A refusal is `Err(AppError::Domain(m))` with `m` starting with its code (§8: `staleVersion`,
//!   `reversalWindowClosed`, `noOpenShift`, `stockInsufficient`), as TX-1..TX-5 answer.
//! - The poster's shift is the branch's open shift at posting time (`ShiftService::active_in`, as TX-5's
//!   completion finds it); it need not be the original's shift.
//! - The effects run in §09's order: the `Redeem` phase (benefit, #560's) before the `Outbox` phase (Loyalty, #559's),
//!   each once, on the caller's transaction (TX-5's precedent for `run_completed_in`).
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips. The originals are completed at "now" and posted seconds later, so
//! every post is inside the branch's business day (R-1) unless a test posts two days later on purpose.

mod quote_common;
mod tx4_common;
mod tx5_common;

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use quote_common::*;
use rust_decimal::Decimal;
use tx4_common::*;
use tx5_common::*;
use uuid::Uuid;
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::reversal::reversal::ReversalId;
use xc_exchange::domain::transaction::transaction::{ExchangeTransaction, TransactionId};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};
use xc_referral::application::feed_writer::{
    CompletedExchange as FeedExchange, ExchangeFeedWriter,
};
use xc_referral::domain::customer_keys::CustomerKeys;
use xc_shift::application::shift::service::ShiftService;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;
use xchange::application::workflows::exchange::complete::{
    BoxFuture, CompleteExchange, EffectPhase, ExchangeCompletedFacts, ExchangeEffect,
    ExchangeEffects, PostedReversal,
};
use xchange::application::workflows::exchange::reversal_approval;
use xchange::application::workflows::exchange::reversal_post::PostReversal;
use xchange::application::workflows::exchange::reversal_request::{
    RequestReversal, RequestReversalInput,
};

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn finance() -> access::Actor {
    access::Actor::new(
        "finance-MAIN",
        "00000000-0000-0000-0000-000000000001",
        vec![access::ScopedRole::new(
            access::Role::Finance,
            Some(BranchId::new("MAIN")),
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

// ------------------------------------------------------------ the effect double

/// A reversal effect: writes one `rv3_effects` row (in run order) on the caller's transaction and records the facts
/// it was told; `refuse` answers an `Err` before writing.
struct Seen {
    name: &'static str,
    phase: EffectPhase,
    refuse: bool,
    told: Arc<Mutex<Vec<PostedReversal>>>,
}

impl ExchangeEffect for Seen {
    fn name(&self) -> &'static str {
        self.name
    }

    fn phase(&self) -> EffectPhase {
        self.phase
    }

    fn on_completed_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _e: &'a ExchangeCompletedFacts,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async { Ok(()) })
    }

    fn on_reversed_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        r: &'a PostedReversal,
    ) -> BoxFuture<'a, AppResult<()>> {
        self.told.lock().unwrap().push(r.clone());
        Box::pin(async move {
            if self.refuse {
                return Err(AppError::Domain(format!("refused: {}", self.name)));
            }
            sqlx::query("INSERT INTO rv3_effects (name, reversal_id) VALUES ($1, $2)")
                .bind(self.name)
                .bind(r.reversal_id)
                .execute(&mut **tx)
                .await
                .map_err(AppError::Database)?;
            Ok(())
        })
    }
}

// ------------------------------------------------------------ the shop

struct Shop {
    c: Counter,
    approvals: Arc<ApprovalService>,
    requests: RequestReversal,
    told: Arc<Mutex<Vec<PostedReversal>>>,
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

impl Shop {
    async fn start() -> Shop {
        let c = Counter::start().await;
        c.desk
            .db
            .exec(
                "CREATE TABLE rv3_effects (seq bigserial PRIMARY KEY, name text NOT NULL, reversal_id uuid NOT NULL)",
            )
            .await;
        let mut registry = ApprovalRegistry::new();
        reversal_approval::register(&mut registry, c.desk.db.pool.clone());
        let approvals = Arc::new(ApprovalService::new(
            registry,
            Arc::new(PgApprovalRepository::new(c.desk.db.pool.clone())),
            Arc::new(PgAuditLog::new(c.desk.db.pool.clone())),
        ));
        let requests = RequestReversal::new(c.desk.db.pool.clone(), approvals.clone());
        Shop {
            c,
            approvals,
            requests,
            told: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn finish(self) {
        self.c.finish().await;
    }

    fn pool(&self) -> &sqlx::PgPool {
        &self.c.desk.db.pool
    }

    /// A LIVE exchange of USD `qty` completed at `now` + 5 s through TX-5 with the ledger's port, and its Broker feed
    /// row, as the completion's feed step writes it (`broker_feed_save_step.rs`).
    async fn completed(
        &self,
        side: Side,
        qty: &str,
        method: PaymentMethod,
        now: DateTime<Utc>,
    ) -> ExchangeTransaction {
        let q = self.c.desk.quote("MAIN", side, qty, None, now).await;
        let t = self.c.live_of(&q, method, true, now).await;
        let service = CompleteExchange::new(
            self.pool().clone(),
            self.c.files.clone(),
            Arc::new(LedgerCashPostings::new()),
            Vec::new(),
            Arc::new(ExchangeEffects::new(Vec::new())),
        );
        let mut tx = self.pool().begin().await.unwrap();
        service
            .run_in(&mut tx, t.id, &teller(), later(now, 5))
            .await
            .unwrap_or_else(|e| panic!("complete the original: {e:?}"));
        let settle = self.settle(t.id).await;
        ExchangeFeedWriter::record_completed(
            &mut tx,
            &FeedExchange {
                transaction_id: t.id.0,
                organization_id: t.organization_id.to_string(),
                branch_id: "MAIN".to_owned(),
                keys: CustomerKeys {
                    customer_sub: None,
                    phone: None,
                },
                value_satang: satang(settle),
                completed_at: later(now, 5),
            },
        )
        .await
        .expect("the feed row of the completion");
        tx.commit().await.unwrap();
        self.c.desk.stored(t.id).await
    }

    /// RV-2's request at `now` + 10 s, approved by MAIN's FINANCE.
    async fn approved(&self, t: &ExchangeTransaction, now: DateTime<Utc>) -> ReversalId {
        let mut tx = self.pool().begin().await.unwrap();
        let r = self
            .requests
            .run_in(
                &mut tx,
                RequestReversalInput {
                    branch: BranchId::new("MAIN"),
                    original: t.id,
                    reason: "the customer changed their mind".to_owned(),
                    key: next("rv3-request"),
                },
                &teller(),
                later(now, 10),
            )
            .await
            .unwrap_or_else(|e| panic!("request the reversal: {e:?}"));
        tx.commit().await.unwrap();
        let mut tx = self.pool().begin().await.unwrap();
        self.approvals
            .decide(
                &mut tx,
                &RequestId::new(r.approval_request_id.to_string()),
                &finance(),
                Outcome::Approved,
                "checked the drawer",
                &next("rv3-decide"),
            )
            .await
            .unwrap_or_else(|e| panic!("FINANCE approves: {}", e.code()));
        tx.commit().await.unwrap();
        r.id
    }

    /// The poster (#2964): the teller of MAIN's open shift, by `users.id`, as a v3 session carries it. A reversal posts
    /// on the poster's own OPEN shift at the branch (`ShiftService::open_of_teller_in`); there is no fallback to "the
    /// branch's shift", so the harness posts as that shift's own teller (the `test` user, `tx5_common`).
    async fn shift_teller(&self) -> xchange::application::workflows::save_step::SaveActor {
        let id: String = if let Some(id) = sqlx::query_scalar::<_, String>(
            "SELECT u.id::text FROM users u JOIN organizations o ON o.id = u.organization_id \
             WHERE o.code = 'XC' AND u.name = 'test' AND u.created_by = 'test' \
             ORDER BY u.created_at, u.id LIMIT 1",
        )
        .fetch_optional(self.pool())
        .await
        .expect("the shift's teller (tx5_common seeds it)")
        {
            id
        } else {
            sqlx::query_scalar(
                "INSERT INTO users (organization_id, name, status, created_by) \
                 SELECT id, 'test', 'PENDING', 'test' FROM organizations WHERE code = 'XC' \
                 RETURNING id::text",
            )
            .fetch_one(self.pool())
            .await
            .expect("the shift's teller")
        };
        xchange::application::workflows::save_step::SaveActor {
            id,
            username: "test".to_owned(),
            employee_id: None,
            standing: xc_auth::domain::access::StandingKind::Staff(
                xc_auth::domain::access::Role::Teller,
            ),
        }
    }

    fn poster(&self, effects: Vec<Arc<dyn ExchangeEffect>>) -> PostReversal {
        PostReversal::new(
            self.pool().clone(),
            self.c.files.clone(),
            Arc::new(LedgerCashPostings::new()),
            Arc::new(ExchangeEffects::new(effects)),
        )
    }

    /// The two watched effects: `benefit` (Redeem) and `loyalty` (Outbox).
    fn watched(&self, refuse_loyalty: bool) -> Vec<Arc<dyn ExchangeEffect>> {
        vec![
            Arc::new(Seen {
                name: "benefit",
                phase: EffectPhase::Redeem,
                refuse: false,
                told: self.told.clone(),
            }),
            Arc::new(Seen {
                name: "loyalty",
                phase: EffectPhase::Outbox,
                refuse: refuse_loyalty,
                told: self.told.clone(),
            }),
        ]
    }

    /// `run_in` in a transaction **committed whatever the answer**: a refusal must have written nothing.
    async fn post_at(&self, id: ReversalId, now: DateTime<Utc>) -> AppResult<PostedReversal> {
        let poster = self.poster(self.watched(false));
        let mut tx = self.pool().begin().await.unwrap();
        let got = poster
            .run_in(&mut tx, id, &self.shift_teller().await, now)
            .await;
        tx.commit().await.unwrap();
        got
    }

    /// `run_in` as the command runs it: committed on `Ok`, rolled back on `Err`.
    async fn post_as_command(
        &self,
        poster: &PostReversal,
        id: ReversalId,
        now: DateTime<Utc>,
    ) -> AppResult<PostedReversal> {
        let mut tx = self.pool().begin().await.unwrap();
        let got = poster
            .run_in(&mut tx, id, &self.shift_teller().await, now)
            .await;
        match &got {
            Ok(_) => tx.commit().await.unwrap(),
            Err(_) => tx.rollback().await.unwrap(),
        }
        got
    }

    // ---------------------------------------------------- reads

    async fn dec(&self, sql: &str) -> Decimal {
        sqlx::query_scalar::<_, Option<Decimal>>(sqlx::AssertSqlSafe(sql.to_owned()))
            .fetch_one(self.pool())
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .unwrap_or_default()
    }

    async fn int(&self, sql: &str) -> i64 {
        self.c.desk.db.count(sql).await
    }

    async fn text(&self, sql: &str) -> String {
        self.c.desk.db.text(sql).await.unwrap_or_default()
    }

    async fn settle(&self, t: TransactionId) -> Decimal {
        self.dec(&format!(
            "SELECT sum(settle_thb) FROM transaction_lines WHERE transaction_id = '{}'",
            t.0
        ))
        .await
    }

    async fn line_cost(&self, t: TransactionId) -> Decimal {
        self.dec(&format!(
            "SELECT sum(cost_basis_thb) FROM transaction_lines WHERE transaction_id = '{}'",
            t.0
        ))
        .await
    }

    async fn line_margin(&self, t: TransactionId) -> Decimal {
        self.dec(&format!(
            "SELECT sum(realized_margin_thb) FROM transaction_lines WHERE transaction_id = '{}'",
            t.0
        ))
        .await
    }

    /// MAIN's USD position (quantity, cost).
    async fn position(&self) -> (Decimal, Decimal) {
        sqlx::query_as(
            "SELECT quantity, cost_thb FROM inventory_positions WHERE branch_id = 'MAIN' AND trim(currency) = 'USD'",
        )
        .fetch_optional(self.pool())
        .await
        .unwrap()
        .unwrap_or_default()
    }

    async fn at(&self, place: CashPlace, currency: &str) -> Decimal {
        let mut tx = self.pool().begin().await.unwrap();
        let b = LedgerCashPostings::new()
            .expected_in(&mut tx, &place)
            .await
            .expect("expected_in");
        tx.rollback().await.unwrap();
        b.get(CurrencyCode::parse(currency).unwrap()).value()
    }

    async fn reversal(&self, id: ReversalId, column: &str) -> String {
        self.text(&format!(
            "SELECT {column}::text FROM reversals WHERE id = '{}'",
            id.0
        ))
        .await
    }

    /// The reversal's postings: `source_type REVERSAL`, `source_id` = its id.
    async fn reversal_postings(&self, id: ReversalId) -> i64 {
        self.int(&format!(
            "SELECT count(*) FROM ledger_postings WHERE source_type = 'REVERSAL' AND source_id = '{}'",
            id.0
        ))
        .await
    }

    /// (movement type, Σ quantity, Σ cost) of the reversal posting's inventory movements.
    async fn movements(&self, id: ReversalId) -> Vec<(String, Decimal, Decimal)> {
        sqlx::query_as(
            "SELECT m.movement_type, sum(m.quantity_signed), sum(m.cost_thb_signed) \
               FROM inventory_movements m JOIN ledger_postings p ON p.id = m.posting_id \
              WHERE p.source_type = 'REVERSAL' AND p.source_id = $1 GROUP BY 1 ORDER BY 1",
        )
        .bind(id.0.to_string())
        .fetch_all(self.pool())
        .await
        .unwrap()
        .into_iter()
        .map(|(t, q, c): (String, Decimal, Decimal)| (t, q.normalize(), c))
        .collect()
    }

    /// Σ quantity of the reversal posting's entries on `kind`'s accounts in `currency`, and the shift those accounts
    /// belong to (DRAWER only).
    async fn moved(&self, id: ReversalId, kind: &str, currency: &str) -> (Decimal, Option<String>) {
        sqlx::query_as(
            "SELECT COALESCE(sum(e.quantity_signed), 0), max(a.shift_id) \
               FROM ledger_entries e JOIN cash_accounts a ON a.id = e.account_id \
               JOIN ledger_postings p ON p.id = e.posting_id \
              WHERE p.source_type = 'REVERSAL' AND p.source_id = $1 AND a.kind = $2 AND trim(e.currency) = $3",
        )
        .bind(id.0.to_string())
        .bind(kind)
        .bind(currency)
        .fetch_one(self.pool())
        .await
        .unwrap()
    }

    /// The original's row, its lines and its receipt revision 1, as JSON text.
    async fn original(&self, t: TransactionId) -> (String, String, String) {
        let id = t.0;
        (
            self.text(&format!(
                "SELECT row_to_json(x)::text FROM transactions x WHERE id = '{id}'"
            ))
            .await,
            self.text(&format!(
                "SELECT string_agg(row_to_json(x)::text, ',' ORDER BY x.id) FROM transaction_lines x \
                 WHERE transaction_id = '{id}'"
            ))
            .await,
            self.text(&format!(
                "SELECT row_to_json(x)::text FROM receipts x WHERE transaction_id = '{id}' AND revision = 1"
            ))
            .await,
        )
    }

    async fn effects(&self) -> Vec<(String, Uuid)> {
        sqlx::query_as("SELECT name, reversal_id FROM rv3_effects ORDER BY seq")
            .fetch_all(self.pool())
            .await
            .unwrap()
    }

    // ---------------------------------------------------- moving the ledger outside the reversal

    /// Another exchange's posting, with no stored line: `q` USD from the customer into MAIN's drawer for THB `thb`
    /// (an acquisition at `thb`, which moves the average).
    async fn bought_elsewhere(&self, shift: &str, q: &str, thb: &str) {
        let source = SourceRef {
            source_type: SourceType::Exchange,
            source_id: next("rv3-other"),
            source_key: "complete".to_owned(),
        };
        post(
            self.pool(),
            vec![
                request(
                    CashPlace::Customer,
                    CashPlace::Drawer(shift.to_owned()),
                    "USD",
                    q,
                    &source,
                ),
                request(CashPlace::Bank, CashPlace::Customer, "THB", thb, &source),
            ],
        )
        .await;
    }

    /// `q` USD out of `shift`'s drawer to the bank, at the average (a TRANSFER).
    async fn out_to_bank(&self, shift: &str, q: &str) {
        let source = SourceRef {
            source_type: SourceType::Transfer,
            source_id: next("rv3-out"),
            source_key: "dispatch".to_owned(),
        };
        post(
            self.pool(),
            vec![request(
                CashPlace::Drawer(shift.to_owned()),
                CashPlace::Bank,
                "USD",
                q,
                &source,
            )],
        )
        .await;
    }
}

fn request(
    from: CashPlace,
    to: CashPlace,
    currency: &str,
    q: &str,
    source: &SourceRef,
) -> PostingRequest {
    PostingRequest {
        from,
        to,
        currency: CurrencyCode::parse(currency).unwrap(),
        quantity: ForeignAmount::from_decimal(d(q)),
        source: source.clone(),
        line: None,
        reverses: None,
    }
}

async fn post(pool: &sqlx::PgPool, reqs: Vec<PostingRequest>) {
    let mut tx = pool.begin().await.unwrap();
    LedgerCashPostings::new()
        .post_in(&mut tx, &reqs, None)
        .await
        .unwrap_or_else(|e| panic!("a posting outside the reversal: {e:?}"));
    tx.commit().await.unwrap();
}

fn satang(thb: Decimal) -> i64 {
    use rust_decimal::prelude::ToPrimitive;
    (thb * Decimal::from(100)).to_i64().unwrap()
}

fn whole(d: Decimal) -> Decimal {
    d.normalize()
}

/// 12:00 Asia/Bangkok today as UTC.
fn business_noon() -> DateTime<Utc> {
    use chrono::TimeZone;
    use chrono_tz::Asia::Bangkok;

    let today = Utc::now().with_timezone(&Bangkok).date_naive();
    Bangkok
        .from_local_datetime(&today.and_hms_opt(12, 0, 0).expect("12:00:00 is valid"))
        .single()
        .expect("12:00 in Asia/Bangkok is unambiguous")
        .with_timezone(&Utc)
}

// ============================================================ a reversed BUY

/// §8.1: a reversed BUY returns the THB through the original's place (cash ⇒ the drawer) and takes the foreign back
/// out at the original line's `cost_basis_thb` — **after another exchange has moved the average**; its
/// `cost_difference_thb` is 0 (the position is not emptied).
#[tokio::test]
async fn a_reversed_cash_buy_returns_the_thb_to_the_drawer_and_disposes_at_the_original_cost() {
    let s = Shop::start().await;
    let now = business_noon();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let cost = s.line_cost(t.id).await;
    let settle = s.settle(t.id).await;
    let id = s.approved(&t, now).await;
    // another exchange moves the average away from the original's unit cost
    s.bought_elsewhere(&s.c.shift, "200", "6600.00").await;
    let (q_before, cost_before) = s.position().await;
    let thb_before = s.at(CashPlace::Drawer(s.c.shift.clone()), "THB").await;

    s.post_at(id, later(now, 20))
        .await
        .expect("an APPROVED reversal posts");

    assert_eq!(s.reversal_postings(id).await, 1, "one REVERSAL posting");
    assert_eq!(
        s.movements(id).await,
        vec![("DISPOSE".to_owned(), d("-100"), -cost)],
        "the USD 100 leave at the original line's cost_basis_thb, not at today's average"
    );
    assert_eq!(
        s.position().await,
        (q_before - d("100"), cost_before - cost),
        "the position gives back exactly what the BUY added"
    );
    assert_eq!(
        s.at(CashPlace::Drawer(s.c.shift.clone()), "THB").await,
        thb_before + settle,
        "R-4: the customer's THB come back into the drawer the original paid from"
    );
    assert_eq!(
        s.moved(id, "BANK", "THB").await.0,
        Decimal::ZERO,
        "nothing through the bank for a cash exchange"
    );
    assert_eq!(
        whole(d(&s.reversal(id, "cost_difference_thb").await)),
        Decimal::ZERO,
        "cost_difference_thb is 0 when the position is not emptied"
    );
    s.finish().await;
}

/// §8.1 / R-4: a non-cash BUY's THB go back through `Bank`, never the drawer.
#[tokio::test]
async fn a_reversed_transfer_buy_returns_the_thb_through_the_bank() {
    let s = Shop::start().await;
    let now = business_noon();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::BankTransfer, now)
        .await;
    let settle = s.settle(t.id).await;
    let id = s.approved(&t, now).await;
    let thb_before = s.at(CashPlace::Drawer(s.c.shift.clone()), "THB").await;

    s.post_at(id, later(now, 20))
        .await
        .expect("an APPROVED reversal posts");

    assert_eq!(
        s.moved(id, "BANK", "THB").await.0,
        settle,
        "the THB come back through the bank, as they were paid"
    );
    assert_eq!(
        s.at(CashPlace::Drawer(s.c.shift.clone()), "THB").await,
        thb_before,
        "the drawer's THB do not move"
    );
    s.finish().await;
}

/// §8.1: `cost_difference_thb` is not 0 when the reversal empties the position — the whole remaining cost leaves,
/// and the difference from the original cost is recorded (`ReversedCost.difference_thb`, ledger §4.6).
#[tokio::test]
async fn a_reversed_buy_that_empties_the_position_records_the_cost_difference() {
    let s = Shop::start().await;
    let now = business_noon();
    let shift = s.c.shift.clone();
    // the counter's USD 1,000 opening leaves, so the BUY's 100 are all the position holds
    s.out_to_bank(&shift, "1000.00").await;
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let cost = s.line_cost(t.id).await;
    let id = s.approved(&t, now).await;
    // 50 more at another price, then 50 out at the average: still 100, at a cost that is no longer the BUY's
    s.bought_elsewhere(&shift, "50", "1700.00").await;
    s.out_to_bank(&shift, "50.00").await;
    let (q_before, cost_before) = s.position().await;
    assert_eq!(whole(q_before), d("100"), "the reversal takes it all");
    assert_ne!(cost_before, cost, "the position's cost has moved");

    s.post_at(id, later(now, 20))
        .await
        .expect("an APPROVED reversal posts");

    assert_eq!(
        s.position().await,
        (Decimal::ZERO, Decimal::ZERO),
        "the position is empty"
    );
    assert_eq!(
        d(&s.reversal(id, "cost_difference_thb").await),
        cost_before - cost,
        "cost_difference_thb = what left (the whole remaining cost) − the original cost"
    );
    s.finish().await;
}

// ============================================================ a reversed SELL

/// §8.1: a reversed SELL brings the foreign back as a lot at the original unit cost and reverses the margin exactly —
/// after another exchange has moved the average.
#[tokio::test]
async fn a_reversed_sell_brings_the_foreign_back_as_a_lot_at_the_original_unit_cost_and_reverses_the_margin()
 {
    let s = Shop::start().await;
    let now = business_noon();
    let shift = s.c.shift.clone();
    // the position gets a real cost first: the zero-cost opening leaves, 1,000 come in at 32.00
    s.out_to_bank(&shift, "1000.00").await;
    s.bought_elsewhere(&shift, "1000", "32000.00").await;
    let t = s
        .completed(Side::Sell, "100", PaymentMethod::Cash, now)
        .await;
    let cost = s.line_cost(t.id).await;
    let margin = s.line_margin(t.id).await;
    let settle = s.settle(t.id).await;
    assert_ne!(margin, Decimal::ZERO, "the SELL earned a margin");
    let id = s.approved(&t, now).await;
    // another exchange moves the average to (900 × 32.00 + 100 × 35.00) / 1,000
    s.bought_elsewhere(&shift, "100", "3500.00").await;
    let (q_before, cost_before) = s.position().await;
    let thb_before = s.at(CashPlace::Drawer(shift.clone()), "THB").await;

    s.post_at(id, later(now, 20))
        .await
        .expect("an APPROVED reversal posts");

    assert_eq!(
        s.movements(id).await,
        vec![("ACQUIRE".to_owned(), d("100"), cost)],
        "the USD 100 come back at the SELL line's cost_basis_thb"
    );
    let lots: Vec<(Decimal, Decimal)> = sqlx::query_as(
        "SELECT l.quantity, l.unit_cost_thb FROM inventory_lots l JOIN ledger_postings p ON p.id = l.source_posting_id \
         WHERE p.source_type = 'REVERSAL' AND p.source_id = $1",
    )
    .bind(id.0.to_string())
    .fetch_all(s.pool())
    .await
    .unwrap();
    assert_eq!(
        lots.into_iter()
            .map(|(q, u)| (q.normalize(), u.normalize()))
            .collect::<Vec<_>>(),
        vec![(d("100"), (cost / d("100")).normalize())],
        "one lot at the original unit cost, whatever the average is now"
    );
    assert_eq!(
        s.position().await,
        (q_before + d("100"), cost_before + cost)
    );
    assert_eq!(
        d(&s.reversal(id, "realized_margin_reversed_thb").await),
        -margin,
        "the margin is reversed exactly"
    );
    assert_eq!(
        whole(d(&s.reversal(id, "cost_difference_thb").await)),
        Decimal::ZERO
    );
    assert_eq!(
        s.at(CashPlace::Drawer(shift), "THB").await,
        thb_before - settle,
        "R-4: the customer's THB go back out of the drawer they were paid into"
    );
    s.finish().await;
}

// ============================================================ the poster's shift

/// §8.1: cash moves on the poster's own shift — the branch's open shift at posting time, not the original's.
#[tokio::test]
async fn cash_moves_on_the_posters_open_shift_not_the_originals() {
    let s = Shop::start().await;
    let now = business_noon();
    let first = s.c.shift.clone();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let id = s.approved(&t, now).await;
    // the original's shift closes; the poster opens the next one, with USD in its drawer
    s.c.desk
        .db
        .exec(&format!(
            "UPDATE shifts SET status = 'CLOSED', closed_at = now() WHERE id = '{first}'"
        ))
        .await;
    let shifts = ShiftService::new(
        Arc::new(PgShiftRepository::new(s.pool().clone())),
        BranchTx::new(Transactor::new(s.pool().clone())),
    );
    let _ = &shifts;
    let second = ca10_seed_open(
        s.pool(),
        "MAIN",
        unique("rv3-open").replace('_', "-"),
        &[("USD", "500.00")],
        later(now, 12),
    )
    .await;
    lg4_fund(
        s.pool(),
        CashPlace::Bank,
        CashPlace::Drawer(second.clone()),
        "USD",
        "500.00",
    )
    .await;
    let first_usd = s.at(CashPlace::Drawer(first.clone()), "USD").await;

    s.post_at(id, later(now, 20))
        .await
        .expect("an APPROVED reversal posts");

    assert_eq!(
        s.moved(id, "DRAWER", "USD").await,
        (d("-100"), Some(second.clone())),
        "the USD leave the poster's drawer"
    );
    assert_eq!(
        s.reversal(id, "shift_id").await,
        second,
        "the reversal records the poster's shift"
    );
    assert_eq!(
        s.at(CashPlace::Drawer(first), "USD").await,
        first_usd,
        "the original's drawer is not touched"
    );
    s.finish().await;
}

/// §8: no open shift at the branch ⇒ `noOpenShift`, nothing written.
#[tokio::test]
async fn a_poster_with_no_open_shift_is_refused() {
    let s = Shop::start().await;
    let now = business_noon();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let id = s.approved(&t, now).await;
    s.c.desk
        .db
        .exec(&format!(
            "UPDATE shifts SET status = 'CLOSED', closed_at = now() WHERE id = '{}'",
            s.c.shift
        ))
        .await;

    assert_eq!(code(s.post_at(id, later(now, 20)).await), "noOpenShift");
    assert_eq!(s.reversal(id, "status").await, "APPROVED");
    assert_eq!(s.reversal_postings(id).await, 0);
    s.finish().await;
}

// ============================================================ refusals

/// §8.1: a shortfall ⇒ `stockInsufficient` (no approved `NEGATIVE_STOCK`), and nothing is written.
#[tokio::test]
async fn a_shortfall_in_the_drawer_is_stock_insufficient_and_writes_nothing() {
    let s = Shop::start().await;
    let now = business_noon();
    let shift = s.c.shift.clone();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let id = s.approved(&t, now).await;
    // every USD leaves the drawer: the reversal cannot hand 100 back
    let usd = s.at(CashPlace::Drawer(shift.clone()), "USD").await;
    s.out_to_bank(&shift, &usd.to_string()).await;

    assert_eq!(
        code(s.post_at(id, later(now, 20)).await),
        "stockInsufficient"
    );
    assert_eq!(s.reversal(id, "status").await, "APPROVED");
    assert_eq!(s.reversal_postings(id).await, 0);
    assert_eq!(
        s.int(&format!(
            "SELECT count(*) FROM receipts WHERE transaction_id = '{}' AND revision = 2",
            t.id.0
        ))
        .await,
        0,
        "no reversal document"
    );
    assert!(s.effects().await.is_empty(), "no effect ran");
    s.finish().await;
}

/// R-1 / §8.1: after the original's business day ⇒ `reversalWindowClosed`, and the reversal stays APPROVED.
#[tokio::test]
async fn a_post_after_the_business_day_is_refused() {
    let s = Shop::start().await;
    let now = business_noon();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let id = s.approved(&t, now).await;

    assert_eq!(
        code(s.post_at(id, now + Duration::days(2)).await),
        "reversalWindowClosed"
    );
    assert_eq!(s.reversal(id, "status").await, "APPROVED");
    assert_eq!(s.reversal_postings(id).await, 0);
    s.finish().await;
}

/// §8: only an APPROVED reversal posts — a REQUESTED one is `staleVersion`.
#[tokio::test]
async fn a_reversal_that_is_not_approved_is_stale() {
    let s = Shop::start().await;
    let now = business_noon();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let mut tx = s.pool().begin().await.unwrap();
    let r = s
        .requests
        .run_in(
            &mut tx,
            RequestReversalInput {
                branch: BranchId::new("MAIN"),
                original: t.id,
                reason: "not decided yet".to_owned(),
                key: next("rv3-request"),
            },
            &teller(),
            later(now, 10),
        )
        .await
        .expect("request");
    tx.commit().await.unwrap();

    assert_eq!(code(s.post_at(r.id, later(now, 20)).await), "staleVersion");
    assert_eq!(s.reversal(r.id, "status").await, "REQUESTED");
    assert_eq!(s.reversal_postings(r.id).await, 0);
    s.finish().await;
}

// ============================================================ the document, the mark, the original

/// V-4 / §8.1: the reversal document is receipts revision 2, `kind REVERSAL`, numbered `<document_ref>-R2`, with its
/// own stored HTML; the reversal is marked POSTED with its posting, document, shift and poster.
#[tokio::test]
async fn the_reversal_document_is_revision_2_numbered_r2_and_the_reversal_is_marked_posted() {
    let s = Shop::start().await;
    let now = business_noon();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let document_ref = t.document_ref.clone().expect("a completed document_ref");
    let id = s.approved(&t, now).await;
    let at = later(now, 20);

    let posted = s.post_at(id, at).await.expect("posted");
    assert_eq!(posted.reversal_id, id.0);
    assert_eq!(posted.transaction_id, t.id.0);
    assert_eq!(posted.amount_thb, s.settle(t.id).await);
    assert_eq!(posted.posted_at, at);

    let (receipt, number, kind, file, original_file): (Uuid, String, String, String, String) =
        sqlx::query_as(
            "SELECT r2.id, r2.number, r2.kind, r2.document_ref, r1.document_ref \
               FROM receipts r2 JOIN receipts r1 ON r1.transaction_id = r2.transaction_id AND r1.revision = 1 \
              WHERE r2.transaction_id = $1 AND r2.revision = 2",
        )
        .bind(t.id.0)
        .fetch_one(s.pool())
        .await
        .expect("receipts revision 2");
    assert_eq!(number, format!("{document_ref}-R2"));
    assert_eq!(kind, "REVERSAL");
    assert_ne!(file, original_file, "its own stored HTML");
    assert_eq!(
        s.int(&format!("SELECT count(*) FROM files WHERE key = '{file}'"))
            .await,
        1,
        "the document's HTML is a stored file"
    );

    assert_eq!(s.reversal(id, "status").await, "POSTED");
    assert_eq!(s.reversal(id, "receipt_id").await, receipt.to_string());
    assert_eq!(
        s.reversal(id, "posting_id").await,
        s.text(&format!(
            "SELECT id::text FROM ledger_postings WHERE source_type = 'REVERSAL' AND source_id = '{}' \
             AND source_key = 'post'",
            id.0
        ))
        .await,
        "the posting is source REVERSAL / the reversal id / 'post'"
    );
    assert_eq!(s.reversal(id, "shift_id").await, s.c.shift);
    assert_eq!(s.reversal(id, "posted_by").await, s.shift_teller().await.id);
    assert_eq!(
        s.dec(&format!(
            "SELECT extract(epoch FROM posted_at)::numeric FROM reversals WHERE id = '{}'",
            id.0
        ))
        .await,
        Decimal::from(at.timestamp())
    );
    s.finish().await;
}

/// V-2 / §8.1: the original's `transactions` row, its lines and its receipt (revision 1) are unchanged.
#[tokio::test]
async fn the_originals_row_lines_and_receipt_are_unchanged() {
    let s = Shop::start().await;
    let now = business_noon();
    let t = s
        .completed(Side::Sell, "100", PaymentMethod::Cash, now)
        .await;
    let id = s.approved(&t, now).await;
    let before = s.original(t.id).await;

    s.post_at(id, later(now, 20)).await.expect("posted");

    assert_eq!(
        s.original(t.id).await,
        before,
        "nothing written on the original"
    );
    s.finish().await;
}

// ============================================================ the feed, the effects, a repeat

/// §8.1: `record_reversed` and the Loyalty event each happen once — the feed row is REVERSED with the whole amount in
/// satang; the benefit (Redeem) and Loyalty (Outbox) effects each run once, in that order, told the reversal's facts.
/// A repeat posts once: the second call is `staleVersion` and writes nothing more.
#[tokio::test]
async fn the_feed_and_each_effect_happen_once_and_a_repeat_posts_once() {
    let s = Shop::start().await;
    let now = business_noon();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let settle = s.settle(t.id).await;
    let id = s.approved(&t, now).await;
    let at = later(now, 20);

    s.post_at(id, at).await.expect("posted");

    let (status, reversed, reversed_at): (String, i64, DateTime<Utc>) = sqlx::query_as(
        "SELECT status, reversed_satang, reversed_at FROM broker_exchange_feed WHERE transaction_id = $1",
    )
    .bind(t.id.0)
    .fetch_one(s.pool())
    .await
    .expect("the feed row");
    assert_eq!(status, "REVERSED");
    assert_eq!(reversed, satang(settle), "amount_thb × 100, exact");
    assert_eq!(reversed_at, at);
    assert_eq!(
        s.effects().await,
        vec![("benefit".to_owned(), id.0), ("loyalty".to_owned(), id.0)],
        "each effect once, Redeem before Outbox"
    );
    {
        let told = s.told.lock().unwrap();
        assert_eq!(told.len(), 2);
        for r in told.iter() {
            assert_eq!(r.reversal_id, id.0);
            assert_eq!(r.transaction_id, t.id.0);
            assert_eq!(r.amount_thb, settle);
            assert_eq!(r.posted_at, at);
        }
    }

    // the repeat
    assert_eq!(code(s.post_at(id, later(now, 30)).await), "staleVersion");
    assert_eq!(s.reversal_postings(id).await, 1, "one posting");
    assert_eq!(
        s.int(&format!(
            "SELECT count(*) FROM receipts WHERE transaction_id = '{}' AND revision = 2",
            t.id.0
        ))
        .await,
        1,
        "one reversal document"
    );
    assert_eq!(s.effects().await.len(), 2, "no effect ran again");
    let again: DateTime<Utc> = sqlx::query_scalar(
        "SELECT reversed_at FROM broker_exchange_feed WHERE transaction_id = $1",
    )
    .bind(t.id.0)
    .fetch_one(s.pool())
    .await
    .unwrap();
    assert_eq!(again, at, "the feed is not written again");
    s.finish().await;
}

/// The effect seam ruling (issuecomment-6095137406): the effects run inside the reversal's transaction — a refusing
/// effect rolls the whole reversal back: no posting, no document, no feed change, still APPROVED.
#[tokio::test]
async fn a_refusing_effect_rolls_the_whole_reversal_back() {
    let s = Shop::start().await;
    let now = business_noon();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let id = s.approved(&t, now).await;
    let poster = s.poster(s.watched(true));

    let got = s.post_as_command(&poster, id, later(now, 20)).await;
    assert!(got.is_err(), "the Loyalty effect refuses: {got:?}");

    assert_eq!(s.reversal(id, "status").await, "APPROVED");
    assert_eq!(s.reversal_postings(id).await, 0);
    assert_eq!(
        s.int(&format!(
            "SELECT count(*) FROM receipts WHERE transaction_id = '{}' AND revision = 2",
            t.id.0
        ))
        .await,
        0
    );
    assert_eq!(
        s.text(&format!(
            "SELECT status FROM broker_exchange_feed WHERE transaction_id = '{}'",
            t.id.0
        ))
        .await,
        "COMPLETED"
    );
    assert!(
        s.effects().await.is_empty(),
        "the benefit effect's row went with the transaction"
    );
    assert_eq!(
        s.told.lock().unwrap().len(),
        2,
        "both effects were reached, on the reversal's own transaction"
    );
    s.finish().await;
}

/// CA-10 (#626, #2955): v2's `ShiftService::open` is deleted with the core. The OPEN shift it wrote (opener `test`,
/// the given opening), written straight into the tables in one branch transaction (setup only). Answers its id.
async fn ca10_seed_open(
    pool: &sqlx::PgPool,
    branch: &str,
    request_id: String,
    opening: &[(&str, &str)],
    opened_at: chrono::DateTime<chrono::Utc>,
) -> String {
    let shift = format!("SH-{}", uuid::Uuid::new_v4());
    let (id, b) = (shift.clone(), branch.to_owned());
    let opening: Vec<(String, String)> = opening
        .iter()
        .map(|(c, a)| ((*c).to_owned(), (*a).to_owned()))
        .collect();
    xc_kernel::branch_tx::BranchTx::new(pos_infra::transactions::Transactor::new(pool.clone()))
        .run(&xc_kernel::ids::BranchId::new(branch), move |tx| {
            Box::pin(async move {
                let teller_id: uuid::Uuid = if let Some(id) = sqlx::query_scalar::<_, uuid::Uuid>(
                    "SELECT u.id FROM users u JOIN organizations o ON o.id = u.organization_id \
                     WHERE o.code = 'XC' AND u.name = 'test' AND u.created_by = 'test' \
                     ORDER BY u.created_at, u.id LIMIT 1",
                )
                .fetch_optional(&mut **tx)
                .await?
                {
                    id
                } else {
                    sqlx::query_scalar(
                        "INSERT INTO users (organization_id, name, status, created_by) \
                         SELECT id, 'test', 'PENDING', 'test' FROM organizations WHERE code = 'XC' \
                         RETURNING id",
                    )
                    .fetch_one(&mut **tx)
                    .await?
                };
                sqlx::query(
                    "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, \
                     teller_id) VALUES ($1, $2, 'OPEN', $3, 'test', '', $4, $5)",
                )
                .bind(&id)
                .bind(&b)
                .bind(opened_at)
                .bind(&request_id)
                .bind(teller_id)
                .execute(&mut **tx)
                .await?;
                for (code, amount) in &opening {
                    use std::str::FromStr;
                    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
                    use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
                    use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
                    use xc_kernel::money::{CurrencyCode, ForeignAmount};

                    let parsed =
                        rust_decimal::Decimal::from_str(amount).expect("opening decimal");
                    if parsed <= rust_decimal::Decimal::ZERO {
                        continue;
                    }
                    let quantity = ForeignAmount::from_decimal(parsed);
                    LedgerCashPostings::new()
                        .post_in(
                            &mut *tx,
                            &[PostingRequest {
                                from: CashPlace::Bank,
                                to: CashPlace::Drawer(id.clone()),
                                currency: CurrencyCode::parse(code).expect("opening currency"),
                                quantity,
                                source: SourceRef {
                                    source_type: SourceType::Opening,
                                    source_id: format!("r1-fund:{id}:{code}"),
                                    source_key: "opening".to_owned(),
                                },
                                line: None,
                                reverses: None,
                            }],
                            None,
                        )
                        .await?;
                }
                Ok::<(), pos_error::AppError>(())
            })
        })
        .await
        .unwrap_or_else(|e| panic!("open a shift at {branch}: {e}"));
    shift
}
