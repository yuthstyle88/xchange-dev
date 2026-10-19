//! #1992 (+ #1994 folded in): up-front tests for the branch scope of `campaignSimulate`
//! (`POST /api/campaigns/{id}/simulate`, `CampaignSimulateCommand::run` in
//! `api/src/presentation/http/v3/commands/campaign_simulate.rs`).
//!
//! Written before the fix by the ticket author's test writer (for claude/d7aa64), held to the
//! BE lane controller claude/7ca80c's ruling of 2026-10-15 (options A + B, plus the
//! one-branch-per-request rule from #1994). The implementer must not edit, delete or weaken them.
//!
//! Behaviour rows:
//! 1. org-A MARKETING caller, scenario `branchId` is a branch of org B (even one the version's
//!    `campaign_branches` names) -> 400 `validationFailed`, field `branchId`; 0 `campaign_simulations` rows; version stays DRAFT; neither the response nor
//!    the database holds org B's published rates.
//! 2. scenario `branchId` that names no branch at all -> 400 `validationFailed`, field `branchId`;
//!    0 rows; DRAFT.
//! 3. same-org branch the version does not target (`campaign_branches`) -> 400 `validationFailed`,
//!    field `branchId`; 0 rows; DRAFT.
//! 4. two scenarios, the first valid and the second on an org-B branch -> 400 field `branchId`
//!    (per-scenario branch checks run before the multi-branch check); 0 rows; DRAFT.
//! 5. keep-pin: one scenario on a targeted same-org branch -> 200; 1 row; DRAFT -> SIMULATED.
//! 6. two scenarios on two different targeted same-org branches -> 400 `validationFailed`, field
//!    `scenarios`; 0 rows; DRAFT.
//! 7. keep-pin: two scenarios on the same targeted branch (different currencies) -> 200; 1 row;
//!    DRAFT -> SIMULATED.
//!
//! Not covered here on purpose: empty `scenarios` and currency parsing belong to #1989.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::Utc;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
/// Organisation A: the caller's organisation, seeded by the migrations (owns `MAIN`).
const ORG_A: &str = "00000000-0000-0000-0000-000000000001";
/// Organisation B: a second organisation the caller has nothing to do with.
const ORG_B: &str = "00000000-0000-0000-0000-0000000000b2";
const MAIN: &str = "MAIN";
/// A second org-A branch.
const A_SECOND: &str = "XC-A02";
/// An org-A branch the campaign version does not target.
const A_UNTARGETED: &str = "XC-A09";
/// A branch of org B.
const B_BRANCH: &str = "ORGB-B01";
/// Org B's published USD rates. Distinctive values so a leak is easy to find.
const B_BUY: &str = "41.23450000";
const B_SELL: &str = "42.98760000";
const B_BUY_SHORT: &str = "41.2345";
const B_SELL_SHORT: &str = "42.9876";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

// ------------------------------------------------------------ database

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
        let schema = unique("sim_branch_scope");
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

        // Two more org-A branches (organization_id defaults to org A).
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, 'A02'), ($2, 'A09')")
            .bind(A_SECOND)
            .bind(A_UNTARGETED)
            .execute(&pool)
            .await
            .expect("seed org-A branches");

        // Organisation B with one branch.
        sqlx::query(
            "INSERT INTO organizations (id, code, timezone, created_at) VALUES ($1, 'ORGB', 'Asia/Bangkok', now())",
        )
        .bind(Uuid::parse_str(ORG_B).unwrap())
        .execute(&pool)
        .await
        .expect("seed organisation B");
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, 'B01', $2)")
            .bind(B_BRANCH)
            .bind(Uuid::parse_str(ORG_B).unwrap())
            .execute(&pool)
            .await
            .expect("seed org-B branch");

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

/// Publishes one USD line for `branch`, in the branch's own organisation, and makes it current.
async fn publish_usd(pool: &PgPool, branch: &str, buy: &str, sell: &str) {
    let pub_request = Uuid::new_v4();
    let publication = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
             object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
             status, idempotency_key) \
         SELECT $1, organization_id, 'RATE_PUBLICATION', id, $2, 'pub_hash', \
             '1', '{}'::jsonb, 'test', now(), 'APPROVED', $3 \
         FROM branches WHERE id = $4",
    )
    .bind(pub_request)
    .bind(publication.to_string())
    .bind(unique("pub"))
    .bind(branch)
    .execute(pool)
    .await
    .expect("seed approval_request for publication");

    sqlx::query(
        "INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, \
             approved_request_id, draft_id, published_by) \
         SELECT $1, b.organization_id, b.id, \
             COALESCE((SELECT max(version) FROM rate_publications WHERE branch_id = b.id), 0) + 1, \
             now(), $2, NULL, 'test' \
         FROM branches b WHERE b.id = $3",
    )
    .bind(publication)
    .bind(pub_request)
    .bind(branch)
    .execute(pool)
    .await
    .expect("seed rate_publications");

    sqlx::query(
        "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
             denomination_group, unit_size, buy_rate, sell_rate, source_snapshot) \
         SELECT $1, organization_id, $2, 'USD', 'ALL', 1, $3::numeric, $4::numeric, $5 \
         FROM rate_publications WHERE id = $2",
    )
    .bind(Uuid::new_v4())
    .bind(publication)
    .bind(buy)
    .bind(sell)
    .bind(json!({
        "currency": "USD",
        "buy_mode": "MANUAL",
        "sell_mode": "MANUAL",
        "reason": "seeded by campaign_simulate_branch_scope_test",
    }))
    .execute(pool)
    .await
    .expect("seed published_rate_lines");

    sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
        .bind(publication)
        .bind(branch)
        .execute(pool)
        .await
        .expect("set branch current_publication_id");
}

/// An org-A campaign with one DRAFT version that targets `targets` (`campaign_branches`).
async fn seed_campaign(pool: &PgPool, targets: &[&str]) -> (Uuid, Uuid) {
    let org_id = Uuid::parse_str(ORG_A).unwrap();
    let user_id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, current_version_id, created_by, created_at) \
         VALUES ($1, $2, 'Branch Scope Campaign', NULL, $3, now())",
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
         ) VALUES ($1, $2, $3, 1, 'DRAFT', now() - interval '1 hour', now() + interval '10 days', \
                   10000.00, 'RATE', 1, NULL, $4, now(), now())",
    )
    .bind(version_id)
    .bind(org_id)
    .bind(campaign_id)
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

    for branch in targets {
        sqlx::query(
            "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2)",
        )
        .bind(version_id)
        .bind(*branch)
        .execute(pool)
        .await
        .expect("seed campaign_branches");
    }

    (campaign_id, version_id)
}

/// A 0.25 USD rate-improvement rule on the version (same shape as `mk4_campaign_commands_test`).
async fn add_usd_rule(pool: &PgPool, version_id: Uuid) {
    sqlx::query(
        "INSERT INTO benefit_rules ( \
             id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, caps, rule_no \
         ) VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', '{\"currencies\":[\"USD\"]}', '{\"delta\":\"0.25\"}', '{}', 1)",
    )
    .bind(Uuid::new_v4())
    .bind(version_id)
    .execute(pool)
    .await
    .expect("insert benefit rule");
}

// ------------------------------------------------------------ callers

/// The caller is always an org-A MARKETING user with no branch scope.
fn marketing_of_org_a() -> Actor {
    Actor::new(
        "user-MARKETING",
        ORG_A,
        vec![ScopedRole::new(
            Role::Marketing,
            None,
            chrono::DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

struct HeaderCallers;

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let as_marketing = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v == "MARKETING");
        let user = as_marketing.then(marketing_of_org_a);
        let staff_user = user.clone();
        Box::pin(async move { Callers { user, staff_user } })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }

    fn field(&self) -> &str {
        self.body["error"]["field"].as_str().unwrap_or("")
    }
}

async fn simulate(room: &Room, campaign_id: Uuid, scenarios: Value) -> Answer {
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

    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(&format!("/api/campaigns/{campaign_id}/simulate"))
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((AS, "MARKETING"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header(("idempotency-key", unique("sim_scope")))
        .set_payload(json!({ "scenarios": scenarios }).to_string());

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// Every stored simulation `result` (as text) for the version.
async fn stored_results(pool: &PgPool, version_id: Uuid) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT result::text FROM campaign_simulations WHERE campaign_version_id = $1 \
         ORDER BY simulated_at",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
    .expect("read campaign_simulations")
}

async fn version_status(pool: &PgPool, version_id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM campaign_versions WHERE id = $1")
        .bind(version_id)
        .fetch_one(pool)
        .await
        .expect("read version status")
}

fn scenario(branch: &str, currency: &str) -> Value {
    json!({ "branchId": branch, "currency": currency, "amount": "100.00" })
}

/// The refusal every rejected row shares: 400 `validationFailed` on `field`, nothing written.
async fn assert_refused_nothing_written(
    room: &Room,
    version_id: Uuid,
    ans: &Answer,
    field: &str,
    row: &str,
) {
    let rows = stored_results(&room.pool, version_id).await;
    let status = version_status(&room.pool, version_id).await;
    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "{row}: must be refused with 400; got {} body={} stored results={rows:?} version status={status}",
        ans.status,
        ans.text
    );
    assert_eq!(
        ans.code(),
        "validationFailed",
        "{row}: error code must be validationFailed: {}",
        ans.text
    );
    assert_eq!(
        ans.field(),
        field,
        "{row}: error field must be `{field}`: {}",
        ans.text
    );
    assert!(
        rows.is_empty(),
        "{row}: no campaign_simulations row may be written; found {rows:?}"
    );
    assert_eq!(status, "DRAFT", "{row}: version status must stay DRAFT");
}

// ------------------------------------------------------------ rows

#[actix_web::test]
async fn row1_foreign_org_branch_is_refused_and_no_foreign_rate_leaks() {
    let room = Room::start().await;
    publish_usd(&room.pool, MAIN, "33.50000000", "34.00000000").await;
    publish_usd(&room.pool, B_BRANCH, B_BUY, B_SELL).await;
    // The version's target list names org B's branch too. `campaign_branches.branch_id` references
    // `branches(id)` with no organisation check, so this row is storable; it makes the organisation
    // check the only thing that can refuse the scenario (the target check alone would let it through),
    // and with a USD rule on the version, today's code copies org B's rates into `result`.
    let (campaign_id, version_id) = seed_campaign(&room.pool, &[MAIN, B_BRANCH]).await;
    add_usd_rule(&room.pool, version_id).await;

    let ans = simulate(&room, campaign_id, json!([scenario(B_BRANCH, "USD")])).await;

    assert_refused_nothing_written(&room, version_id, &ans, "branchId", "row 1 (org-B branch)")
        .await;
    for leak in [B_BUY_SHORT, B_SELL_SHORT] {
        assert!(
            !ans.text.contains(leak),
            "row 1: the response must not carry org B's rate {leak}: {}",
            ans.text
        );
    }
    let leaked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM campaign_simulations \
         WHERE result::text LIKE '%' || $1 || '%' OR result::text LIKE '%' || $2 || '%' \
            OR input::text LIKE '%' || $3 || '%'",
    )
    .bind(B_BUY_SHORT)
    .bind(B_SELL_SHORT)
    .bind(B_BRANCH)
    .fetch_one(&room.pool)
    .await
    .expect("scan campaign_simulations for org B data");
    assert_eq!(
        leaked, 0,
        "row 1: no stored simulation may hold org B's branch or rates"
    );

    room.finish().await;
}

#[actix_web::test]
async fn row2_unknown_branch_is_refused() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_campaign(&room.pool, &[MAIN]).await;

    let ans = simulate(
        &room,
        campaign_id,
        json!([scenario("NO-SUCH-BRANCH", "USD")]),
    )
    .await;

    assert_refused_nothing_written(
        &room,
        version_id,
        &ans,
        "branchId",
        "row 2 (unknown branch)",
    )
    .await;
    room.finish().await;
}

#[actix_web::test]
async fn row3_same_org_branch_not_targeted_by_version_is_refused() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_campaign(&room.pool, &[MAIN]).await;

    let ans = simulate(&room, campaign_id, json!([scenario(A_UNTARGETED, "USD")])).await;

    assert_refused_nothing_written(
        &room,
        version_id,
        &ans,
        "branchId",
        "row 3 (same-org branch not targeted)",
    )
    .await;
    room.finish().await;
}

#[actix_web::test]
async fn row4_valid_then_foreign_scenario_is_refused_on_branch_id() {
    let room = Room::start().await;
    publish_usd(&room.pool, B_BRANCH, B_BUY, B_SELL).await;
    // Org B's branch is in the target list (see row 1), so only the organisation check refuses it.
    let (campaign_id, version_id) = seed_campaign(&room.pool, &[MAIN, B_BRANCH]).await;

    let ans = simulate(
        &room,
        campaign_id,
        json!([scenario(MAIN, "USD"), scenario(B_BRANCH, "USD")]),
    )
    .await;

    assert_refused_nothing_written(
        &room,
        version_id,
        &ans,
        "branchId",
        "row 4 (valid then org-B scenario)",
    )
    .await;
    room.finish().await;
}

#[actix_web::test]
async fn row5_targeted_same_org_branch_simulates() {
    let room = Room::start().await;
    publish_usd(&room.pool, MAIN, "33.50000000", "34.00000000").await;
    let (campaign_id, version_id) = seed_campaign(&room.pool, &[MAIN]).await;

    let ans = simulate(&room, campaign_id, json!([scenario(MAIN, "USD")])).await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "row 5: a targeted same-org branch must simulate: {}",
        ans.text
    );
    assert!(
        ans.body["simulationId"].as_str().is_some(),
        "row 5: response must carry simulationId: {}",
        ans.text
    );
    assert_eq!(
        stored_results(&room.pool, version_id).await.len(),
        1,
        "row 5: exactly one campaign_simulations row"
    );
    assert_eq!(
        version_status(&room.pool, version_id).await,
        "SIMULATED",
        "row 5: DRAFT must become SIMULATED"
    );
    room.finish().await;
}

#[actix_web::test]
async fn row6_two_distinct_targeted_branches_are_refused_on_scenarios() {
    let room = Room::start().await;
    publish_usd(&room.pool, MAIN, "33.50000000", "34.00000000").await;
    let (campaign_id, version_id) = seed_campaign(&room.pool, &[MAIN, A_SECOND]).await;

    let ans = simulate(
        &room,
        campaign_id,
        json!([scenario(MAIN, "USD"), scenario(A_SECOND, "USD")]),
    )
    .await;

    assert_refused_nothing_written(
        &room,
        version_id,
        &ans,
        "scenarios",
        "row 6 (two distinct targeted branches)",
    )
    .await;
    room.finish().await;
}

#[actix_web::test]
async fn row7_two_scenarios_on_one_targeted_branch_simulate() {
    let room = Room::start().await;
    publish_usd(&room.pool, MAIN, "33.50000000", "34.00000000").await;
    let (campaign_id, version_id) = seed_campaign(&room.pool, &[MAIN, A_SECOND]).await;

    // Both scenarios priceable: MAIN publishes USD only, and since #1993 an unpublished currency
    // is refused with 409 (ruling claude/7c9b7042, 2026-10-16).
    let ans = simulate(
        &room,
        campaign_id,
        json!([scenario(MAIN, "USD"), scenario(MAIN, "USD")]),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "row 7: two scenarios on the same targeted branch must simulate: {}",
        ans.text
    );
    assert_eq!(
        stored_results(&room.pool, version_id).await.len(),
        1,
        "row 7: exactly one campaign_simulations row"
    );
    assert_eq!(
        version_status(&room.pool, version_id).await,
        "SIMULATED",
        "row 7: DRAFT must become SIMULATED"
    );
    room.finish().await;
}
