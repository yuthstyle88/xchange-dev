//! Ticket #2488: `couponIssue` (`POST /api/campaigns/{id}/coupons`) must verify that a
//! provided `customerId` exists in `customers` for the caller's organisation before
//! issuing coupons, answering 404 `notFound` (field `customerId`) for both unknown and
//! foreign-organisation customer IDs and writing no `coupons` rows.
//!
//! Behaviour table:
//! 1. `customerId` names no customer -> 404 `notFound`, field `customerId`; no coupon row written.
//! 2. `customerId` is a customer of another organisation -> 404 `notFound`, field `customerId`; no coupon row written.
//! 3. `customerId` is a customer of the caller's organisation -> 200; coupons bound to that customer.
//! 4. `customerId` absent / null -> 200; coupons with no customer.
//! 5. Edge: `count: 0` with an unknown `customerId` -> still 404 `notFound`, field `customerId`.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::Utc;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const OTHER_ORG: &str = "00000000-0000-0000-0000-000000000099";
const MAIN: &str = "MAIN";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

struct Room {
    pool: PgPool,
    schema: String,
    guard: Arc<AppSessionGuard>,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect(
            "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip",
        );
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("coupon_customer_scope");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, version, created_at, created_by) \
             VALUES ('00000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 'Test User', 'ACTIVE', 1, now(), 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
        .execute(&pool)
        .await
        .expect("seed test user");

        common::install_test_pii_keys();

        Self {
            pool,
            schema,
            guard: Arc::new(AppSessionGuard::new(20)),
        }
    }

    async fn finish(self) {
        let Room { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
    }
}

fn user_of(spec: &str) -> Option<Actor> {
    if spec.is_empty() {
        return None;
    }
    let from = chrono::DateTime::<Utc>::UNIX_EPOCH;
    if let Some(cust) = spec.strip_prefix("CUSTOMER:") {
        return Some(Actor::new(
            "user-cust",
            ORG,
            vec![ScopedRole::new(Role::Customer, None, from, None)],
            Some(cust),
        ));
    }
    let role = match spec {
        "ADMIN" => Role::Admin,
        "EXECUTIVE" => Role::Executive,
        "TELLER" => Role::Teller,
        "MARKETING" => Role::Marketing,
        _ => return None,
    };
    let branch = if role == Role::Teller {
        Some(BranchId::new(MAIN))
    } else {
        None
    };
    let user_id = format!("user-{}", role.as_str());
    Some(Actor::new(
        &user_id,
        ORG,
        vec![ScopedRole::new(role, branch, from, None)],
        None,
    ))
}

struct HeaderCallers;

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let user = user_of(spec);
        let staff_user = user.as_ref().and_then(|u| {
            if u.roles.iter().any(|r| r.role != Role::Customer) {
                Some(u.clone())
            } else {
                None
            }
        });
        Box::pin(async move { Callers { user, staff_user } })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> String {
        self.body["error"]["code"]
            .as_str()
            .or_else(|| self.body["code"].as_str())
            .unwrap_or("")
            .to_owned()
    }

    fn field(&self) -> String {
        self.body["error"]["field"]
            .as_str()
            .or_else(|| self.body["field"].as_str())
            .unwrap_or("")
            .to_owned()
    }
}

async fn call(
    room: &Room,
    who: Option<&str>,
    method: Method,
    uri: &str,
    body: Option<(Value, &str)>,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let guard = room.guard.clone();
    let app = atest::init_service(
        App::new()
            .app_data(web::Data::from(guard))
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"));

    if let Some(w) = who {
        req = req.insert_header((AS, w));
    }
    if let Some((b, key)) = body {
        req = req
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header(("idempotency-key", key.to_owned()))
            .set_payload(b.to_string());
    }

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

    Answer { status, text, body }
}

async fn seed_campaign(pool: &PgPool, status: &str) -> (Uuid, Uuid) {
    let org_id = Uuid::parse_str(ORG).unwrap();
    let user_id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    let v_hash = if status == "DRAFT" {
        None
    } else {
        Some("v1_hash")
    };

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, current_version_id, created_by, created_at) \
         VALUES ($1, $2, 'Test Campaign', NULL, $3, now())",
    )
    .bind(campaign_id)
    .bind(org_id)
    .bind(user_id)
    .execute(pool)
    .await
    .expect("seed campaign");

    sqlx::query(
        "INSERT INTO campaign_versions ( \
             id, organization_id, campaign_id, version, status, starts_at, ends_at, budget_thb, \
             stack_group, priority, version_hash, created_by, created_at, edited_at \
         ) VALUES ($1, $2, $3, 1, $4, now() - interval '1 hour', now() + interval '10 days', \
                   10000.00, 'RATE', 1, $5, $6, now(), now())",
    )
    .bind(version_id)
    .bind(org_id)
    .bind(campaign_id)
    .bind(status)
    .bind(v_hash)
    .bind(user_id)
    .execute(pool)
    .await
    .expect("seed version");

    sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
        .bind(version_id)
        .bind(campaign_id)
        .execute(pool)
        .await
        .expect("update current_version_id");

    sqlx::query(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) \
         VALUES ($1, 0, 0)",
    )
    .bind(version_id)
    .execute(pool)
    .await
    .expect("seed budget");

    (campaign_id, version_id)
}

async fn seed_coupon_rule(pool: &PgPool, version_id: Uuid) -> Uuid {
    let rule_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO benefit_rules ( \
             id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, caps, requires_coupon, rule_no \
         ) VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', '{}', '{}', '{}', true, 1)",
    )
    .bind(rule_id)
    .bind(version_id)
    .execute(pool)
    .await
    .expect("seed coupon rule");
    rule_id
}

async fn seed_customer(pool: &PgPool, org: &str, customer_id: &str) {
    let org_id = Uuid::parse_str(org).unwrap();
    let org_code = unique("ORG");
    sqlx::query(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ($1, $2, 'Other Org', 'Asia/Bangkok', now()) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(org_id)
    .bind(&org_code)
    .execute(pool)
    .await
    .expect("seed organization");

    let customer_no = unique("CUST");
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2, $3, 'Test Customer', 'LIVE', now())",
    )
    .bind(customer_id)
    .bind(org_id)
    .bind(&customer_no)
    .execute(pool)
    .await
    .expect("seed customer");
}

// ------------------------------------------------------------
// Row 1: customerId names no customer -> 404 notFound, field customerId; no coupon row written
// ------------------------------------------------------------

#[actix_web::test]
async fn row1_unknown_customer_id_is_refused_404_not_found_and_writes_no_coupons() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_campaign(&room.pool, "APPROVED").await;
    let rule_id = seed_coupon_rule(&room.pool, version_id).await;

    let uri = format!("/api/campaigns/{campaign_id}/coupons");
    let payload = json!({
        "ruleId": rule_id.to_string(),
        "count": 2,
        "customerId": "cust-does-not-exist",
        "expiresAt": "2026-12-31T23:59:59Z"
    });

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload, &unique("row1_unknown_customer"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::NOT_FOUND,
        "couponIssue with unknown customerId must be refused with 404: {}",
        ans.text
    );
    assert_eq!(ans.code(), "notFound", "body: {}", ans.text);
    assert_eq!(ans.field(), "customerId", "body: {}", ans.text);

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM coupons")
        .fetch_one(&room.pool)
        .await
        .expect("count coupons");
    assert_eq!(count, 0, "no coupons row must be written");

    room.finish().await;
}

// ------------------------------------------------------------
// Row 2: customerId is a customer of another organisation -> 404 notFound, field customerId; no coupon row written
// ------------------------------------------------------------

#[actix_web::test]
async fn row2_customer_of_another_organisation_is_refused_404_not_found_and_writes_no_coupons() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_campaign(&room.pool, "APPROVED").await;
    let rule_id = seed_coupon_rule(&room.pool, version_id).await;
    seed_customer(&room.pool, OTHER_ORG, "cust-foreign-org").await;

    let uri = format!("/api/campaigns/{campaign_id}/coupons");
    let payload = json!({
        "ruleId": rule_id.to_string(),
        "count": 2,
        "customerId": "cust-foreign-org",
        "expiresAt": "2026-12-31T23:59:59Z"
    });

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload, &unique("row2_foreign_customer"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::NOT_FOUND,
        "couponIssue with customerId of another organisation must be refused with 404: {}",
        ans.text
    );
    assert_eq!(ans.code(), "notFound", "body: {}", ans.text);
    assert_eq!(ans.field(), "customerId", "body: {}", ans.text);

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM coupons")
        .fetch_one(&room.pool)
        .await
        .expect("count coupons");
    assert_eq!(count, 0, "no coupons row must be written");

    room.finish().await;
}

// ------------------------------------------------------------
// Row 3: customerId is a customer of the caller's organisation -> 200; coupons bound to that customer
// ------------------------------------------------------------

#[actix_web::test]
async fn row3_customer_of_callers_organisation_issues_coupons_bound_to_customer_200() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_campaign(&room.pool, "APPROVED").await;
    let rule_id = seed_coupon_rule(&room.pool, version_id).await;
    seed_customer(&room.pool, ORG, "cust-same-org").await;

    let uri = format!("/api/campaigns/{campaign_id}/coupons");
    let payload = json!({
        "ruleId": rule_id.to_string(),
        "count": 2,
        "customerId": "cust-same-org",
        "expiresAt": "2026-12-31T23:59:59Z"
    });

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload, &unique("row3_same_org_customer"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "couponIssue with customerId of caller's organisation must succeed: {}",
        ans.text
    );
    let coupons = ans.body["coupons"]
        .as_array()
        .expect("response body must contain 'coupons' array");
    assert_eq!(coupons.len(), 2, "must issue 2 coupons");

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM coupons WHERE rule_id = $1 AND customer_id = 'cust-same-org'",
    )
    .bind(rule_id)
    .fetch_one(&room.pool)
    .await
    .expect("count coupons bound to customer");
    assert_eq!(
        count, 2,
        "2 coupons rows bound to cust-same-org must be written"
    );

    room.finish().await;
}

// ------------------------------------------------------------
// Row 4: customerId absent / null -> 200; coupons with no customer
// ------------------------------------------------------------

#[actix_web::test]
async fn row4_customer_id_absent_or_null_issues_coupons_with_no_customer_200() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_campaign(&room.pool, "APPROVED").await;
    let rule_id = seed_coupon_rule(&room.pool, version_id).await;

    let uri = format!("/api/campaigns/{campaign_id}/coupons");

    let ans_absent = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((
            json!({
                "ruleId": rule_id.to_string(),
                "count": 2,
                "expiresAt": "2026-12-31T23:59:59Z"
            }),
            &unique("row4_absent"),
        )),
    )
    .await;
    assert_eq!(ans_absent.status, StatusCode::OK, "{}", ans_absent.text);

    let ans_null = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((
            json!({
                "ruleId": rule_id.to_string(),
                "count": 1,
                "customerId": null,
                "expiresAt": "2026-12-31T23:59:59Z"
            }),
            &unique("row4_null"),
        )),
    )
    .await;
    assert_eq!(ans_null.status, StatusCode::OK, "{}", ans_null.text);

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM coupons WHERE rule_id = $1 AND customer_id IS NULL",
    )
    .bind(rule_id)
    .fetch_one(&room.pool)
    .await
    .expect("count unlinked coupons");
    assert_eq!(count, 3, "3 unlinked coupons must be written");

    room.finish().await;
}

// ------------------------------------------------------------
// Edge case: count: 0 with an unknown customerId -> still 404 notFound, field customerId
// ------------------------------------------------------------

#[actix_web::test]
async fn edge_count_zero_with_unknown_customer_id_is_refused_404_not_found() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_campaign(&room.pool, "APPROVED").await;
    let rule_id = seed_coupon_rule(&room.pool, version_id).await;

    let uri = format!("/api/campaigns/{campaign_id}/coupons");
    let payload = json!({
        "ruleId": rule_id.to_string(),
        "count": 0,
        "customerId": "cust-unknown-count-zero",
        "expiresAt": "2026-12-31T23:59:59Z"
    });

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload, &unique("edge_count_zero_unknown"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::NOT_FOUND,
        "couponIssue with count: 0 and unknown customerId must still be refused with 404: {}",
        ans.text
    );
    assert_eq!(ans.code(), "notFound", "body: {}", ans.text);
    assert_eq!(ans.field(), "customerId", "body: {}", ans.text);

    room.finish().await;
}
