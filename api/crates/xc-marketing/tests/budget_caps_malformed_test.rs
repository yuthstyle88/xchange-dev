//! Behaviour table tests for benefit caps decoding (#1569, PR):
//! Covers every row of the #1569 behaviour table:
//! 1. Valid caps, under the limits -> reserve succeeds (unchanged)
//! 2. Valid caps, per-customer limit reached -> Skipped(CapReached) (unchanged)
//! 3. caps = {} or NULL-equivalent default -> no per-customer cap (unchanged; an empty object is valid)
//! 4. caps cannot be decoded -> an error; no reservation, no ledger row, no redemption

use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xc_marketing::domain::benefit::rule::BenefitCaps;
use xc_marketing::domain::coupon::benefit_error::BenefitError;
use xc_marketing::infrastructure::pg_budget_ledger::{
    BudgetLedger, ReserveOutcome, ReserveRequest, SkipReason,
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
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
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
                "TEST_DATABASE_URL is not set. Needs a real PostgreSQL; point it at \
                 an existing database whose name ends in `_test`"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("caps_test");
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

    async fn campaign_with_caps_value(
        &self,
        budget_thb: &str,
        max_uses: Option<i32>,
        caps_value: serde_json::Value,
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
        .bind(caps_value)
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

    fn reserve_request(
        &self,
        c: Campaign,
        quote: Uuid,
        amount: &str,
        customer: Option<&str>,
    ) -> ReserveRequest {
        ReserveRequest {
            organization_id: Uuid::parse_str(ORG_1).unwrap(),
            quote_id: quote,
            rule_id: c.rule,
            campaign_version_id: c.version,
            coupon_id: None,
            customer_id: customer.map(str::to_owned),
            amount_thb: d(amount),
            expires_at: in_minutes(15),
        }
    }
}

// Row 1: Valid caps, under the limits -> reserve succeeds (unchanged)
#[tokio::test]
async fn test_row1_valid_caps_under_limits_reserve_succeeds() {
    let db = Db::start().await;
    let caps = BenefitCaps {
        per_customer_uses: Some(2),
        ..Default::default()
    };
    let c = db
        .campaign_with_caps_value("1000.00", None, serde_json::to_value(&caps).unwrap())
        .await;
    let cust = db.customer().await;
    let quote = db.quote(Some(&cust)).await;
    let req = db.reserve_request(c, quote, "10.00", Some(&cust));

    let mut tx = db.pool.begin().await.expect("begin");
    let out = db
        .ledger()
        .reserve_in(&mut tx, &req)
        .await
        .expect("reserve_in should succeed");
    tx.commit().await.expect("commit");

    match out {
        ReserveOutcome::Reserved(id) => {
            let (status,): (String,) =
                sqlx::query_as("SELECT status FROM benefit_reservations WHERE id = $1")
                    .bind(id)
                    .fetch_one(&db.pool)
                    .await
                    .expect("fetch reservation");
            assert_eq!(status, "RESERVED");
        }
        other => panic!("expected ReserveOutcome::Reserved, got {:?}", other),
    }

    db.finish().await;
}

// Row 2: Valid caps, per-customer limit reached -> Skipped(CapReached) (unchanged)
#[tokio::test]
async fn test_row2_valid_caps_per_customer_limit_reached_skipped_cap_reached() {
    let db = Db::start().await;
    let caps = BenefitCaps {
        per_customer_uses: Some(1),
        ..Default::default()
    };
    let c = db
        .campaign_with_caps_value("1000.00", None, serde_json::to_value(&caps).unwrap())
        .await;
    let cust1 = db.customer().await;
    let cust2 = db.customer().await;

    // First reservation for cust1 succeeds
    let quote1 = db.quote(Some(&cust1)).await;
    let req1 = db.reserve_request(c, quote1, "10.00", Some(&cust1));
    let mut tx1 = db.pool.begin().await.expect("begin");
    let out1 = db
        .ledger()
        .reserve_in(&mut tx1, &req1)
        .await
        .expect("first reserve_in succeeds");
    tx1.commit().await.expect("commit");
    assert!(matches!(out1, ReserveOutcome::Reserved(_)));

    // Second reservation for cust1 reaches per-customer limit -> Skipped(CapReached)
    let quote2 = db.quote(Some(&cust1)).await;
    let req2 = db.reserve_request(c, quote2, "10.00", Some(&cust1));
    let mut tx2 = db.pool.begin().await.expect("begin");
    let out2 = db
        .ledger()
        .reserve_in(&mut tx2, &req2)
        .await
        .expect("reserve_in returns outcome");
    tx2.commit().await.expect("commit");
    assert_eq!(out2, ReserveOutcome::Skipped(SkipReason::CapReached));

    // Another customer cust2 can still reserve
    let quote3 = db.quote(Some(&cust2)).await;
    let req3 = db.reserve_request(c, quote3, "10.00", Some(&cust2));
    let mut tx3 = db.pool.begin().await.expect("begin");
    let out3 = db
        .ledger()
        .reserve_in(&mut tx3, &req3)
        .await
        .expect("cust2 reserve_in succeeds");
    tx3.commit().await.expect("commit");
    assert!(matches!(out3, ReserveOutcome::Reserved(_)));

    db.finish().await;
}

// Row 3: caps = {} or NULL-equivalent default -> no per-customer cap (unchanged; an empty object is valid)
#[tokio::test]
async fn test_row3_caps_empty_object_no_per_customer_cap() {
    let db = Db::start().await;
    let c = db
        .campaign_with_caps_value("1000.00", None, serde_json::json!({}))
        .await;
    let cust = db.customer().await;

    // Multiple reserves for same customer all succeed because caps = {} has no per-customer limit
    for _ in 0..3 {
        let quote = db.quote(Some(&cust)).await;
        let req = db.reserve_request(c, quote, "10.00", Some(&cust));
        let mut tx = db.pool.begin().await.expect("begin");
        let out = db
            .ledger()
            .reserve_in(&mut tx, &req)
            .await
            .expect("reserve_in succeeds with empty caps");
        tx.commit().await.expect("commit");
        assert!(matches!(out, ReserveOutcome::Reserved(_)));
    }

    db.finish().await;
}

// Row 4: caps cannot be decoded -> an error; no reservation, no ledger row, no redemption
#[tokio::test]
async fn test_row4_caps_cannot_be_decoded_returns_error_and_no_ledger_or_redemption_rows() {
    let db = Db::start().await;

    // Test multiple variants of malformed/undecodable caps:
    // a. Field type mismatch: string instead of integer for perCustomerUses
    // b. String literal instead of JSON object
    // c. JSON Array instead of JSON object
    let malformed_caps_cases = vec![
        serde_json::json!({"perCustomerUses": "not-a-number"}),
        serde_json::json!("not-an-object"),
        serde_json::json!(true),
        serde_json::json!({"perCustomerPeriodDays": -1}),
    ];

    for malformed_caps in malformed_caps_cases {
        let c = db
            .campaign_with_caps_value("1000.00", None, malformed_caps.clone())
            .await;
        let cust = db.customer().await;
        let quote = db.quote(Some(&cust)).await;
        let req = db.reserve_request(c, quote, "10.00", Some(&cust));

        let mut tx = db.pool.begin().await.expect("begin");
        let res = db.ledger().reserve_in(&mut tx, &req).await;

        // Must fail with error, NOT unwrap_or_default() to valid caps
        assert!(
            res.is_err(),
            "expected error on undecodable caps {:?}, but reserve_in returned {:?}",
            malformed_caps,
            res
        );

        let err = res.unwrap_err();
        match err {
            BenefitError::Store(pos_err) => {
                let err_msg = format!("{pos_err}");
                assert!(
                    err_msg.contains("failed to decode caps"),
                    "expected 'failed to decode caps' in error, got: {err_msg}"
                );
            }
            other => panic!(
                "expected BenefitError::Store with decode error, got {:?}",
                other
            ),
        }

        // Transaction is rolled back
        tx.rollback().await.expect("rollback");

        // Verify: no reservation rows written
        let (res_count,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM benefit_reservations WHERE rule_id = $1")
                .bind(c.rule)
                .fetch_one(&db.pool)
                .await
                .expect("count reservations");
        assert_eq!(
            res_count, 0,
            "no reservation row should be written on undecodable caps"
        );

        // Verify: no redemption rows written
        let (red_count,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM benefit_redemptions WHERE rule_id = $1")
                .bind(c.rule)
                .fetch_one(&db.pool)
                .await
                .expect("count redemptions");
        assert_eq!(
            red_count, 0,
            "no redemption row should be written on undecodable caps"
        );

        // Verify: budget row unchanged (consumed_thb == 0, used_count == 0)
        let (consumed, used_count): (Decimal, i32) = sqlx::query_as(
            "SELECT consumed_thb, used_count FROM campaign_budgets WHERE campaign_version_id = $1",
        )
        .bind(c.version)
        .fetch_one(&db.pool)
        .await
        .expect("budget row");
        assert_eq!(consumed, Decimal::ZERO, "consumed_thb must remain 0");
        assert_eq!(used_count, 0, "used_count must remain 0");
    }

    db.finish().await;
}
