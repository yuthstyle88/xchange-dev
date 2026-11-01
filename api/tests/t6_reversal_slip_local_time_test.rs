//! Ticket #5861 (T6-BE): the reversal slip prints its date in the branch's local time, not raw UTC.
//!
//! Asserts the printed date is in the branch's timezone (e.g. `Asia/Bangkok`, UTC+7) formatted as
//! `%d/%m/%Y %H:%M` (matching the sale receipt's format), rather than raw UTC RFC 3339.

mod quote_common;
mod tx4_common;
mod tx5_common;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use tx4_common::*;
use tx5_common::*;
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
    CompleteExchange, EffectPhase, ExchangeCompletedFacts, ExchangeEffect, ExchangeEffects,
    PostedReversal,
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

struct DummyEffect;

impl ExchangeEffect for DummyEffect {
    fn name(&self) -> &'static str {
        "dummy"
    }

    fn phase(&self) -> EffectPhase {
        EffectPhase::Redeem
    }

    fn on_completed_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _e: &'a ExchangeCompletedFacts,
    ) -> xchange::application::workflows::exchange::complete::BoxFuture<'a, AppResult<()>> {
        Box::pin(async { Ok(()) })
    }

    fn on_reversed_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _r: &'a PostedReversal,
    ) -> xchange::application::workflows::exchange::complete::BoxFuture<'a, AppResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

struct Shop {
    c: Counter,
    approvals: Arc<ApprovalService>,
    requests: RequestReversal,
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

fn satang(thb: Decimal) -> i64 {
    (thb * Decimal::from(100)).to_i64().unwrap()
}

impl Shop {
    async fn start() -> Shop {
        let c = Counter::start().await;
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
        }
    }

    fn pool(&self) -> &sqlx::PgPool {
        &self.c.desk.db.pool
    }

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
                    key: next("t6-request"),
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
                &next("t6-decide"),
            )
            .await
            .unwrap_or_else(|e| panic!("FINANCE approves: {}", e.code()));
        tx.commit().await.unwrap();
        r.id
    }

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
        PostReversal::new(
            self.pool().clone(),
            self.c.files.clone(),
            Arc::new(LedgerCashPostings::new()),
            Arc::new(ExchangeEffects::new(vec![Arc::new(DummyEffect)])),
        )
    }

    async fn post_at(&self, id: ReversalId, now: DateTime<Utc>) -> AppResult<PostedReversal> {
        let poster = self.poster();
        let mut tx = self.pool().begin().await.unwrap();
        let got = poster
            .run_in(&mut tx, id, &self.shift_teller().await, now)
            .await;
        tx.commit().await.unwrap();
        got
    }

    async fn settle(&self, t: TransactionId) -> Decimal {
        sqlx::query_scalar::<_, Option<Decimal>>(sqlx::AssertSqlSafe(format!(
            "SELECT sum(settle_thb) FROM transaction_lines WHERE transaction_id = '{}'",
            t.0
        )))
        .fetch_one(self.pool())
        .await
        .unwrap()
        .unwrap_or_default()
    }
}

/// §4 Done criteria:
/// Posts a reversal at a fixed now = 2026-11-01T05:44:25Z for a branch in Asia/Bangkok,
/// reads the stored slip HTML, and asserts it contains `วันที่ 01/11/2026 12:44` and does not contain `+00:00`.
#[tokio::test]
async fn reversal_slip_prints_date_in_branch_local_time() {
    let s = Shop::start().await;
    let completed_at = "2026-11-01T05:40:00Z".parse::<DateTime<Utc>>().unwrap();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, completed_at)
        .await;
    let id = s.approved(&t, completed_at).await;

    let now = "2026-11-01T05:44:25Z".parse::<DateTime<Utc>>().unwrap();
    s.post_at(id, now).await.expect("reversal posted");

    let file: String = sqlx::query_scalar(
        "SELECT r2.document_ref FROM receipts r2 \
         WHERE r2.transaction_id = $1 AND r2.revision = 2",
    )
    .bind(t.id.0)
    .fetch_one(s.pool())
    .await
    .expect("receipt revision 2");

    let stored =
        s.c.files
            .open_in_branch(&file, &BranchId::new("MAIN"))
            .await
            .unwrap()
            .expect("stored slip file");
    let html = String::from_utf8(stored.bytes).expect("valid utf-8 html");

    assert!(
        html.contains("วันที่ 01/11/2026 12:44"),
        "expected slip to contain 'วันที่ 01/11/2026 12:44', but got:\n{html}"
    );
    assert!(
        !html.contains("+00:00"),
        "expected slip not to contain raw UTC offset '+00:00', but got:\n{html}"
    );
}

/// §9 Edge cases:
/// posted 17:30 UTC, branch Asia/Bangkok -> crosses midnight to next day's date: `02/11/2026 00:30`.
#[tokio::test]
async fn reversal_slip_prints_next_day_date_when_crossing_midnight_in_branch_time() {
    let s = Shop::start().await;
    // 17:25 UTC is 00:25 next day in Asia/Bangkok (2026-11-02)
    let completed_at = "2026-11-01T17:25:00Z".parse::<DateTime<Utc>>().unwrap();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, completed_at)
        .await;
    let id = s.approved(&t, completed_at).await;

    // 17:30 UTC is 00:30 next day in Asia/Bangkok (2026-11-02)
    let now = "2026-11-01T17:30:00Z".parse::<DateTime<Utc>>().unwrap();
    s.post_at(id, now).await.expect("reversal posted");

    let file: String = sqlx::query_scalar(
        "SELECT r2.document_ref FROM receipts r2 \
         WHERE r2.transaction_id = $1 AND r2.revision = 2",
    )
    .bind(t.id.0)
    .fetch_one(s.pool())
    .await
    .expect("receipt revision 2");

    let stored =
        s.c.files
            .open_in_branch(&file, &BranchId::new("MAIN"))
            .await
            .unwrap()
            .expect("stored slip file");
    let html = String::from_utf8(stored.bytes).expect("valid utf-8 html");

    assert!(
        html.contains("วันที่ 02/11/2026 00:30"),
        "expected slip to contain 'วันที่ 02/11/2026 00:30', but got:\n{html}"
    );
    assert!(
        !html.contains("+00:00"),
        "expected slip not to contain raw UTC offset '+00:00', but got:\n{html}"
    );
}
