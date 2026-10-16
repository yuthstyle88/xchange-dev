//! A single-use coupon, or a version's `max_uses`, is redeemed twice when an expired reservation is re-reserved at
//! completion (`BenefitRedeemer::redeem_in`) while a newer reservation already holds the same coupon or the last use.
//! Design `docs/design/blueprint/marketing-campaigns.md` D-8 (an expired reservation is re-reserved under the lock),
//! D-9 (a single-use coupon is used once) and the reservation rules `BudgetLedger::reserve_in` already enforces
//! (`one_live_reservation_per_coupon`, `max_uses`).
//!
//! Written before the fix by the ticket author (claude/8389e7f9, on the BE lane controller claude/1656900d's order).
//! Do not edit, delete or weaken these tests; add your own in another file.
//!
//! The harness is a copy of `budget_coupon_test.rs`'s (that file is not to be edited). Needs `TEST_DATABASE_URL`
//! naming an existing database whose name ends in `_test`; nothing here skips.

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_kernel::pii::PiiKeys;
use xc_marketing::application::coupon_service::CouponService;
use xc_marketing::application::redeemer::BenefitRedeemer;
use xc_marketing::domain::coupon::benefit_error::BenefitError;
use xc_marketing::domain::coupon::code_hasher::CouponCodeHasher;
use xc_marketing::infrastructure::pg_budget_ledger::{
    BudgetLedger, CompletedExchange, ReserveOutcome, ReserveRequest,
};

/// The organisation OR-1's migration creates.
const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const KEYS_A: &str = "v1:BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";
const MUST_FINISH: Duration = Duration::from_secs(60);

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal")
}

fn in_minutes(m: i64) -> DateTime<Utc> {
    Utc::now() + chrono::Duration::minutes(m)
}

fn hasher(keys: &str) -> CouponCodeHasher {
    CouponCodeHasher::new(&PiiKeys::parse(keys).expect("a key file"))
}

// ── the database ────────────────────────────────────────────────────────────

struct Db {
    pool: PgPool,
    schema: String,
    branch: String,
    user: Uuid,
}

/// One APPROVED, ACTIVE campaign version with one rule and its `campaign_budgets` row.
#[derive(Clone, Copy)]
struct Campaign {
    version: Uuid,
    rule: Uuid,
}

impl Db {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. The MK-3 tests need a real PostgreSQL and do not skip; point it at \
                 an existing database whose name ends in `_test`"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("mk3");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(6)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(dir.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()))
            .run(&pool)
            .await
            .unwrap_or_else(|e| panic!("every migration applies on a fresh database: {e}"));
        let tables: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM information_schema.tables WHERE table_schema = current_schema() \
             AND table_name IN ('coupons','campaign_budgets','benefit_reservations','benefit_redemptions')",
        )
        .fetch_one(&pool)
        .await
        .expect("count M2's tables");
        assert_eq!(tables, 4, "MK-3's M2 created its four tables (design §8)");

        let branch = unique("B").replace('_', "-");
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(&branch)
            .bind(ORG_1)
            .execute(&pool)
            .await
            .expect("a branch");
        let mut db = Self {
            pool,
            schema,
            branch,
            user: Uuid::nil(),
        };
        db.user = db.add_user().await;
        db
    }

    async fn finish(self) {
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.pool)
        .await
        .expect("drop the test schema");
        self.pool.close().await;
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn text(&self, sql: &str) -> String {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn add_user(&self) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, $3, 'ACTIVE', 'test')",
        )
        .bind(id)
        .bind(ORG_1)
        .bind(unique("user"))
        .execute(&self.pool)
        .await
        .expect("a user");
        id
    }

    async fn customer(&self) -> String {
        let id = unique("CX");
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($1, $2::uuid, $1, 'Somchai', 'LIVE', now())",
        )
        .bind(&id)
        .bind(ORG_1)
        .execute(&self.pool)
        .await
        .expect("a customer");
        id
    }

    /// An APPROVED version of a new campaign, ACTIVE now, with `budget_thb`, one PRICE rule (`requires_coupon` as
    /// given) and its `campaign_budgets` row.
    async fn campaign(&self, budget_thb: &str, requires_coupon: bool) -> Campaign {
        let campaign = Uuid::new_v4();
        let version = Uuid::new_v4();
        let rule = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
             VALUES ($1, $2::uuid, $3, $4, now())",
        )
        .bind(campaign)
        .bind(ORG_1)
        .bind(unique("campaign"))
        .bind(self.user)
        .execute(&self.pool)
        .await
        .expect("a campaign");
        sqlx::query(
            "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
             budget_thb, stack_group, priority, version_hash, edited_at, created_by, created_at) \
             VALUES ($1, $2::uuid, $3, 1, 'APPROVED', now() - interval '1 day', now() + interval '30 days', \
             $4, 'g', 1, 'hash', now(), $5, now())",
        )
        .bind(version)
        .bind(ORG_1)
        .bind(campaign)
        .bind(d(budget_thb))
        .bind(self.user)
        .execute(&self.pool)
        .await
        .expect("an APPROVED version");
        sqlx::query(
            "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, \
             calculation, caps, requires_coupon, rule_no) \
             VALUES ($1, $2, 'FEE_WAIVER', 'PRICE', '{}'::jsonb, '{}'::jsonb, '{}'::jsonb, $3, 1)",
        )
        .bind(rule)
        .bind(version)
        .bind(requires_coupon)
        .execute(&self.pool)
        .await
        .expect("a rule");
        sqlx::query("INSERT INTO campaign_budgets (campaign_version_id) VALUES ($1)")
            .bind(version)
            .execute(&self.pool)
            .await
            .expect("the version's budget row (M2)");
        Campaign { version, rule }
    }

    /// An ACTIVE quote of the branch, expiring in 15 minutes.
    async fn quote(&self, customer: Option<&str>) -> Uuid {
        let request = Uuid::new_v4();
        let publication = Uuid::new_v4();
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, object_version_hash, \
             policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES ($1, $2::uuid, 'RATE_PUBLICATION', $3, $4::text, 'test', '1', '{}'::jsonb, 'test', now(), \
             'APPROVED', $5)",
        )
        .bind(request)
        .bind(ORG_1)
        .bind(&self.branch)
        .bind(publication.to_string())
        .bind(unique("pub"))
        .execute(&self.pool)
        .await
        .expect("an approved publication request");
        sqlx::query(
            "INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, \
             approved_request_id, published_by) \
             SELECT $1, $2::uuid, $3, COALESCE((SELECT max(version) FROM rate_publications WHERE branch_id = $3), 0) + 1, \
             now(), $4, 'test'",
        )
        .bind(publication)
        .bind(ORG_1)
        .bind(&self.branch)
        .bind(request)
        .execute(&self.pool)
        .await
        .expect("a publication");
        sqlx::query(
            "INSERT INTO quotes (id, organization_id, branch_id, customer_id, type, channel, status, publication_id, \
             expires_at, created_by, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, $4, 'INDICATIVE', 'DESK', 'ACTIVE', $5, now() + interval '15 minutes', \
             'test', now(), now())",
        )
        .bind(id)
        .bind(ORG_1)
        .bind(&self.branch)
        .bind(customer)
        .bind(publication)
        .execute(&self.pool)
        .await
        .expect("a quote");
        id
    }

    /// A COMPLETED LIVE exchange of the branch (the redemption's `transaction_id`), on a shift of its own (a LIVE
    /// transaction names its shift).
    async fn transaction(&self, customer: &str) -> Uuid {
        let teller = self.add_user().await;
        let shift = unique("shift");
        sqlx::query(
            "UPDATE shifts SET status = 'CLOSED', closed_at = now() WHERE branch_id = $1 AND status = 'OPEN'",
        )
        .bind(&self.branch)
        .execute(&self.pool)
        .await
        .expect("close the branch's open shift");
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', now(), 'system', $3, $4)",
        )
        .bind(&shift)
        .bind(&self.branch)
        .bind(unique("open-req"))
        .bind(teller)
        .execute(&self.pool)
        .await
        .expect("a shift");
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, shift_id, submission_id, \
             payload_hash, customer_id, customer_name, nationality, type, code, amount, rate, principal, fee, \
             bank_fee, received, paid, method, payment_ref, account, staff_id, booking_no, notes, value_date, \
             paid_at, recorded_at, recorded_by, status, completed_at) \
             VALUES ($1, $2::uuid, $3, $4, 'LIVE', $7, $5, 'hash', $6, 'Somchai', 'THA', 'BUY', 'USD', 100, 32.0, \
             3200, 0, 0, 0, 3200, 'Cash', '', 'Cash drawer', 'system', 'WALK-IN', '', current_date, now(), now(), \
             'system', 'COMPLETED', now())",
        )
        .bind(id)
        .bind(ORG_1)
        .bind(unique("DOC"))
        .bind(&self.branch)
        .bind(unique("sub"))
        .bind(customer)
        .bind(&shift)
        .execute(&self.pool)
        .await
        .expect("a completed transaction");
        id
    }

    fn ledger(&self) -> Arc<BudgetLedger> {
        Arc::new(BudgetLedger::new(self.pool.clone()))
    }

    fn coupons(&self) -> CouponService {
        CouponService::new(self.pool.clone(), hasher(KEYS_A))
    }

    fn redeemer(&self) -> BenefitRedeemer {
        BenefitRedeemer::new(self.ledger())
    }

    /// A coupon of `c` (personal when `customer` is set), issued through `CouponService`; returns its id.
    async fn coupon(&self, c: Campaign, customer: Option<&str>) -> Uuid {
        self.coupons()
            .issue(c.rule, 1, customer, in_minutes(60 * 24), self.user)
            .await
            .expect("issue a coupon")
            .remove(0)
            .coupon
            .id
    }

    /// Reserve, committed.
    async fn reserve(&self, request: &ReserveRequest) -> ReserveOutcome {
        let mut tx = self.pool.begin().await.expect("begin");
        let out = self
            .ledger()
            .reserve_in(&mut tx, request)
            .await
            .expect("reserve_in answers");
        tx.commit().await.expect("commit");
        out
    }

    /// The reservation's quote expired: its `expires_at` moves into the past.
    async fn expire(&self, reservation: Uuid) {
        self.exec(&format!(
            "UPDATE benefit_reservations SET expires_at = now() - interval '1 minute' WHERE id = '{reservation}'"
        ))
        .await;
    }

    async fn coupon_status(&self, coupon: Uuid) -> String {
        self.text(&format!("SELECT status FROM coupons WHERE id = '{coupon}'"))
            .await
    }

    /// (consumed_thb, used_count) of the version.
    async fn budget(&self, c: Campaign) -> (Decimal, i32) {
        sqlx::query_as(
            "SELECT consumed_thb, used_count FROM campaign_budgets WHERE campaign_version_id = $1",
        )
        .bind(c.version)
        .fetch_one(&self.pool)
        .await
        .expect("the budget row")
    }

    async fn redemptions(&self, c: Campaign) -> i64 {
        self.count(&format!(
            "SELECT count(*) FROM benefit_redemptions WHERE campaign_version_id = '{}'",
            c.version
        ))
        .await
    }
}

fn request(
    c: Campaign,
    quote: Uuid,
    amount: &str,
    coupon: Option<Uuid>,
    customer: Option<&str>,
) -> ReserveRequest {
    ReserveRequest {
        organization_id: Uuid::parse_str(ORG_1).expect("a uuid"),
        quote_id: quote,
        rule_id: c.rule,
        campaign_version_id: c.version,
        coupon_id: coupon,
        customer_id: customer.map(str::to_owned),
        amount_thb: d(amount),
        expires_at: in_minutes(15),
    }
}

fn reserved(out: &ReserveOutcome) -> Uuid {
    match out {
        ReserveOutcome::Reserved(id) => *id,
        other => panic!("expected Reserved, got {other:?}"),
    }
}

fn exchange(transaction: Uuid, customer: &str) -> CompletedExchange {
    CompletedExchange {
        transaction_id: transaction,
        customer_id: Some(customer.to_owned()),
        referral_attribution_id: None,
    }
}

// ── the double redemption ───────────────────────────────────────────────────

/// Completes `quote`'s exchange as a new transaction of `customer`: `Ok` commits, `Err` rolls back.
async fn complete(db: &Db, quote: Uuid, customer: &str) -> Result<Vec<Uuid>, BenefitError> {
    let txn = db.transaction(customer).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let out = db
        .redeemer()
        .redeem_in(&mut tx, &exchange(txn, customer), quote)
        .await;
    if out.is_ok() {
        tx.commit().await.expect("commit");
    } else {
        tx.rollback().await.expect("rollback");
    }
    out
}

/// Quote A reserves coupon C and expires; quote B reserves C (A is released). A completes first: its re-reserve must
/// refuse `benefitLimitReached`, because B's live reservation holds C. B then redeems C once.
#[tokio::test]
async fn an_expired_reservation_is_not_re_reserved_while_another_live_reservation_holds_its_coupon()
{
    let db = Db::start().await;
    let c = db.campaign("100000.00", true).await;
    let customer = db.customer().await;
    let coupon = db.coupon(c, None).await;
    let quote_a = db.quote(Some(&customer)).await;
    let a = reserved(
        &db.reserve(&request(c, quote_a, "50.00", Some(coupon), Some(&customer)))
            .await,
    );
    db.expire(a).await;
    let quote_b = db.quote(Some(&customer)).await;
    reserved(
        &db.reserve(&request(c, quote_b, "50.00", Some(coupon), Some(&customer)))
            .await,
    );

    let err = complete(&db, quote_a, &customer)
        .await
        .expect_err("B's live reservation holds the coupon");
    assert!(matches!(err, BenefitError::BenefitLimitReached), "{err:?}");

    let ids = complete(&db, quote_b, &customer)
        .await
        .expect("B redeems the coupon it holds");
    assert_eq!(ids.len(), 1);
    assert_eq!(db.redemptions(c).await, 1, "one coupon, one redemption");
    assert_eq!(db.coupon_status(coupon).await, "REDEEMED");
    assert_eq!(db.budget(c).await, (d("50.00"), 1));
    db.finish().await;
}

/// The same race on a version with `max_uses = 1` and no coupon: the expired reservation's re-reserve must refuse
/// `benefitLimitReached` while B's live reservation holds the last use; `used_count` never passes `max_uses`.
#[tokio::test]
async fn an_expired_reservation_is_not_re_reserved_past_max_uses() {
    let db = Db::start().await;
    let c = db.campaign("100000.00", false).await;
    db.exec(&format!(
        "UPDATE campaign_versions SET max_uses = 1 WHERE id = '{}'",
        c.version
    ))
    .await;
    let customer = db.customer().await;
    let quote_a = db.quote(Some(&customer)).await;
    let a = reserved(
        &db.reserve(&request(c, quote_a, "50.00", None, Some(&customer)))
            .await,
    );
    db.expire(a).await;
    let quote_b = db.quote(Some(&customer)).await;
    reserved(
        &db.reserve(&request(c, quote_b, "50.00", None, Some(&customer)))
            .await,
    );

    let err = complete(&db, quote_a, &customer)
        .await
        .expect_err("B's live reservation holds the last use");
    assert!(matches!(err, BenefitError::BenefitLimitReached), "{err:?}");

    complete(&db, quote_b, &customer)
        .await
        .expect("B redeems the last use");
    assert_eq!(db.redemptions(c).await, 1);
    assert_eq!(
        db.budget(c).await,
        (d("50.00"), 1),
        "used_count stays at max_uses"
    );
    db.finish().await;
}

/// Control: with nothing else holding the coupon, an expired reservation is still re-reserved and redeemed (D-8).
#[tokio::test]
async fn an_expired_coupon_reservation_with_no_rival_is_still_re_reserved_and_redeemed() {
    let db = Db::start().await;
    let c = db.campaign("100000.00", true).await;
    let customer = db.customer().await;
    let coupon = db.coupon(c, None).await;
    let quote = db.quote(Some(&customer)).await;
    let r = reserved(
        &db.reserve(&request(c, quote, "50.00", Some(coupon), Some(&customer)))
            .await,
    );
    db.expire(r).await;
    let ids = complete(&db, quote, &customer)
        .await
        .expect("re-reserved and redeemed");
    assert_eq!(ids.len(), 1);
    assert_eq!(db.coupon_status(coupon).await, "REDEEMED");
    assert_eq!(db.budget(c).await, (d("50.00"), 1));
    db.finish().await;
}
