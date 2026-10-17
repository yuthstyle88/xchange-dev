//! MK-3 #1395 — the race on the last budget or the last coupon: `BudgetLedger` (reserve / consume / lazy release),
//! `CouponCodeHasher`, `CouponService::issue`, `BenefitRedeemer` (design `docs/design/blueprint/marketing-campaigns.md`
//! D-8, D-9, D-12, §4.1, §4.2, §7; §9.1 row MK-3; ticket #1395 §4 item 1, §9 edge cases; Blueprint v2 §09
//! bp:555-556 *"ห้ามสอง request ใช้สิทธิสุดท้ายพร้อมกัน; transaction failed ต้อง release reserve"*).
//! Written before the implementation by the test writer (claude/dell-claude-3, on the BE lane controller
//! claude/ad05bf8d's order, rule A). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The database is built from every migration in `api/migrations/`, MK-3's M2 (`*_benefit_budgets.sql`, design §8)
//! included: the harness asserts M2's four tables exist. Each test works in a schema of its own. Needs
//! `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.
//!
//! The races are real: two transactions on two connections, released together by a barrier after `BEGIN`, repeated
//! for several rounds, each round on a fresh campaign version. Expiry is simulated by moving a reservation's
//! `expires_at` into the past, so the implementation may compare with the database's `now()` (design §4.1).
//!
//! Not pinned here (ticket §7 "must ask"): the generated codes' length and alphabet.

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Barrier;
use uuid::Uuid;
use xc_kernel::pii::PiiKeys;
use xc_marketing::application::coupon_service::CouponService;
use xc_marketing::application::redeemer::{BenefitRedeemer, PostedReversal};
use xc_marketing::domain::coupon::benefit_error::BenefitError;
use xc_marketing::domain::coupon::code_hasher::CouponCodeHasher;
use xc_marketing::domain::coupon::coupon::CouponStatus;
use xc_marketing::infrastructure::pg_budget_ledger::{
    BudgetLedger, CompletedExchange, ReserveOutcome, ReserveRequest, SkipReason,
};

/// The organisation OR-1's migration creates.
const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const KEYS_A: &str = "v1:BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";
const KEYS_B: &str = "v1:CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk=";
/// Rounds of each race: every round is a fresh version and two fresh quotes.
const ROUNDS: usize = 8;
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
            "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
             customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, \
             recorded_at, recorded_by, status, completed_at) \
             VALUES ($1, $2::uuid, $3, $4, 'LIVE', $7, $5, 'hash', $6, 'Somchai', 'THA', 'system', 'WALK-IN', \
             '', current_date, now(), now(), 'system', 'COMPLETED', now())",
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

    /// A reversal of `transaction` (#552's table; REQUESTED is the status that needs no approval or receipt).
    async fn reversal(&self, transaction: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, reason, \
             amount_thb, business_date, requested_by, requested_at) \
             VALUES ($1, $2::uuid, $3, $4, 'REQUESTED', 'test', 3200, current_date, 'test', now())",
        )
        .bind(id)
        .bind(ORG_1)
        .bind(&self.branch)
        .bind(transaction)
        .execute(&self.pool)
        .await
        .expect("a reversal");
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

    async fn reservation_status(&self, reservation: Uuid) -> String {
        self.text(&format!(
            "SELECT status FROM benefit_reservations WHERE id = '{reservation}'"
        ))
        .await
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

/// Two reserves at once on two connections, released together after `BEGIN`; each commits what it got.
async fn race(db: &Db, a: ReserveRequest, b: ReserveRequest) -> [ReserveOutcome; 2] {
    let gate = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for r in [a, b] {
        let pool = db.pool.clone();
        let ledger = db.ledger();
        let gate = gate.clone();
        handles.push(tokio::spawn(async move {
            let mut tx = pool.begin().await.expect("begin");
            sqlx::query("SELECT 1")
                .execute(&mut *tx)
                .await
                .expect("the transaction holds its connection");
            gate.wait().await;
            let out = ledger
                .reserve_in(&mut tx, &r)
                .await
                .expect("reserve_in answers");
            tx.commit().await.expect("commit");
            out
        }));
    }
    let mut outs = Vec::new();
    for h in handles {
        outs.push(h.await.expect("the racer ran"));
    }
    [outs[0].clone(), outs[1].clone()]
}

// ── §9: the last budget and the last coupon, raced ──────────────────────────

/// §9 edge 1, §9.1: two connections reserving the last budget at once → exactly one RESERVED, the other skipped
/// `budgetExhausted`; the budget never holds more than it has. Repeated for `ROUNDS` fresh versions.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_reserves_of_the_last_budget_give_it_to_exactly_one() {
    let db = Db::start().await;
    for round in 0..ROUNDS {
        let c = db.campaign("100.00", false).await;
        let (qa, qb) = (db.quote(None).await, db.quote(None).await);
        let outs = race(
            &db,
            request(c, qa, "100.00", None, None),
            request(c, qb, "100.00", None, None),
        )
        .await;
        let won = outs
            .iter()
            .filter(|o| matches!(o, ReserveOutcome::Reserved(_)))
            .count();
        let exhausted = outs
            .iter()
            .filter(|o| **o == ReserveOutcome::Skipped(SkipReason::BudgetExhausted))
            .count();
        assert_eq!(
            (won, exhausted),
            (1, 1),
            "round {round}: one reserves, one is budgetExhausted: {outs:?}"
        );
        assert_eq!(
            db.count(&format!(
                "SELECT count(*) FROM benefit_reservations WHERE campaign_version_id = '{}' AND status = 'RESERVED'",
                c.version
            ))
            .await,
            1,
            "round {round}: one live reservation"
        );
    }
    assert_eq!(SkipReason::BudgetExhausted.as_str(), "budgetExhausted");
    db.finish().await;
}

/// §9 edge 2, §9.1: the same coupon presented on two quotes at once → one reserves, the other `couponInvalid`
/// (the budget is ample, so only the coupon decides). Repeated for `ROUNDS` fresh coupons.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_reserves_of_the_last_coupon_give_it_to_exactly_one() {
    let db = Db::start().await;
    for round in 0..ROUNDS {
        let c = db.campaign("100000.00", true).await;
        let coupon = db.coupon(c, None).await;
        let (qa, qb) = (db.quote(None).await, db.quote(None).await);
        let outs = race(
            &db,
            request(c, qa, "50.00", Some(coupon), None),
            request(c, qb, "50.00", Some(coupon), None),
        )
        .await;
        let won = outs
            .iter()
            .filter(|o| matches!(o, ReserveOutcome::Reserved(_)))
            .count();
        let invalid = outs
            .iter()
            .filter(|o| **o == ReserveOutcome::Skipped(SkipReason::CouponInvalid))
            .count();
        assert_eq!(
            (won, invalid),
            (1, 1),
            "round {round}: one reserves the coupon, one is couponInvalid: {outs:?}"
        );
    }
    assert_eq!(SkipReason::CouponInvalid.as_str(), "couponInvalid");
    db.finish().await;
}

// ── expiry: the clock releases ──────────────────────────────────────────────

/// §9.1: an expired reservation stops counting, and the next reserve sets it `RELEASED` (lazily, D-8).
#[tokio::test]
async fn an_expired_reservation_stops_counting_and_the_next_reserve_releases_it() {
    let db = Db::start().await;
    let c = db.campaign("100.00", false).await;
    let first = reserved(
        &db.reserve(&request(c, db.quote(None).await, "100.00", None, None))
            .await,
    );
    assert_eq!(
        db.reserve(&request(c, db.quote(None).await, "1.00", None, None))
            .await,
        ReserveOutcome::Skipped(SkipReason::BudgetExhausted),
        "a live reservation holds the whole budget"
    );
    db.expire(first).await;
    let second = reserved(
        &db.reserve(&request(c, db.quote(None).await, "100.00", None, None))
            .await,
    );
    assert_ne!(first, second);
    assert_eq!(
        db.reservation_status(first).await,
        "RELEASED",
        "released by the next reserve"
    );
    assert_eq!(db.reservation_status(second).await, "RESERVED");
    assert_eq!(db.budget(c).await, (d("0"), 0), "nothing consumed");
    db.finish().await;
}

/// D-8: `release_expired_in` sets the version's expired `RESERVED` rows `RELEASED` and leaves live ones.
#[tokio::test]
async fn release_expired_releases_only_the_expired_reservations_of_the_version() {
    let db = Db::start().await;
    let c = db.campaign("1000.00", false).await;
    let other = db.campaign("1000.00", false).await;
    let old = reserved(
        &db.reserve(&request(c, db.quote(None).await, "10.00", None, None))
            .await,
    );
    let live = reserved(
        &db.reserve(&request(c, db.quote(None).await, "10.00", None, None))
            .await,
    );
    let elsewhere = reserved(
        &db.reserve(&request(other, db.quote(None).await, "10.00", None, None))
            .await,
    );
    db.expire(old).await;
    db.expire(elsewhere).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let n = db
        .ledger()
        .release_expired_in(&mut tx, c.version)
        .await
        .expect("release");
    tx.commit().await.expect("commit");
    assert_eq!(n, 1);
    assert_eq!(db.reservation_status(old).await, "RELEASED");
    assert_eq!(db.reservation_status(live).await, "RESERVED");
    assert_eq!(
        db.reservation_status(elsewhere).await,
        "RESERVED",
        "another version's row is not touched"
    );
    db.finish().await;
}

/// §9.1 (bold row), D-8: a coupon whose quote expired is reservable again: its expired row is released before the
/// insert, so `one_live_reservation_per_coupon` does not block it.
#[tokio::test]
async fn a_coupon_whose_quote_expired_is_reservable_again() {
    let db = Db::start().await;
    let c = db.campaign("100000.00", true).await;
    let coupon = db.coupon(c, None).await;
    let first = reserved(
        &db.reserve(&request(
            c,
            db.quote(None).await,
            "50.00",
            Some(coupon),
            None,
        ))
        .await,
    );
    assert_eq!(
        db.reserve(&request(
            c,
            db.quote(None).await,
            "50.00",
            Some(coupon),
            None
        ))
        .await,
        ReserveOutcome::Skipped(SkipReason::CouponInvalid),
        "a coupon held by a live reservation is couponInvalid"
    );
    db.expire(first).await;
    let second = reserved(
        &db.reserve(&request(
            c,
            db.quote(None).await,
            "50.00",
            Some(coupon),
            None,
        ))
        .await,
    );
    assert_eq!(db.reservation_status(first).await, "RELEASED");
    assert_eq!(db.reservation_status(second).await, "RESERVED");
    assert_eq!(db.coupon_status(coupon).await, "ISSUED");
    db.finish().await;
}

// ── coupons: who may use one ────────────────────────────────────────────────

/// §9.1, D-9: a personal coupon refuses another customer with `couponInvalid` and reserves for its own customer.
#[tokio::test]
async fn a_personal_coupon_refuses_another_customer_with_coupon_invalid() {
    let db = Db::start().await;
    let c = db.campaign("100000.00", true).await;
    let (owner, stranger) = (db.customer().await, db.customer().await);
    let coupon = db.coupon(c, Some(&owner)).await;
    assert_eq!(
        db.reserve(&request(
            c,
            db.quote(Some(&stranger)).await,
            "50.00",
            Some(coupon),
            Some(&stranger)
        ))
        .await,
        ReserveOutcome::Skipped(SkipReason::CouponInvalid),
        "another customer's coupon"
    );
    assert_eq!(
        db.reserve(&request(
            c,
            db.quote(None).await,
            "50.00",
            Some(coupon),
            None
        ))
        .await,
        ReserveOutcome::Skipped(SkipReason::CouponInvalid),
        "a customer-less quote cannot use a personal coupon"
    );
    reserved(
        &db.reserve(&request(
            c,
            db.quote(Some(&owner)).await,
            "50.00",
            Some(coupon),
            Some(&owner),
        ))
        .await,
    );
    db.finish().await;
}

/// §4.5, §8: `couponInvalid` is the one answer for an expired, a used and a void coupon too.
#[tokio::test]
async fn an_expired_used_or_void_coupon_is_coupon_invalid() {
    let db = Db::start().await;
    let c = db.campaign("100000.00", true).await;
    for (what, status, expired) in [
        ("expired", "ISSUED", true),
        ("used", "REDEEMED", false),
        ("void", "VOID", false),
    ] {
        let coupon = db.coupon(c, None).await;
        let expires = if expired {
            "now() - interval '1 minute'"
        } else {
            "expires_at"
        };
        db.exec(&format!(
            "UPDATE coupons SET status = '{status}', expires_at = {expires} WHERE id = '{coupon}'"
        ))
        .await;
        assert_eq!(
            db.reserve(&request(
                c,
                db.quote(None).await,
                "50.00",
                Some(coupon),
                None
            ))
            .await,
            ReserveOutcome::Skipped(SkipReason::CouponInvalid),
            "{what} coupon"
        );
    }
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM benefit_reservations WHERE campaign_version_id = '{}'",
            c.version
        ))
        .await,
        0,
        "a refused coupon reserves nothing"
    );
    db.finish().await;
}

// ── consume and redeem ──────────────────────────────────────────────────────

/// §9.1: consume inserts one redemption (the reservation CONSUMED, budget and use count up, the coupon REDEEMED); a
/// second consume of the same reservation is refused and changes nothing.
#[tokio::test]
async fn consume_inserts_one_redemption_and_a_second_consume_is_refused() {
    let db = Db::start().await;
    let c = db.campaign("1000.00", true).await;
    let customer = db.customer().await;
    let coupon = db.coupon(c, None).await;
    let r = reserved(
        &db.reserve(&request(
            c,
            db.quote(Some(&customer)).await,
            "40.00",
            Some(coupon),
            Some(&customer),
        ))
        .await,
    );
    let txn = db.transaction(&customer).await;
    let ledger = db.ledger();
    let mut tx = db.pool.begin().await.expect("begin");
    let redemption = ledger
        .consume_in(&mut tx, r, &exchange(txn, &customer))
        .await
        .expect("consume");
    tx.commit().await.expect("commit");

    let (rule, reservation, transaction, amount, status, class): (
        Uuid,
        Uuid,
        Uuid,
        Decimal,
        String,
        String,
    ) = sqlx::query_as(
        "SELECT rule_id, reservation_id, transaction_id, amount_thb, status, benefit_class \
         FROM benefit_redemptions WHERE id = $1",
    )
    .bind(redemption)
    .fetch_one(&db.pool)
    .await
    .expect("the redemption row");
    assert_eq!((rule, reservation, transaction), (c.rule, r, txn));
    assert_eq!(
        (amount, status.as_str(), class.as_str()),
        (d("40.00"), "REDEEMED", "PRICE")
    );
    assert_eq!(db.reservation_status(r).await, "CONSUMED");
    assert_eq!(db.budget(c).await, (d("40.00"), 1));
    assert_eq!(
        db.coupon_status(coupon).await,
        CouponStatus::Redeemed.as_str()
    );

    let mut tx = db.pool.begin().await.expect("begin");
    let again = ledger
        .consume_in(&mut tx, r, &exchange(txn, &customer))
        .await;
    assert!(again.is_err(), "a second consume is refused: {again:?}");
    tx.rollback().await.expect("rollback");
    assert_eq!(db.redemptions(c).await, 1, "one redemption");
    assert_eq!(db.budget(c).await, (d("40.00"), 1), "consumed once");
    db.finish().await;
}

/// §9 edge 5: the same completion retried → no second redemption (`reservation_id UNIQUE`,
/// `UNIQUE (rule_id, transaction_id)`); the budget is consumed once.
#[tokio::test]
async fn the_same_completion_retried_redeems_once() {
    let db = Db::start().await;
    let c = db.campaign("1000.00", false).await;
    let customer = db.customer().await;
    let quote = db.quote(Some(&customer)).await;
    reserved(
        &db.reserve(&request(c, quote, "40.00", None, Some(&customer)))
            .await,
    );
    let txn = db.transaction(&customer).await;
    let redeemer = db.redeemer();
    let mut tx = db.pool.begin().await.expect("begin");
    let first = redeemer
        .redeem_in(&mut tx, &exchange(txn, &customer), quote)
        .await
        .expect("redeem");
    tx.commit().await.expect("commit");
    assert_eq!(first.len(), 1);

    let mut tx = db.pool.begin().await.expect("begin");
    match redeemer
        .redeem_in(&mut tx, &exchange(txn, &customer), quote)
        .await
    {
        Ok(ids) => {
            assert!(ids.is_empty(), "a retry redeems nothing new: {ids:?}");
            tx.commit().await.expect("commit");
        }
        Err(_) => tx.rollback().await.expect("rollback"),
    }
    assert_eq!(db.redemptions(c).await, 1);
    assert_eq!(db.budget(c).await, (d("40.00"), 1));
    db.finish().await;
}

/// §9 edge 4, bp:555-556: a completion rolled back leaves no redemption and no consume; its reservation stays
/// RESERVED until the quote expires, then stops counting and is released by the next reserve.
#[tokio::test]
async fn a_rolled_back_completion_keeps_nothing_and_its_reserve_is_released_when_the_quote_expires()
{
    let db = Db::start().await;
    let c = db.campaign("100.00", false).await;
    let customer = db.customer().await;
    let quote = db.quote(Some(&customer)).await;
    let r = reserved(
        &db.reserve(&request(c, quote, "100.00", None, Some(&customer)))
            .await,
    );
    let txn = db.transaction(&customer).await;
    let mut tx = db.pool.begin().await.expect("begin");
    db.redeemer()
        .redeem_in(&mut tx, &exchange(txn, &customer), quote)
        .await
        .expect("redeem inside the completion");
    tx.rollback()
        .await
        .expect("the completion fails and rolls back");

    assert_eq!(db.redemptions(c).await, 0, "no redemption");
    assert_eq!(db.budget(c).await, (d("0"), 0), "no consume");
    assert_eq!(
        db.reservation_status(r).await,
        "RESERVED",
        "held until the quote expires"
    );

    db.expire(r).await;
    reserved(
        &db.reserve(&request(c, db.quote(None).await, "100.00", None, None))
            .await,
    );
    assert_eq!(
        db.reservation_status(r).await,
        "RELEASED",
        "the failed exchange keeps no budget"
    );
    db.finish().await;
}

/// §9 edge 3, D-8: a quote that expired before completion: `redeem_in` re-reserves under the lock when the budget
/// still has room, and redeems.
#[tokio::test]
async fn an_expired_quote_is_re_reserved_and_redeemed_when_the_budget_has_room() {
    let db = Db::start().await;
    let c = db.campaign("100.00", false).await;
    let customer = db.customer().await;
    let quote = db.quote(Some(&customer)).await;
    let r = reserved(
        &db.reserve(&request(c, quote, "60.00", None, Some(&customer)))
            .await,
    );
    db.expire(r).await;
    let txn = db.transaction(&customer).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let ids = db
        .redeemer()
        .redeem_in(&mut tx, &exchange(txn, &customer), quote)
        .await
        .expect("re-reserved and redeemed");
    tx.commit().await.expect("commit");
    assert_eq!(ids.len(), 1);
    assert_eq!(db.redemptions(c).await, 1);
    assert_eq!(db.budget(c).await, (d("60.00"), 1));
    db.finish().await;
}

/// §9 edge 3, D-8: a quote that expired before completion while another quote took the budget: `redeem_in` refuses
/// `benefitLimitReached` and redeems nothing.
#[tokio::test]
async fn an_expired_quote_whose_budget_was_taken_is_benefit_limit_reached() {
    let db = Db::start().await;
    let c = db.campaign("100.00", false).await;
    let customer = db.customer().await;
    let quote = db.quote(Some(&customer)).await;
    let r = reserved(
        &db.reserve(&request(c, quote, "60.00", None, Some(&customer)))
            .await,
    );
    db.expire(r).await;
    reserved(
        &db.reserve(&request(c, db.quote(None).await, "60.00", None, None))
            .await,
    );
    let txn = db.transaction(&customer).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let err = db
        .redeemer()
        .redeem_in(&mut tx, &exchange(txn, &customer), quote)
        .await
        .expect_err("no room left for the expired reservation");
    assert!(matches!(err, BenefitError::BenefitLimitReached), "{err:?}");
    assert_eq!(err.code(), "benefitLimitReached");
    tx.rollback().await.expect("the completion rolls back");
    assert_eq!(db.redemptions(c).await, 0);
    assert_eq!(db.budget(c).await, (d("0"), 0));
    db.finish().await;
}

// ── reversal (owner M3) ─────────────────────────────────────────────────────

/// Redeem a coupon on `c` for a new customer and return (coupon, transaction).
async fn redeemed_coupon(db: &Db, c: Campaign) -> (Uuid, Uuid) {
    let customer = db.customer().await;
    let coupon = db.coupon(c, None).await;
    let quote = db.quote(Some(&customer)).await;
    reserved(
        &db.reserve(&request(c, quote, "40.00", Some(coupon), Some(&customer)))
            .await,
    );
    let txn = db.transaction(&customer).await;
    let mut tx = db.pool.begin().await.expect("begin");
    db.redeemer()
        .redeem_in(&mut tx, &exchange(txn, &customer), quote)
        .await
        .expect("redeem");
    tx.commit().await.expect("commit");
    assert_eq!(db.coupon_status(coupon).await, "REDEEMED");
    (coupon, txn)
}

/// Owner M3 (§4.2): a reversal marks the redemption REVERSED with its reversal, returns budget and use count, and
/// re-issues a single-use coupon of a version that is still ACTIVE.
#[tokio::test]
async fn a_reversal_returns_the_budget_and_re_issues_the_coupon_of_an_active_version() {
    let db = Db::start().await;
    let c = db.campaign("1000.00", true).await;
    let (coupon, txn) = redeemed_coupon(&db, c).await;
    let reversal = db.reversal(txn).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let n = db
        .redeemer()
        .adjust_in(
            &mut tx,
            &PostedReversal {
                reversal_id: reversal,
                transaction_id: txn,
            },
        )
        .await
        .expect("adjust");
    tx.commit().await.expect("commit");
    assert_eq!(n, 1);
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM benefit_redemptions WHERE transaction_id = '{txn}' \
             AND status = 'REVERSED' AND reversal_id = '{reversal}'"
        ))
        .await,
        1,
        "the redemption is REVERSED, never deleted"
    );
    assert_eq!(
        db.budget(c).await,
        (d("0.00"), 0),
        "budget and use count returned"
    );
    assert_eq!(db.coupon_status(coupon).await, "ISSUED", "re-issued");
    db.finish().await;
}

/// §9 edge 6, owner M3: the reversal of a coupon redemption whose version has EXPIRED leaves the coupon REDEEMED (the
/// redemption is still REVERSED and the budget still returned).
#[tokio::test]
async fn a_reversal_after_the_version_expired_leaves_the_coupon_redeemed() {
    let db = Db::start().await;
    let c = db.campaign("1000.00", true).await;
    let (coupon, txn) = redeemed_coupon(&db, c).await;
    db.exec(&format!(
        "UPDATE campaign_versions SET starts_at = now() - interval '10 days', ends_at = now() - interval '1 day' \
         WHERE id = '{}'",
        c.version
    ))
    .await;
    let reversal = db.reversal(txn).await;
    let mut tx = db.pool.begin().await.expect("begin");
    db.redeemer()
        .adjust_in(
            &mut tx,
            &PostedReversal {
                reversal_id: reversal,
                transaction_id: txn,
            },
        )
        .await
        .expect("adjust");
    tx.commit().await.expect("commit");
    assert_eq!(
        db.coupon_status(coupon).await,
        "REDEEMED",
        "an EXPIRED version re-issues nothing"
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM benefit_redemptions WHERE transaction_id = '{txn}' AND status = 'REVERSED'"
        ))
        .await,
        1
    );
    assert_eq!(db.budget(c).await, (d("0.00"), 0));
    db.finish().await;
}

// ── the hash ────────────────────────────────────────────────────────────────

/// §9.1, D-9: the stored hash is not the code: `coupons.code_hash` is the hasher's HMAC of the code, never the code's
/// bytes, and no column of `coupons` holds the code.
#[tokio::test]
async fn the_stored_hash_is_not_the_code() {
    let db = Db::start().await;
    let c = db.campaign("1000.00", true).await;
    let issued = db
        .coupons()
        .issue(c.rule, 3, None, in_minutes(60), db.user)
        .await
        .expect("issue three");
    assert_eq!(issued.len(), 3);
    let h = hasher(KEYS_A);
    for i in &issued {
        assert!(!i.code.is_empty());
        assert_eq!(i.coupon.status, CouponStatus::Issued);
        assert_eq!(i.coupon.rule_id, c.rule);
        let (stored, row_text): (Vec<u8>, String) = sqlx::query_as(
            "SELECT code_hash, row_to_json(coupons)::text FROM coupons WHERE id = $1",
        )
        .bind(i.coupon.id)
        .fetch_one(&db.pool)
        .await
        .expect("the coupon row");
        assert_eq!(
            stored,
            h.hash(&i.code).to_vec(),
            "code_hash = HMAC of the code"
        );
        assert_ne!(stored, i.code.as_bytes(), "the stored hash is not the code");
        assert!(
            !row_text.contains(&i.code),
            "no column holds the code: {row_text}"
        );
    }
    let mut codes: Vec<&str> = issued.iter().map(|i| i.code.as_str()).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), 3, "every code is different");
    db.finish().await;
}

/// D-9: the code is normalised (upper-case, no spaces) before hashing; the hash depends on the PII key and is never
/// the code.
#[test]
fn the_code_hash_is_normalised_keyed_and_never_the_code() {
    assert_eq!(CouponCodeHasher::normalize(" ab c-d9 "), "ABC-D9");
    let a = hasher(KEYS_A);
    assert_eq!(a.hash("abcd 1234"), a.hash("ABCD1234"), "normalised first");
    assert_eq!(a.hash("ABCD1234"), a.hash("ABCD1234"), "deterministic");
    assert_ne!(
        a.hash("ABCD1234"),
        a.hash("ABCD1235"),
        "another code, another hash"
    );
    assert_ne!(
        a.hash("ABCD1234"),
        hasher(KEYS_B).hash("ABCD1234"),
        "keyed by the PII key file"
    );
    let mut padded = [0u8; 32];
    padded[..8].copy_from_slice(b"ABCD1234");
    assert_ne!(a.hash("ABCD1234"), padded, "not the code itself");
    assert_eq!(
        BenefitError::BenefitLimitReached.code(),
        "benefitLimitReached"
    );
}
