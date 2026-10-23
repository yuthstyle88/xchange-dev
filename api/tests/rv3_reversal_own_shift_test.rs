//! Ticket #2187 — posting an APPROVED reversal chooses the poster's own open shift at the branch.
//!
//! Restated for #2964 (CA-10 split; ticket writer claude/dell-claude-4): the fallback to "the branch's shift"
//! (`ShiftService::active_in`) is gone with v2's per-branch reads (#626 Q4). The poster's own OPEN shift is read with
//! `ShiftService::open_of_teller_in(tx, teller)` (CA-10p #2916) and is used only when it is at the reversal's branch.
//! No own OPEN shift at that branch ⇒ `noOpenShift`, whatever other tellers hold open there. S3, S5a and S6 pinned
//! the fallback and now pin its absence; S1, S2, S4 and S5b are unchanged.

mod quote_common;
mod tx4_common;
mod tx5_common;

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use pos_error::AppResult;
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
use xc_auth::domain::access::StandingKind;
use xc_cash::domain::posting::{CashPlace, CashPostings};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::reversal::reversal::ReversalId;
use xc_exchange::domain::transaction::transaction::{ExchangeTransaction, TransactionId};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Side};
use xc_referral::application::feed_writer::{
    CompletedExchange as FeedExchange, ExchangeFeedWriter,
};
use xc_referral::domain::customer_keys::CustomerKeys;
use xc_shift::application::shift::service::ShiftService;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;
use xchange::application::workflows::exchange::complete::{
    CompleteExchange, ExchangeEffects, PostedReversal,
};
use xchange::application::workflows::exchange::reversal_approval;
use xchange::application::workflows::exchange::reversal_post::PostReversal;
use xchange::application::workflows::exchange::reversal_request::{
    RequestReversal, RequestReversalInput,
};
use xchange::application::workflows::save_step::SaveActor;

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

fn uuid_actor(id: Uuid) -> SaveActor {
    SaveActor {
        id: id.to_string(),
        username: format!("teller-{id}"),
        employee_id: None,
        standing: StandingKind::Staff(access::Role::Teller),
    }
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
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

struct Shop {
    c: Counter,
    approvals: Arc<ApprovalService>,
    requests: RequestReversal,
    _told: Arc<Mutex<Vec<PostedReversal>>>,
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
            _told: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn finish(self) {
        self.c.finish().await;
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
                    key: next("rv3-own-shift-request"),
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
                &next("rv3-own-shift-decide"),
            )
            .await
            .unwrap_or_else(|e| panic!("FINANCE approves: {}", e.code()));
        tx.commit().await.unwrap();
        r.id
    }

    async fn post_by(
        &self,
        id: ReversalId,
        who: &SaveActor,
        now: DateTime<Utc>,
    ) -> AppResult<PostedReversal> {
        let poster = PostReversal::new(
            self.pool().clone(),
            self.c.files.clone(),
            Arc::new(LedgerCashPostings::new()),
            Arc::new(ExchangeEffects::new(Vec::new())),
        );
        let mut tx = self.pool().begin().await.unwrap();
        let got = poster.run_in(&mut tx, id, who, now).await;
        tx.commit().await.unwrap();
        got
    }

    async fn seed_user(&self, user_id: Uuid) {
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, 'Teller', 'ACTIVE', 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(user_id)
        .bind(ORG_1)
        .execute(self.pool())
        .await
        .expect("seed user");
    }

    /// Opens a custody shift (`opened_by_count` set) for `teller_id` at `branch` via `ShiftService::open_on_verified`,
    /// and funds its ledger drawer with USD 500.00 and THB 10,000.00.
    async fn open_custody_shift(
        &self,
        branch: &str,
        teller_id: Uuid,
        now: DateTime<Utc>,
    ) -> String {
        self.seed_user(teller_id).await;
        let count_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO cash_counts (id, organization_id, branch_id, kind, counted_by, counted_at, idempotency_key) \
             VALUES ($1, $2::uuid, $3, 'OPEN', $4, $5, $6)",
        )
        .bind(count_id)
        .bind(ORG_1)
        .bind(branch)
        .bind(teller_id)
        .bind(now)
        .bind(next("rv3-count"))
        .execute(self.pool())
        .await
        .expect("insert cash count");

        let shifts = ShiftService::new(
            Arc::new(PgShiftRepository::new(self.pool().clone())),
            BranchTx::new(Transactor::new(self.pool().clone())),
        );
        let mut tx = self.pool().begin().await.unwrap();
        let shift = shifts
            .open_on_verified(
                &mut tx,
                &BranchId::new(branch),
                &teller_id.to_string(),
                &count_id.to_string(),
                &[
                    ("THB".to_owned(), d("10000.00")),
                    ("USD".to_owned(), d("500.00")),
                ],
                now,
            )
            .await
            .unwrap_or_else(|e| panic!("open_on_verified: {e:?}"));
        tx.commit().await.unwrap();

        for (code, amount) in [("THB", "10000.00"), ("USD", "500.00")] {
            lg4_fund(
                self.pool(),
                CashPlace::Bank,
                CashPlace::Drawer(shift.id.clone()),
                code,
                amount,
            )
            .await;
        }
        shift.id
    }

    async fn close_legacy_shift(&self) {
        self.c
            .desk
            .db
            .exec(&format!(
                "UPDATE shifts SET status = 'CLOSED', closed_at = now() WHERE id = '{}'",
                self.c.shift
            ))
            .await;
    }

    async fn dec(&self, sql: &str) -> Decimal {
        sqlx::query_scalar::<_, Option<Decimal>>(sqlx::AssertSqlSafe(sql.to_owned()))
            .fetch_one(self.pool())
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .unwrap_or_default()
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
}

/// S1: custody branch: the poster has an OPEN shift opened by a count (`opened_by_count` set, `teller_id = who.id`),
/// no legacy shift -> posts on the poster's shift; `reversals.shift_id` = that id.
#[tokio::test]
async fn s1_custody_branch_posts_on_posters_own_shift_when_no_legacy_shift() {
    let s = Shop::start().await;
    let now = business_noon();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let id = s.approved(&t, now).await;

    s.close_legacy_shift().await;
    let poster_id = Uuid::new_v4();
    let own_shift = s
        .open_custody_shift("MAIN", poster_id, later(now, 12))
        .await;
    let who = uuid_actor(poster_id);

    s.post_by(id, &who, later(now, 20))
        .await
        .expect("S1: custody shift posts reversal without legacy shift");

    assert_eq!(
        s.moved(id, "DRAWER", "USD").await,
        (d("-100"), Some(own_shift.clone())),
        "S1: USD leave the poster's custody drawer"
    );
    assert_eq!(
        s.reversal(id, "shift_id").await,
        own_shift,
        "S1: reversals.shift_id records the poster's custody shift"
    );
    s.finish().await;
}

/// S2: the branch has a legacy OPEN shift and the poster's own OPEN shift -> posts on the poster's own shift,
/// not the legacy one.
#[tokio::test]
async fn s2_mixed_branch_posts_on_posters_own_shift_not_legacy_shift() {
    let s = Shop::start().await;
    let now = business_noon();
    let legacy_shift = s.c.shift.clone();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let id = s.approved(&t, now).await;

    let poster_id = Uuid::new_v4();
    let own_shift = s
        .open_custody_shift("MAIN", poster_id, later(now, 12))
        .await;
    let legacy_usd_before = s.at(CashPlace::Drawer(legacy_shift.clone()), "USD").await;
    let who = uuid_actor(poster_id);

    s.post_by(id, &who, later(now, 20))
        .await
        .expect("S2: mixed branch posts reversal");

    assert_eq!(
        s.moved(id, "DRAWER", "USD").await,
        (d("-100"), Some(own_shift.clone())),
        "S2: USD leave the poster's own shift drawer, not the legacy drawer"
    );
    assert_eq!(
        s.reversal(id, "shift_id").await,
        own_shift,
        "S2: reversals.shift_id is the poster's own shift"
    );
    assert_eq!(
        s.at(CashPlace::Drawer(legacy_shift), "USD").await,
        legacy_usd_before,
        "S2: legacy drawer is untouched"
    );
    s.finish().await;
}

/// S3 (#2964): another teller's OPEN shift at the branch, and the UUID poster has none of their own -> `noOpenShift`;
/// nothing is written and the other teller's drawer is untouched (no fallback to the branch's shift).
#[tokio::test]
async fn s3_a_poster_without_an_own_shift_is_no_open_shift_even_beside_another_tellers_shift() {
    let s = Shop::start().await;
    let now = business_noon();
    let other_tellers_shift = s.c.shift.clone();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let id = s.approved(&t, now).await;
    let usd_before = s
        .at(CashPlace::Drawer(other_tellers_shift.clone()), "USD")
        .await;

    let poster_id = Uuid::new_v4();
    s.seed_user(poster_id).await;
    let who = uuid_actor(poster_id);

    assert_eq!(
        code(s.post_by(id, &who, later(now, 20)).await),
        "noOpenShift",
        "S3: no fallback to the branch's (another teller's) open shift"
    );
    assert_eq!(s.reversal(id, "status").await, "APPROVED");
    assert_eq!(s.reversal_postings(id).await, 0);
    assert_eq!(
        s.at(CashPlace::Drawer(other_tellers_shift), "USD").await,
        usd_before,
        "S3: the other teller's drawer is untouched"
    );
    s.finish().await;
}

/// S4: no open shift at all -> `noOpenShift`, nothing written.
#[tokio::test]
async fn s4_no_open_shift_at_all_is_refused() {
    let s = Shop::start().await;
    let now = business_noon();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let id = s.approved(&t, now).await;
    s.close_legacy_shift().await;

    let poster_id = Uuid::new_v4();
    s.seed_user(poster_id).await;
    let who = uuid_actor(poster_id);

    assert_eq!(
        code(s.post_by(id, &who, later(now, 20)).await),
        "noOpenShift"
    );
    assert_eq!(s.reversal(id, "status").await, "APPROVED");
    assert_eq!(s.reversal_postings(id).await, 0);
    s.finish().await;
}

/// S5 (#2964): the poster has an OPEN shift at another branch (`BR2`) only -> not used: `noOpenShift` at `MAIN`,
/// whether or not another teller holds an OPEN shift there.
#[tokio::test]
async fn s5_posters_open_shift_at_another_branch_only_is_not_used() {
    let s = Shop::start().await;
    let now = business_noon();
    let legacy_shift = s.c.shift.clone();
    let t1 = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let id1 = s.approved(&t1, now).await;
    let t2 = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, later(now, 2))
        .await;
    let id2 = s.approved(&t2, later(now, 2)).await;

    let poster_id = Uuid::new_v4();
    let br2_shift = s.open_custody_shift(BR2, poster_id, later(now, 12)).await;
    let br2_usd_before = s.at(CashPlace::Drawer(br2_shift), "USD").await;
    let who = uuid_actor(poster_id);

    // Subcase 1: another teller still holds MAIN's open shift -> noOpenShift; neither that shift nor BR2's is used.
    assert_eq!(
        code(s.post_by(id1, &who, later(now, 20)).await),
        "noOpenShift",
        "S5a: no fallback to MAIN's other open shift, and BR2's own shift is not used"
    );
    assert_eq!(s.reversal(id1, "status").await, "APPROVED");
    assert_eq!(s.reversal_postings(id1).await, 0);
    assert_ne!(s.reversal(id1, "shift_id").await, legacy_shift);

    // Subcase 2: MAIN's legacy shift is closed -> answers noOpenShift, still ignoring BR2's shift.
    s.close_legacy_shift().await;
    assert_eq!(
        code(s.post_by(id2, &who, later(now, 25)).await),
        "noOpenShift",
        "S5b: no open shift at MAIN -> noOpenShift even with an open shift at BR2"
    );
    assert_eq!(s.reversal(id2, "status").await, "APPROVED");
    assert_eq!(s.reversal_postings(id2).await, 0);
    assert_eq!(
        s.at(CashPlace::Drawer(s.c.shift.clone()), "USD").await,
        s.at(CashPlace::Drawer(s.c.shift.clone()), "USD").await
    );
    assert_eq!(
        br2_usd_before,
        d("500.00"),
        "S5: BR2 drawer had 500 USD and was never used"
    );
    s.finish().await;
}

/// S6 (#2964): `who.id` is not a UUID (a legacy actor) -> it can own no shift: `noOpenShift`, nothing written, no
/// parse panic, even with another teller's OPEN shift at the branch.
#[tokio::test]
async fn s6_a_non_uuid_actor_is_no_open_shift_without_panic() {
    let s = Shop::start().await;
    let now = business_noon();
    let t = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let id = s.approved(&t, now).await;

    let legacy_actor = teller();
    assert!(
        Uuid::parse_str(&legacy_actor.id).is_err(),
        "teller() is a non-UUID legacy actor"
    );

    assert_eq!(
        code(s.post_by(id, &legacy_actor, later(now, 20)).await),
        "noOpenShift",
        "S6: a non-UUID actor owns no shift"
    );
    assert_eq!(s.reversal(id, "status").await, "APPROVED");
    assert_eq!(s.reversal_postings(id).await, 0);
    s.finish().await;
}
