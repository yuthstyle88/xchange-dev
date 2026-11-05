//! Up-front tests for CS-20 (design `docs/design/configurable-shifts.md` §2.10, T9, rows X1/X2): every "until the end
//! of the sale's day" deadline is the end of the sale's business day under the branch's shift plan
//! (`ShiftPlan::business_day_end(value_date)`), not local midnight after the calendar date of `completed_at`. Written
//! by the ticket author claude/35271802 — the implementer must not edit, delete or weaken these.
//!
//! MAIN keeps the migration's defaults (A 06–14, B 14–22, C 22–06, Asia/Bangkok, UTC+7). The sale of the shift-C
//! night of 4 Nov is paid and completed 5 Nov 02:00 Bangkok (4 Nov 19:00 UTC) with `value_date` 4 Nov (CS-8). Its
//! business day ends 5 Nov 06:00 Bangkok = 4 Nov 23:00 UTC. Rows are seeded with SQL, as `reversal_request_test.rs`.
//! Needs `TEST_DATABASE_URL`.

mod quote_common;
mod shift_plan_common;

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use quote_common::TestDb;
use shift_plan_common::clear_plan;
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access;
use xc_auth::domain::access::StandingKind;
use xc_exchange::domain::reversal::reversal::Reversal;
use xc_exchange::domain::transaction::transaction::TransactionId;
use xc_kernel::ids::BranchId;
use xc_loyalty::domain::facts_source::LoyaltyFactsSource;
use xchange::application::workflows::exchange::business_day::BusinessDayEnd;
use xchange::application::workflows::exchange::reversal_approval;
use xchange::application::workflows::exchange::reversal_request::{
    RequestReversal, RequestReversalInput,
};
use xchange::application::workflows::save_step::SaveActor;
use xchange::infrastructure::loyalty::pg_facts_source::PgLoyaltyFactsSource;

/// 5 Nov 02:00 Bangkok: the shift-C sale.
const NIGHT_SALE: &str = "2026-11-04T19:00:00Z";
/// 4 Nov 10:00 Bangkok: a shift-A sale of the same business day.
const DAY_SALE: &str = "2026-11-04T03:00:00Z";
/// 5 Nov 06:00 Bangkok: the end of the 4 Nov business day.
const DAY_END: &str = "2026-11-04T23:00:00Z";

fn at(iso: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(iso)
        .expect("an instant")
        .with_timezone(&Utc)
}

fn nov4() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
}

fn teller() -> SaveActor {
    SaveActor {
        id: "employee-front".to_owned(),
        username: "front".to_owned(),
        employee_id: Some("employee-front".to_owned()),
        standing: StandingKind::Staff(access::Role::Teller),
    }
}

fn code<T: std::fmt::Debug>(got: AppResult<T>) -> String {
    match got {
        Err(AppError::Domain(m)) => m.split(':').next().unwrap_or("").trim().to_owned(),
        other => panic!("expected Err(AppError::Domain(\"<code>: …\")), got {other:?}"),
    }
}

struct Shop {
    db: TestDb,
    requests: RequestReversal,
}

impl Shop {
    async fn start() -> Self {
        let db = TestDb::start().await;
        db.exec(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT 'CU-MAIN', organization_id, 'CU-MAIN', 'Somchai', 'LIVE', now() FROM branches WHERE id = 'MAIN'",
        )
        .await;
        db.exec(
            "WITH u AS (INSERT INTO users (organization_id, name, status, created_by) \
                 SELECT organization_id, 'Teller MAIN', 'ACTIVE', 'test' FROM branches WHERE id = 'MAIN' RETURNING id) \
             INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             SELECT 'SH-MAIN', 'MAIN', 'OPEN', now(), 'owner', 'open-MAIN', u.id FROM u",
        )
        .await;
        let mut registry = ApprovalRegistry::new();
        reversal_approval::register(&mut registry, db.pool.clone());
        let approvals = Arc::new(ApprovalService::new(
            registry,
            Arc::new(PgApprovalRepository::new(db.pool.clone())),
            Arc::new(PgAuditLog::new(db.pool.clone())),
        ));
        let requests = RequestReversal::new(db.pool.clone(), approvals);
        Self { db, requests }
    }

    /// A COMPLETED LIVE exchange at MAIN with `value_date` 4 Nov, paid and completed at `completed_at`.
    async fn sale(&self, completed_at: &str) -> TransactionId {
        let id = uuid::Uuid::new_v4();
        let n = self.db.count("SELECT count(*) FROM transactions").await + 1;
        let org = "(SELECT organization_id FROM branches WHERE id = 'MAIN')";
        self.db
            .exec(&format!(
                "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, shift_id, submission_id, \
                    payload_hash, customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, \
                    paid_at, recorded_at, recorded_by, status, completed_at) \
                 VALUES ('{id}', {org}, 'XC-20261104-{n:06}', 'MAIN', 'LIVE', 'SH-MAIN', 'sub-{id}', 'h', 'CU-MAIN', \
                    'Somchai', 'THA', 'EMP-001', 'WALK-IN', '', '2026-11-04', '{completed_at}', '{completed_at}', \
                    'owner', 'COMPLETED', '{completed_at}')"
            ))
            .await;
        self.db
            .exec(&format!(
                "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, quantity, \
                    unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
                 VALUES (gen_random_uuid(), {org}, '{id}', 'BUY', 'USD', 100, 1, 32.1, 3220, 10, 3210, 1, now(), now())"
            ))
            .await;
        TransactionId(id)
    }

    async fn request(&self, original: TransactionId, now: &str) -> AppResult<Reversal> {
        let mut tx = self.db.pool.begin().await.unwrap();
        let got = self
            .requests
            .run_in(
                &mut tx,
                RequestReversalInput {
                    branch: BranchId::new("MAIN"),
                    original,
                    reason: "wrong currency".to_owned(),
                    key: uuid::Uuid::new_v4().to_string(),
                },
                &teller(),
                at(now),
            )
            .await;
        tx.commit().await.unwrap();
        got
    }

    async fn day_end(&self) -> DateTime<Utc> {
        let mut tx = self.db.pool.begin().await.unwrap();
        let end = BusinessDayEnd::new(self.db.pool.clone())
            .of_in(&mut tx, &BranchId::new("MAIN"), nov4())
            .await
            .expect("the end of 4 Nov at MAIN");
        tx.rollback().await.unwrap();
        end
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

/// B1: under the defaults the 4 Nov business day ends 5 Nov 06:00 Bangkok.
#[tokio::test]
async fn b1_the_day_ends_at_the_end_of_the_plans_service_span() {
    let shop = Shop::start().await;
    assert_eq!(shop.day_end().await, at(DAY_END));
    shop.finish().await;
}

/// B2: a branch with no live template keeps today's rule, local midnight after the date (5 Nov 00:00 Bangkok).
#[tokio::test]
async fn b2_without_a_plan_the_day_ends_at_local_midnight() {
    let shop = Shop::start().await;
    clear_plan(&shop.db.pool, "MAIN").await;
    assert_eq!(shop.day_end().await, at("2026-11-04T17:00:00Z"));
    shop.finish().await;
}

/// X1: the 02:00 shift-C sale may be reversed until 06:00: a request at 05:59 is in the window, at 06:00 it is not.
#[tokio::test]
async fn x1_the_night_sale_is_reversible_until_six() {
    let shop = Shop::start().await;
    let sale = shop.sale(NIGHT_SALE).await;
    let got = shop.request(sale, "2026-11-04T22:59:00Z").await;
    assert!(
        got.is_ok(),
        "05:59 is inside the 4 Nov business day: {got:?}"
    );
    shop.finish().await;

    let shop = Shop::start().await;
    let sale = shop.sale(NIGHT_SALE).await;
    assert_eq!(
        code(shop.request(sale, DAY_END).await),
        "reversalWindowClosed",
        "06:00 is the end of the 4 Nov business day"
    );
    shop.finish().await;
}

/// X1b: a 4 Nov 10:00 sale is still reversible at 5 Nov 03:00 Bangkok, after midnight but before the day ends.
#[tokio::test]
async fn x1b_a_day_sale_is_reversible_after_midnight_until_the_day_ends() {
    let shop = Shop::start().await;
    let sale = shop.sale(DAY_SALE).await;
    let got = shop.request(sale, "2026-11-04T20:00:00Z").await;
    assert!(
        got.is_ok(),
        "5 Nov 03:00 is still the 4 Nov business day: {got:?}"
    );
    shop.finish().await;
}

/// X2: the reversal of the night sale is dated by the original's `value_date`, 4 Nov, not 5 Nov.
#[tokio::test]
async fn x2_the_reversal_takes_the_originals_business_date() {
    let shop = Shop::start().await;
    let sale = shop.sale(NIGHT_SALE).await;
    let r = shop
        .request(sale, "2026-11-04T21:00:00Z")
        .await
        .expect("a reversal at 04:00");
    assert_eq!(r.business_date, nov4());
    assert_eq!(
        shop.db
            .text(&format!(
                "SELECT business_date::text FROM reversals WHERE id = '{}'",
                r.id.0
            ))
            .await
            .as_deref(),
        Some("2026-11-04")
    );
    shop.finish().await;
}

/// L1: loyalty's `reversible_until` for the night sale is 06:00 the same morning, after the sale; never 5 Nov 00:00,
/// which is before it.
#[tokio::test]
async fn l1_loyalty_reversible_until_is_the_end_of_the_business_day() {
    let shop = Shop::start().await;
    let sale = shop.sale(NIGHT_SALE).await;
    let facts = PgLoyaltyFactsSource::new(shop.db.pool.clone())
        .completed(sale.0)
        .await
        .expect("the completed facts");
    assert_eq!(facts.reversible_until, at(DAY_END));
    assert!(facts.reversible_until > at(NIGHT_SALE));
    shop.finish().await;
}
