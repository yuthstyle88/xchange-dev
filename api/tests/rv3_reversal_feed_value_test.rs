//! Ticket #2176 — posting a reversal passes the Broker feed row's own `value_satang` (the principal)
//! as `reversed_total`, not `reversal.amount_thb.minor()` (the settled THB).
//!
//! Behaviour table (§9):
//! - F1: feed `value_satang` < settled THB × 100 (SELL with a fee, as production writes it) =>
//!   post succeeds; `reversed_satang = value_satang`; status `REVERSED`.
//! - F2: feed `value_satang` > settled THB × 100 (BUY with a fee) =>
//!   `reversed_satang = value_satang`, not the settled amount.
//! - F3: feed `value_satang` = settled THB × 100 (fee 0) =>
//!   `reversed_satang = value_satang`.
//! - F4: no feed row => step 4 skipped; post succeeds.
//! - F5: a repeat post of the same reversal => posts once (idempotency).

mod quote_common;
mod tx4_common;
mod tx5_common;

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
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
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::reversal::reversal::ReversalId;
use xc_exchange::domain::transaction::transaction::{ExchangeTransaction, TransactionId};
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_referral::application::feed_writer::{
    CompletedExchange as FeedExchange, ExchangeFeedWriter,
};
use xc_referral::domain::customer_keys::CustomerKeys;
use xchange::application::workflows::exchange::complete::{
    BoxFuture, CompleteExchange, EffectPhase, ExchangeCompletedFacts, ExchangeEffect,
    ExchangeEffects, PostedReversal,
};
use xchange::application::workflows::exchange::reversal_approval;
use xchange::application::workflows::exchange::reversal_post::PostReversal;
use xchange::application::workflows::exchange::reversal_request::{
    RequestReversal, RequestReversalInput,
};

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

struct Seen {
    name: &'static str,
    phase: EffectPhase,
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

    async fn completed_with_feed(
        &self,
        side: Side,
        qty: &str,
        method: PaymentMethod,
        feed_delta_satang: Option<i64>,
        now: DateTime<Utc>,
    ) -> (ExchangeTransaction, Option<i64>) {
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
        let settle = self.settle_in(&mut tx, t.id).await;
        let value_satang = feed_delta_satang.map(|delta| satang(settle) + delta);
        if let Some(vs) = value_satang {
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
                    value_satang: vs,
                    completed_at: later(now, 5),
                },
            )
            .await
            .expect("the feed row of the completion");
        }
        tx.commit().await.unwrap();
        (self.c.desk.stored(t.id).await, value_satang)
    }

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
                    key: next("rv3-feed-req"),
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
                &next("rv3-feed-decide"),
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

    fn poster(&self) -> PostReversal {
        let effects: Vec<Arc<dyn ExchangeEffect>> = vec![
            Arc::new(Seen {
                name: "benefit",
                phase: EffectPhase::Redeem,
                told: self.told.clone(),
            }),
            Arc::new(Seen {
                name: "loyalty",
                phase: EffectPhase::Outbox,
                told: self.told.clone(),
            }),
        ];
        PostReversal::new(
            self.pool().clone(),
            self.c.files.clone(),
            Arc::new(LedgerCashPostings::new()),
            Arc::new(ExchangeEffects::new(effects)),
        )
    }

    async fn post_as_command(
        &self,
        id: ReversalId,
        now: DateTime<Utc>,
    ) -> AppResult<PostedReversal> {
        let poster = self.poster();
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

    async fn settle_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        t: TransactionId,
    ) -> Decimal {
        sqlx::query_scalar::<_, Option<Decimal>>(
            "SELECT sum(settle_thb) FROM transaction_lines WHERE transaction_id = $1",
        )
        .bind(t.0)
        .fetch_one(&mut **tx)
        .await
        .unwrap()
        .unwrap_or_default()
    }

    async fn feed_row(
        &self,
        t: TransactionId,
    ) -> Option<(String, i64, i64, Option<DateTime<Utc>>)> {
        sqlx::query_as(
            "SELECT status, value_satang, reversed_satang, reversed_at FROM broker_exchange_feed WHERE transaction_id = $1",
        )
        .bind(t.0)
        .fetch_optional(self.pool())
        .await
        .unwrap()
    }

    async fn reversal_status(&self, id: ReversalId) -> String {
        self.c
            .desk
            .db
            .text(&format!(
                "SELECT status::text FROM reversals WHERE id = '{}'",
                id.0
            ))
            .await
            .unwrap_or_default()
    }

    async fn reversal_postings(&self, id: ReversalId) -> i64 {
        self.c
            .desk
            .db
            .count(&format!(
                "SELECT count(*) FROM ledger_postings WHERE source_type = 'REVERSAL' AND source_id = '{}'",
                id.0
            ))
            .await
    }

    async fn effects(&self) -> Vec<(String, Uuid)> {
        sqlx::query_as("SELECT name, reversal_id FROM rv3_effects ORDER BY seq")
            .fetch_all(self.pool())
            .await
            .unwrap()
    }
}

fn satang(thb: Decimal) -> i64 {
    use rust_decimal::prelude::ToPrimitive;
    (thb * Decimal::from(100)).to_i64().unwrap()
}

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

/// F1: feed `value_satang` < settled THB × 100 (SELL with a fee, as production writes it) =>
/// post succeeds; `reversed_satang = value_satang`; status `REVERSED` (main: `Err(Validation)`, nothing posted).
#[tokio::test]
async fn f1_feed_value_below_settled_thb_sell_with_fee_posts_and_reverses_full_feed_value() {
    let s = Shop::start().await;
    let now = business_noon();
    let (t, Some(expected_value)) = s
        .completed_with_feed(Side::Sell, "100", PaymentMethod::Cash, Some(-5_000), now)
        .await
    else {
        unreachable!()
    };
    let id = s.approved(&t, now).await;
    let at = later(now, 20);

    s.post_as_command(id, at)
        .await
        .expect("F1: reversal of SELL with fee must succeed when value_satang < settle_thb * 100");

    let (status, value_satang, reversed_satang, reversed_at) =
        s.feed_row(t.id).await.expect("feed row exists");
    assert_eq!(status, "REVERSED");
    assert_eq!(value_satang, expected_value);
    assert_eq!(
        reversed_satang, expected_value,
        "F1: reversed_satang must equal the feed row's value_satang"
    );
    assert_eq!(reversed_at, Some(at));
    assert_eq!(s.reversal_status(id).await, "POSTED");
    assert_eq!(s.reversal_postings(id).await, 1);
    s.finish().await;
}

/// F2: feed `value_satang` > settled THB × 100 (BUY with a fee) =>
/// `reversed_satang = value_satang`, not the settled amount (main: a partial reversal).
#[tokio::test]
async fn f2_feed_value_above_settled_thb_buy_with_fee_reverses_full_feed_value() {
    let s = Shop::start().await;
    let now = business_noon();
    let (t, Some(expected_value)) = s
        .completed_with_feed(Side::Buy, "100", PaymentMethod::Cash, Some(5_000), now)
        .await
    else {
        unreachable!()
    };
    let id = s.approved(&t, now).await;
    let at = later(now, 20);

    s.post_as_command(id, at)
        .await
        .expect("F2: reversal of BUY with fee posts");

    let (status, value_satang, reversed_satang, reversed_at) =
        s.feed_row(t.id).await.expect("feed row exists");
    assert_eq!(status, "REVERSED");
    assert_eq!(value_satang, expected_value);
    assert_eq!(
        reversed_satang, expected_value,
        "F2: reversed_satang must equal the full feed value_satang, not the lower settled THB"
    );
    assert_eq!(reversed_at, Some(at));
    assert_eq!(s.reversal_status(id).await, "POSTED");
    s.finish().await;
}

/// F3: feed `value_satang` = settled THB × 100 (fee 0) and F5: a repeat post of the same reversal posts once.
#[tokio::test]
async fn f3_and_f5_feed_value_equal_to_settled_thb_and_repeat_post_is_idempotent() {
    let s = Shop::start().await;
    let now = business_noon();
    let (t, Some(expected_value)) = s
        .completed_with_feed(Side::Sell, "100", PaymentMethod::Cash, Some(-5_000), now)
        .await
    else {
        unreachable!()
    };
    let id = s.approved(&t, now).await;
    let at = later(now, 20);

    // Also verify F3 with a zero-fee transaction in the same test or separately:
    let (t_zero_fee, Some(zero_fee_val)) = s
        .completed_with_feed(Side::Buy, "100", PaymentMethod::Cash, Some(0), now)
        .await
    else {
        unreachable!()
    };
    let id_zero_fee = s.approved(&t_zero_fee, now).await;
    s.post_as_command(id_zero_fee, at)
        .await
        .expect("F3: zero-fee reversal posts");
    let (status_zf, val_zf, rev_zf, rev_at_zf) =
        s.feed_row(t_zero_fee.id).await.expect("F3 feed row");
    assert_eq!(status_zf, "REVERSED");
    assert_eq!(val_zf, zero_fee_val);
    assert_eq!(rev_zf, zero_fee_val, "F3: reversed_satang = value_satang");
    assert_eq!(rev_at_zf, Some(at));

    // F5 on the SELL-with-fee reversal (so F5 also goes red if the fix is reverted):
    s.post_as_command(id, at)
        .await
        .expect("first post succeeds");
    assert_eq!(
        code(s.post_as_command(id, later(now, 30)).await),
        "staleVersion",
        "F5: repeat post is refused as staleVersion"
    );
    assert_eq!(s.reversal_postings(id).await, 1, "F5: posted once");
    assert_eq!(
        s.effects().await.len(),
        4,
        "2 effects per posted reversal, none on repeat"
    );
    let (status, _, reversed_satang, reversed_at) = s.feed_row(t.id).await.expect("feed row");
    assert_eq!(status, "REVERSED");
    assert_eq!(reversed_satang, expected_value);
    assert_eq!(reversed_at, Some(at), "F5: feed not overwritten by repeat");
    s.finish().await;
}

/// F4: no feed row => step 4 skipped; post succeeds (and on a second transaction with a fee-adjusted feed row in the
/// same scenario, the feed row reverses its full `value_satang`).
#[tokio::test]
async fn f4_no_feed_row_skips_step_4_and_post_succeeds() {
    let s = Shop::start().await;
    let now = business_noon();
    let (t_no_feed, None) = s
        .completed_with_feed(Side::Sell, "100", PaymentMethod::Cash, None, now)
        .await
    else {
        unreachable!()
    };
    let id_no_feed = s.approved(&t_no_feed, now).await;
    let at = later(now, 20);

    s.post_as_command(id_no_feed, at)
        .await
        .expect("F4: reversal with no feed row succeeds");
    assert!(
        s.feed_row(t_no_feed.id).await.is_none(),
        "F4: still no feed row when original had none"
    );
    assert_eq!(s.reversal_status(id_no_feed).await, "POSTED");
    assert_eq!(s.reversal_postings(id_no_feed).await, 1);

    // Pair with a fee-bearing feed row in the same test so every test in this file goes red if the fix is reverted:
    let (t_with_feed, Some(expected_value)) = s
        .completed_with_feed(Side::Sell, "50", PaymentMethod::Cash, Some(-2_500), now)
        .await
    else {
        unreachable!()
    };
    let id_with_feed = s.approved(&t_with_feed, now).await;
    s.post_as_command(id_with_feed, at)
        .await
        .expect("paired SELL with fee posts");
    let (status, _, reversed_satang, _) = s.feed_row(t_with_feed.id).await.expect("feed row");
    assert_eq!(status, "REVERSED");
    assert_eq!(reversed_satang, expected_value);

    s.finish().await;
}
