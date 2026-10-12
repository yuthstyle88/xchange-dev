//! Rework tests for MK-3 must-fixes (#1395, PR 1532):
//! 1. A two-connection last-use race on max_uses: exactly one reserves, the other capReached.
//! 2. A per-customer cap test: the second use by the same customer -> capReached, another customer still reserves.
//! 3. A per-customer period days test: allows reserve after window expires.

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

use xc_marketing::domain::benefit::rule::BenefitCaps;
use xc_marketing::infrastructure::pg_budget_ledger::{
    BudgetLedger, CompletedExchange, ReserveOutcome, ReserveRequest, SkipReason,
};

const MUST_FINISH: Duration = Duration::from_secs(60);
const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{nanos}_{n}")
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a valid Decimal")
}

fn in_minutes(m: i64) -> DateTime<Utc> {
    Utc::now() + chrono::Duration::minutes(m)
}

#[derive(Clone, Copy)]
struct Campaign {
    version: Uuid,
    rule: Uuid,
}

struct Db {
    pool: PgPool,
    schema: String,
    branch: String,
    user: Uuid,
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
        let schema = unique("mk3_caps");
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

        let branch = unique("B").replace('_', "-");
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(&branch)
            .bind(ORG_1)
            .execute(&pool)
            .await
            .expect("a branch");

        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, $3, 'ACTIVE', 'test')",
        )
        .bind(user_id)
        .bind(ORG_1)
        .bind(unique("user"))
        .execute(&pool)
        .await
        .expect("a user");

        Self {
            pool,
            schema,
            branch,
            user: user_id,
        }
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

    fn ledger(&self) -> BudgetLedger {
        BudgetLedger::new(self.pool.clone())
    }

    async fn customer(&self) -> String {
        let id = unique("CX");
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($1, $2::uuid, $1, 'Customer', 'LIVE', now())",
        )
        .bind(&id)
        .bind(ORG_1)
        .execute(&self.pool)
        .await
        .expect("a customer");
        id
    }

    async fn campaign_with_caps(
        &self,
        budget_thb: &str,
        max_uses: Option<i32>,
        caps: BenefitCaps,
    ) -> Campaign {
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
             budget_thb, max_uses, stack_group, priority, version_hash, edited_at, created_by, created_at) \
             VALUES ($1, $2::uuid, $3, 1, 'APPROVED', now() - interval '1 day', now() + interval '30 days', \
             $4, $5, 'g', 1, 'hash', now(), $6, now())",
        )
        .bind(version)
        .bind(ORG_1)
        .bind(campaign)
        .bind(d(budget_thb))
        .bind(max_uses)
        .bind(self.user)
        .execute(&self.pool)
        .await
        .expect("an APPROVED version with caps");

        sqlx::query(
            "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, \
             calculation, caps, requires_coupon, rule_no) \
             VALUES ($1, $2, 'FEE_WAIVER', 'PRICE', '{}'::jsonb, '{}'::jsonb, $3, false, 1)",
        )
        .bind(rule)
        .bind(version)
        .bind(serde_json::to_value(&caps).unwrap())
        .execute(&self.pool)
        .await
        .expect("a rule with caps");

        sqlx::query("INSERT INTO campaign_budgets (campaign_version_id) VALUES ($1)")
            .bind(version)
            .execute(&self.pool)
            .await
            .expect("budget row");

        Campaign { version, rule }
    }

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

    async fn transaction(&self, customer: &str) -> Uuid {
        let teller = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, $3, 'ACTIVE', 'test')",
        )
        .bind(teller)
        .bind(ORG_1)
        .bind(unique("teller"))
        .execute(&self.pool)
        .await
        .expect("teller user");

        let shift = unique("shift");
        sqlx::query(
            "UPDATE shifts SET status = 'CLOSED', closed_at = now() WHERE branch_id = $1 AND status = 'OPEN'",
        )
        .bind(&self.branch)
        .execute(&self.pool)
        .await
        .expect("close open shift");

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
             VALUES ($1, $2::uuid, $3, $4, 'LIVE', $7, $5, 'hash', $6, 'Customer', 'THA', 'BUY', 'USD', 100, 32.0, \
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
        .expect("a transaction");
        id
    }

    async fn reserve(
        &self,
        c: Campaign,
        quote: Uuid,
        amount: &str,
        customer: Option<&str>,
    ) -> ReserveOutcome {
        let req = ReserveRequest {
            organization_id: Uuid::parse_str(ORG_1).unwrap(),
            quote_id: quote,
            rule_id: c.rule,
            campaign_version_id: c.version,
            coupon_id: None,
            customer_id: customer.map(str::to_owned),
            amount_thb: d(amount),
            expires_at: in_minutes(15),
        };
        let mut tx = self.pool.begin().await.expect("begin");
        let out = self
            .ledger()
            .reserve_in(&mut tx, &req)
            .await
            .expect("reserve_in");
        tx.commit().await.expect("commit");
        out
    }
}

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
                .expect("holds connection");
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
        outs.push(h.await.expect("racer ran"));
    }
    [outs[0].clone(), outs[1].clone()]
}

fn request(c: Campaign, quote: Uuid, amount: &str, customer: Option<&str>) -> ReserveRequest {
    ReserveRequest {
        organization_id: Uuid::parse_str(ORG_1).expect("a uuid"),
        quote_id: quote,
        rule_id: c.rule,
        campaign_version_id: c.version,
        coupon_id: None,
        customer_id: customer.map(str::to_owned),
        amount_thb: d(amount),
        expires_at: in_minutes(15),
    }
}

/// A two-connection last-use race on max_uses: exactly one reserves, the other capReached.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_reserves_of_the_last_max_uses_give_it_to_exactly_one() {
    let db = Db::start().await;
    for round in 0..5 {
        let c = db
            .campaign_with_caps("100000.00", Some(1), BenefitCaps::default())
            .await;
        let (qa, qb) = (db.quote(None).await, db.quote(None).await);
        let outs = race(
            &db,
            request(c, qa, "10.00", None),
            request(c, qb, "10.00", None),
        )
        .await;

        let won = outs
            .iter()
            .filter(|o| matches!(o, ReserveOutcome::Reserved(_)))
            .count();
        let cap_reached = outs
            .iter()
            .filter(|o| **o == ReserveOutcome::Skipped(SkipReason::CapReached))
            .count();

        assert_eq!(
            (won, cap_reached),
            (1, 1),
            "round {round}: exactly one reserves, the other is capReached: {outs:?}"
        );

        let live_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM benefit_reservations WHERE campaign_version_id = $1 AND status = 'RESERVED'",
        )
        .bind(c.version)
        .fetch_one(&db.pool)
        .await
        .expect("count live reservations");

        assert_eq!(
            live_count, 1,
            "round {round}: exactly one live reservation in DB"
        );
    }
    assert_eq!(SkipReason::CapReached.as_str(), "capReached");
    db.finish().await;
}

/// A per-customer cap test: the second use by the same customer -> capReached, another customer still reserves.
#[tokio::test(flavor = "multi_thread")]
async fn per_customer_cap_limits_same_customer_while_another_customer_still_reserves() {
    let db = Db::start().await;
    let caps = BenefitCaps {
        per_transaction_max_thb: None,
        per_customer_uses: Some(1),
        per_customer_period_days: None,
    };
    let c = db.campaign_with_caps("100000.00", None, caps).await;
    let cust_a = db.customer().await;
    let cust_b = db.customer().await;

    // 1. First quote for customer A reserves successfully
    let qa1 = db.quote(Some(&cust_a)).await;
    let out_a1 = db.reserve(c, qa1, "25.00", Some(&cust_a)).await;
    let res_a1 = match out_a1 {
        ReserveOutcome::Reserved(id) => id,
        other => panic!("expected Reserved for customer A's first use, got {other:?}"),
    };

    // 2. Second quote for customer A is rejected with CapReached (while first is RESERVED)
    let qa2 = db.quote(Some(&cust_a)).await;
    let out_a2 = db.reserve(c, qa2, "25.00", Some(&cust_a)).await;
    assert_eq!(
        out_a2,
        ReserveOutcome::Skipped(SkipReason::CapReached),
        "customer A second use while first is RESERVED must be capReached"
    );

    // 3. Customer B with quote can still reserve successfully
    let qb1 = db.quote(Some(&cust_b)).await;
    let out_b1 = db.reserve(c, qb1, "25.00", Some(&cust_b)).await;
    assert!(
        matches!(out_b1, ReserveOutcome::Reserved(_)),
        "customer B should be able to reserve despite customer A cap reached"
    );

    // 4. Consume customer A's reservation (now REDEEMED)
    let tx_a = db.transaction(&cust_a).await;
    let exchange_a = CompletedExchange {
        transaction_id: tx_a,
        customer_id: Some(cust_a.clone()),
        referral_attribution_id: None,
    };
    let mut tx = db.pool.begin().await.expect("begin");
    db.ledger()
        .consume_in(&mut tx, res_a1, &exchange_a)
        .await
        .expect("consume reservation for customer A");
    tx.commit().await.expect("commit");

    // 5. Third quote for customer A after redemption is still rejected with CapReached
    let qa3 = db.quote(Some(&cust_a)).await;
    let out_a3 = db.reserve(c, qa3, "25.00", Some(&cust_a)).await;
    assert_eq!(
        out_a3,
        ReserveOutcome::Skipped(SkipReason::CapReached),
        "customer A use after REDEEMED must still be capReached"
    );

    db.finish().await;
}

/// A per-customer period days test: allows reserve after window expires.
#[tokio::test(flavor = "multi_thread")]
async fn per_customer_period_days_allows_reserve_after_window_expires() {
    let db = Db::start().await;
    let caps = BenefitCaps {
        per_transaction_max_thb: None,
        per_customer_uses: Some(1),
        per_customer_period_days: Some(7),
    };
    let c = db.campaign_with_caps("100000.00", None, caps).await;
    let cust = db.customer().await;

    // First use: reserve and consume
    let q1 = db.quote(Some(&cust)).await;
    let out1 = db.reserve(c, q1, "25.00", Some(&cust)).await;
    let res1 = match out1 {
        ReserveOutcome::Reserved(id) => id,
        other => panic!("expected Reserved, got {other:?}"),
    };

    let tx_id = db.transaction(&cust).await;
    let exchange = CompletedExchange {
        transaction_id: tx_id,
        customer_id: Some(cust.clone()),
        referral_attribution_id: None,
    };
    let mut tx = db.pool.begin().await.expect("begin");
    db.ledger()
        .consume_in(&mut tx, res1, &exchange)
        .await
        .expect("consume reservation");
    tx.commit().await.expect("commit");

    // Backdate the redemption to 10 days ago (outside the 7-day window)
    sqlx::query(
        "UPDATE benefit_redemptions SET redeemed_at = now() - interval '10 days' WHERE customer_id = $1",
    )
    .bind(&cust)
    .execute(&db.pool)
    .await
    .expect("backdate redemption");

    // Now a new reservation should succeed because old redemption is outside window
    let q2 = db.quote(Some(&cust)).await;
    let out2 = db.reserve(c, q2, "25.00", Some(&cust)).await;
    assert!(
        matches!(out2, ReserveOutcome::Reserved(_)),
        "customer should be able to reserve after per_customer_period_days has elapsed"
    );

    db.finish().await;
}
