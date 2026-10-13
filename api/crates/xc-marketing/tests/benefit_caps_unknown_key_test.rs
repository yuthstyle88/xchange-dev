//! Behaviour table tests for BenefitCaps unknown/misspelled keys (#1579):
//! Covers every row of the #1579 behaviour table:
//! - Row 1: caps `{}` -> decodes, no caps (unchanged)
//! - Row 2: caps with the 3 valid keys -> decodes them exactly (unchanged)
//! - Row 3: caps with an unknown or misspelled key -> decode error; reserve refused, nothing written; repository read errors
//! - Row 4: caps not an object -> decode error (unchanged after #1569)

use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xc_marketing::domain::benefit::rule::BenefitCaps;
use xc_marketing::domain::coupon::benefit_error::BenefitError;
use xc_marketing::domain::engine::active_rules::ActiveRules;
use xc_marketing::infrastructure::pg_budget_ledger::{
    BudgetLedger, ReserveOutcome, ReserveRequest, SkipReason,
};
use xc_marketing::infrastructure::pg_campaign_repository::PgCampaignRepository;

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
        let schema = unique("caps_unknown_test");
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

    fn repository(&self) -> PgCampaignRepository {
        PgCampaignRepository::new()
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
            "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2)",
        )
        .bind(version)
        .bind(&self.branch)
        .execute(&self.pool)
        .await
        .expect("campaign_branches");

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

// ---------------------------------------------------------------------------
// Row 1: caps {} -> decodes, no caps (unchanged)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_row1_caps_empty_object_decodes_no_caps_and_allows_unlimited_per_customer() {
    // 1. Direct type deserialization
    let caps: BenefitCaps = serde_json::from_str("{}").expect("empty object decodes");
    assert_eq!(caps, BenefitCaps::default());
    assert_eq!(caps.per_transaction_max_thb, None);
    assert_eq!(caps.per_customer_uses, None);
    assert_eq!(caps.per_customer_period_days, None);

    let db = Db::start().await;
    let c = db
        .campaign_with_caps_value("1000.00", None, json!({}))
        .await;

    // 2. Repository read
    let mut tx = db.pool.begin().await.expect("begin");
    let org = Uuid::parse_str(ORG_1).unwrap();
    let rules = db
        .repository()
        .active_for(&mut tx, org, &db.branch, Utc::now())
        .await
        .expect("repository read succeeds with empty caps");
    tx.commit().await.expect("commit");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].rule.caps, BenefitCaps::default());

    // 3. Reserve path (multiple reserves for same customer succeed because there are no caps)
    let cust = db.customer().await;
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

// ---------------------------------------------------------------------------
// Row 2: caps with the 3 valid keys -> decodes them exactly (unchanged)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_row2_caps_with_three_valid_keys_decodes_exactly() {
    let valid_caps_json = json!({
        "perTransactionMaxThb": "50.00",
        "perCustomerUses": 2,
        "perCustomerPeriodDays": 30,
    });

    // 1. Direct type deserialization
    let caps: BenefitCaps =
        serde_json::from_value(valid_caps_json.clone()).expect("valid 3 keys decode");
    assert_eq!(
        caps.per_transaction_max_thb,
        Some(Decimal::from_str("50.00").unwrap())
    );
    assert_eq!(caps.per_customer_uses, Some(2));
    assert_eq!(caps.per_customer_period_days, Some(30));

    let db = Db::start().await;
    let c = db
        .campaign_with_caps_value("1000.00", None, valid_caps_json)
        .await;

    // 2. Repository read decodes all 3 keys exactly
    let mut tx = db.pool.begin().await.expect("begin");
    let org = Uuid::parse_str(ORG_1).unwrap();
    let rules = db
        .repository()
        .active_for(&mut tx, org, &db.branch, Utc::now())
        .await
        .expect("repository read succeeds with 3 valid keys");
    tx.commit().await.expect("commit");
    assert_eq!(rules.len(), 1);
    assert_eq!(
        rules[0].rule.caps.per_transaction_max_thb,
        Some(Decimal::from_str("50.00").unwrap())
    );
    assert_eq!(rules[0].rule.caps.per_customer_uses, Some(2));
    assert_eq!(rules[0].rule.caps.per_customer_period_days, Some(30));

    // 3. Reserve path respects the decoded perCustomerUses cap (2 uses allowed, 3rd is CapReached)
    let cust = db.customer().await;

    // Use 1
    let q1 = db.quote(Some(&cust)).await;
    let req1 = db.reserve_request(c, q1, "10.00", Some(&cust));
    let mut tx1 = db.pool.begin().await.expect("begin");
    let out1 = db
        .ledger()
        .reserve_in(&mut tx1, &req1)
        .await
        .expect("use 1");
    tx1.commit().await.expect("commit");
    assert!(matches!(out1, ReserveOutcome::Reserved(_)));

    // Use 2
    let q2 = db.quote(Some(&cust)).await;
    let req2 = db.reserve_request(c, q2, "10.00", Some(&cust));
    let mut tx2 = db.pool.begin().await.expect("begin");
    let out2 = db
        .ledger()
        .reserve_in(&mut tx2, &req2)
        .await
        .expect("use 2");
    tx2.commit().await.expect("commit");
    assert!(matches!(out2, ReserveOutcome::Reserved(_)));

    // Use 3 exceeds perCustomerUses: 2 -> CapReached
    let q3 = db.quote(Some(&cust)).await;
    let req3 = db.reserve_request(c, q3, "10.00", Some(&cust));
    let mut tx3 = db.pool.begin().await.expect("begin");
    let out3 = db
        .ledger()
        .reserve_in(&mut tx3, &req3)
        .await
        .expect("use 3");
    tx3.commit().await.expect("commit");
    assert_eq!(out3, ReserveOutcome::Skipped(SkipReason::CapReached));

    db.finish().await;
}

// ---------------------------------------------------------------------------
// Row 3: caps with an unknown or misspelled key -> decode error; reserve refused, nothing written; repository read errors
// (This test goes RED if #[serde(deny_unknown_fields)] is missing from BenefitCaps)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_row3_caps_with_misspelled_or_unknown_key_refuses_reserve_and_repository_read() {
    let misspelled_caps_cases = vec![
        json!({"perCustomerUse": 1}),          // misspelled perCustomerUses
        json!({"perTransactionMax": "50.00"}), // misspelled perTransactionMaxThb
        json!({"unknownKey": 123}),            // completely unknown key
        json!({"perCustomerUses": 1, "extra": "not-allowed"}), // valid + extra key
    ];

    for misspelled in misspelled_caps_cases {
        // 1. Direct type deserialization must fail
        let direct_decode = serde_json::from_value::<BenefitCaps>(misspelled.clone());
        assert!(
            direct_decode.is_err(),
            "direct decode of misspelled/unknown caps {:?} must fail under deny_unknown_fields",
            misspelled
        );

        let db = Db::start().await;
        let c = db
            .campaign_with_caps_value("1000.00", None, misspelled.clone())
            .await;

        // 2. Repository read must fail with decode error
        let mut tx_repo = db.pool.begin().await.expect("begin");
        let org = Uuid::parse_str(ORG_1).unwrap();
        let repo_res = db
            .repository()
            .active_for(&mut tx_repo, org, &db.branch, Utc::now())
            .await;
        assert!(
            repo_res.is_err(),
            "repository active_for must fail on unknown/misspelled caps {:?}, but got {:?}",
            misspelled,
            repo_res
        );
        let repo_err_msg = format!("{:?}", repo_res.unwrap_err());
        assert!(
            repo_err_msg.contains("failed to decode caps"),
            "expected 'failed to decode caps' in repo error, got: {repo_err_msg}"
        );
        tx_repo.rollback().await.expect("rollback");

        // 3. Reserve path must fail with decode error and write nothing
        let cust = db.customer().await;
        let quote = db.quote(Some(&cust)).await;
        let req = db.reserve_request(c, quote, "10.00", Some(&cust));

        let mut tx_res = db.pool.begin().await.expect("begin");
        let reserve_res = db.ledger().reserve_in(&mut tx_res, &req).await;
        assert!(
            reserve_res.is_err(),
            "reserve_in must fail on unknown/misspelled caps {:?}, but got {:?}",
            misspelled,
            reserve_res
        );

        match reserve_res.unwrap_err() {
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
        tx_res.rollback().await.expect("rollback");

        // Verify: no reservation rows written
        let (res_count,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM benefit_reservations WHERE rule_id = $1")
                .bind(c.rule)
                .fetch_one(&db.pool)
                .await
                .expect("count reservations");
        assert_eq!(
            res_count, 0,
            "no reservation row should be written on unknown caps"
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
            "no redemption row should be written on unknown caps"
        );

        // Verify: budget row untouched (consumed_thb == 0, used_count == 0)
        let (consumed, used_count): (Decimal, i32) = sqlx::query_as(
            "SELECT consumed_thb, used_count FROM campaign_budgets WHERE campaign_version_id = $1",
        )
        .bind(c.version)
        .fetch_one(&db.pool)
        .await
        .expect("budget row");
        assert_eq!(consumed, Decimal::ZERO, "consumed_thb must remain 0");
        assert_eq!(used_count, 0, "used_count must remain 0");

        db.finish().await;
    }
}

// ---------------------------------------------------------------------------
// Row 4: caps not an object -> decode error (unchanged after #1569)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_row4_caps_not_an_object_refuses_reserve_and_repository_read() {
    let non_object_cases = vec![json!("not-an-object"), json!(true), json!(42)];

    for non_object in non_object_cases {
        // 1. Direct type deserialization must fail
        let direct_decode = serde_json::from_value::<BenefitCaps>(non_object.clone());
        assert!(direct_decode.is_err());

        let db = Db::start().await;
        let c = db
            .campaign_with_caps_value("1000.00", None, non_object.clone())
            .await;

        // 2. Repository read must fail
        let mut tx_repo = db.pool.begin().await.expect("begin");
        let org = Uuid::parse_str(ORG_1).unwrap();
        let repo_res = db
            .repository()
            .active_for(&mut tx_repo, org, &db.branch, Utc::now())
            .await;
        assert!(repo_res.is_err());
        tx_repo.rollback().await.expect("rollback");

        // 3. Reserve path must fail and write nothing
        let cust = db.customer().await;
        let quote = db.quote(Some(&cust)).await;
        let req = db.reserve_request(c, quote, "10.00", Some(&cust));

        let mut tx_res = db.pool.begin().await.expect("begin");
        let reserve_res = db.ledger().reserve_in(&mut tx_res, &req).await;
        assert!(reserve_res.is_err());
        tx_res.rollback().await.expect("rollback");

        // Verify: no reservation rows written
        let (res_count,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM benefit_reservations WHERE rule_id = $1")
                .bind(c.rule)
                .fetch_one(&db.pool)
                .await
                .expect("count reservations");
        assert_eq!(res_count, 0);

        // Verify: no redemption rows written
        let (red_count,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM benefit_redemptions WHERE rule_id = $1")
                .bind(c.rule)
                .fetch_one(&db.pool)
                .await
                .expect("count redemptions");
        assert_eq!(red_count, 0);

        // Verify: budget row untouched
        let (consumed, used_count): (Decimal, i32) = sqlx::query_as(
            "SELECT consumed_thb, used_count FROM campaign_budgets WHERE campaign_version_id = $1",
        )
        .bind(c.version)
        .fetch_one(&db.pool)
        .await
        .expect("budget row");
        assert_eq!(consumed, Decimal::ZERO);
        assert_eq!(used_count, 0);

        db.finish().await;
    }
}
