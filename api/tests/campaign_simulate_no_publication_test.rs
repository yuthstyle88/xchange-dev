//! #1993 up-front tests: `campaignSimulate` refuses a scenario the real quote would refuse.
//!
//! Written before the fix by the ticket author's test writer (for claude/d7aa64). The implementer
//! must not edit, delete or weaken these tests; add tests in a file of your own.
//!
//! Ruling (BE lane controller claude/7ca80c, 2026-10-15): a scenario whose branch has no current
//! publication, or whose publication has no line for the currency, is refused with the same answer
//! the real quote gives, and nothing is written. Rate 1.0 never appears. Design D-7
//! (`docs/design/blueprint/marketing-campaigns.md:77`): "simulate equals the real quote".
//!
//! The real quote's answer, measured at main `40303f11`:
//! - `api/src/application/workflows/exchange/quote.rs:198-221`: `line_for` returns `None` (no
//!   current publication, or no line for the currency) ⇒ `AppError::Domain("rateUnavailable: no
//!   published line")`.
//! - `api/src/presentation/http/v3/commands/quotes.rs:214-221`: a `Domain` message starting with
//!   `rateUnavailable` ⇒ HTTP **409**, code **`rateUnavailable`**, field `None`.
//!
//! Rows:
//! 1. `branch_without_publication_is_refused_like_the_quote`: MAIN has no current publication ⇒
//!    409 `rateUnavailable`, 0 `campaign_simulations` rows, version stays `DRAFT`, `simulated_at` NULL.
//! 2. `currency_without_published_line_is_refused_like_the_quote`: MAIN publishes USD only, the
//!    scenario asks EUR ⇒ the same refusal, nothing written.
//! 3. `one_unpriceable_scenario_refuses_the_whole_simulation`: first scenario priced (MAIN/USD),
//!    second on MAIN in a currency MAIN has not published (EUR) ⇒ the same refusal, 0 rows,
//!    status unchanged. (Same branch on purpose: #1992 refuses two branches with 400 first.)
//! 4. `priced_scenario_is_simulated_at_the_published_rate`: MAIN publishes USD 33.50/34.00 ⇒ 200,
//!    one row, version `SIMULATED`, and the stored result's `rateBefore` equals the published buy
//!    rate (never 1.0) and `rateAfter` is that rate plus the rule's 0.25.

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
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const USER: &str = "00000000-0000-0000-0000-000000000002";
const MAIN: &str = "MAIN";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

/// What the real quote answers when it has no published line (quotes.rs:214-221).
const QUOTE_REFUSAL_STATUS: StatusCode = StatusCode::CONFLICT;
const QUOTE_REFUSAL_CODE: &str = "rateUnavailable";

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
    Decimal::from_str(s).unwrap_or_else(|e| panic!("'{s}' is not a decimal: {e}"))
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
        let schema = unique("sim_nopub");
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
             VALUES ($1::uuid, $2::uuid, 'Test User', 'ACTIVE', 1, now(), 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(USER)
        .bind(ORG)
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
    let role = match spec {
        "MARKETING" => Role::Marketing,
        _ => return None,
    };
    let from = chrono::DateTime::<Utc>::UNIX_EPOCH;
    Some(Actor::new(
        &format!("user-{}", role.as_str()),
        ORG,
        vec![ScopedRole::new(role, None, from, None)],
        None,
    ))
}

struct HeaderCallers;

impl SessionReader for HeaderCallers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let user = user_of(spec);
        let staff_user = user.clone();
        Box::pin(async move {
            Callers {
                owner: None,
                staff: None,
                user,
                staff_user,
            }
        })
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
        .insert_header(("idempotency-key", unique("sim1993")))
        .set_payload(json!({ "scenarios": scenarios }).to_string());

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// A DRAFT campaign version on `branches`, with one RATE_IMPROVEMENT rule (+0.25 on USD and EUR),
/// so a priced scenario stores an applied line carrying `rateBefore` / `rateAfter`.
async fn seed_draft_campaign(pool: &PgPool, branches: &[&str]) -> (Uuid, Uuid) {
    let org_id = Uuid::parse_str(ORG).unwrap();
    let user_id = Uuid::parse_str(USER).unwrap();
    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, current_version_id, created_by, created_at) \
         VALUES ($1, $2, 'No-publication campaign', NULL, $3, now())",
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
        .expect("set current_version_id");

    sqlx::query(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) \
         VALUES ($1, 0, 0)",
    )
    .bind(version_id)
    .execute(pool)
    .await
    .expect("seed budget");

    for b in branches {
        sqlx::query(
            "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2)",
        )
        .bind(version_id)
        .bind(*b)
        .execute(pool)
        .await
        .expect("seed campaign branch");
    }

    sqlx::query(
        "INSERT INTO benefit_rules ( \
             id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, caps, rule_no \
         ) VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', '{\"currencies\":[\"USD\",\"EUR\"]}', \
                   '{\"delta\":\"0.25\"}', '{}', 1)",
    )
    .bind(Uuid::new_v4())
    .bind(version_id)
    .execute(pool)
    .await
    .expect("seed benefit rule");

    (campaign_id, version_id)
}

/// Publishes `lines` (currency, buy, sell) at unit size 1 as `branch`'s current publication.
async fn publish(pool: &PgPool, branch: &str, lines: &[(&str, &str, &str)]) {
    let request = Uuid::new_v4();
    let publication = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
             object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
             status, idempotency_key) \
         SELECT $1, organization_id, 'RATE_PUBLICATION', id, $2, 'pub_hash', \
             '1', '{}'::jsonb, 'test', now(), 'APPROVED', $3 \
         FROM branches WHERE id = $4",
    )
    .bind(request)
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
    .bind(request)
    .bind(branch)
    .execute(pool)
    .await
    .expect("seed rate_publications");

    for (currency, buy, sell) in lines {
        sqlx::query(
            "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
                 denomination_group, unit_size, buy_rate, sell_rate, source_snapshot) \
             SELECT $1, organization_id, $2, $3, 'ALL', 1, $4::numeric, $5::numeric, \
                 jsonb_build_object('currency', $3::text, 'buy_mode', 'MANUAL', 'sell_mode', 'MANUAL') \
             FROM rate_publications WHERE id = $2",
        )
        .bind(Uuid::new_v4())
        .bind(publication)
        .bind(*currency)
        .bind(*buy)
        .bind(*sell)
        .execute(pool)
        .await
        .expect("seed published_rate_lines");
    }

    sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
        .bind(publication)
        .bind(branch)
        .execute(pool)
        .await
        .expect("set current_publication_id");
}

async fn has_publication(pool: &PgPool, branch: &str) -> bool {
    sqlx::query_scalar("SELECT current_publication_id IS NOT NULL FROM branches WHERE id = $1")
        .bind(branch)
        .fetch_one(pool)
        .await
        .expect("read branch")
}

async fn simulation_rows(pool: &PgPool, version_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM campaign_simulations WHERE campaign_version_id = $1")
        .bind(version_id)
        .fetch_one(pool)
        .await
        .expect("count simulations")
}

async fn version_state(pool: &PgPool, version_id: Uuid) -> (String, bool) {
    sqlx::query_as("SELECT status, simulated_at IS NOT NULL FROM campaign_versions WHERE id = $1")
        .bind(version_id)
        .fetch_one(pool)
        .await
        .expect("read version")
}

/// The refusal the real quote gives, and nothing written (ruling of 2026-10-15).
async fn assert_refused_like_the_quote(room: &Room, version_id: Uuid, ans: &Answer, row: &str) {
    assert_eq!(
        ans.status, QUOTE_REFUSAL_STATUS,
        "{row}: campaignSimulate must refuse with the real quote's status 409 \
         (quotes.rs:214-221), never price at rate 1.0: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        QUOTE_REFUSAL_CODE,
        "{row}: campaignSimulate must refuse with the real quote's code `rateUnavailable`: {}",
        ans.text
    );
    assert_eq!(
        simulation_rows(&room.pool, version_id).await,
        0,
        "{row}: a refused simulation must write no campaign_simulations row"
    );
    assert_eq!(
        version_state(&room.pool, version_id).await,
        ("DRAFT".to_string(), false),
        "{row}: a refused simulation must leave the version DRAFT with simulated_at NULL"
    );
}

#[actix_web::test]
async fn branch_without_publication_is_refused_like_the_quote() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft_campaign(&room.pool, &[MAIN]).await;
    assert!(
        !has_publication(&room.pool, MAIN).await,
        "precondition: a fresh schema gives MAIN no current publication"
    );

    let ans = simulate(
        &room,
        campaign_id,
        json!([{ "branchId": MAIN, "currency": "USD", "amount": "100.00" }]),
    )
    .await;

    assert_refused_like_the_quote(&room, version_id, &ans, "row 1 (no publication)").await;
    room.finish().await;
}

#[actix_web::test]
async fn currency_without_published_line_is_refused_like_the_quote() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft_campaign(&room.pool, &[MAIN]).await;
    publish(&room.pool, MAIN, &[("USD", "33.50000000", "34.00000000")]).await;

    let ans = simulate(
        &room,
        campaign_id,
        json!([{ "branchId": MAIN, "currency": "EUR", "amount": "100.00" }]),
    )
    .await;

    assert_refused_like_the_quote(&room, version_id, &ans, "row 2 (no line for EUR)").await;
    room.finish().await;
}

#[actix_web::test]
async fn one_unpriceable_scenario_refuses_the_whole_simulation() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft_campaign(&room.pool, &[MAIN]).await;
    // One branch only: #1992 refuses scenarios on two branches with 400 before any rate is read,
    // so the unpriceable scenario is a currency MAIN has not published (ruling claude/7c9b7042,
    // 2026-10-16, #1993).
    publish(&room.pool, MAIN, &[("USD", "33.50000000", "34.00000000")]).await;

    let ans = simulate(
        &room,
        campaign_id,
        json!([
            { "branchId": MAIN, "currency": "USD", "amount": "100.00" },
            { "branchId": MAIN, "currency": "EUR", "amount": "100.00" }
        ]),
    )
    .await;

    assert_refused_like_the_quote(
        &room,
        version_id,
        &ans,
        "row 3 (second scenario unpriceable)",
    )
    .await;
    room.finish().await;
}

#[actix_web::test]
async fn priced_scenario_is_simulated_at_the_published_rate() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft_campaign(&room.pool, &[MAIN]).await;
    publish(&room.pool, MAIN, &[("USD", "33.50000000", "34.00000000")]).await;

    let ans = simulate(
        &room,
        campaign_id,
        json!([{ "branchId": MAIN, "currency": "USD", "amount": "100.00" }]),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "row 4: a priced scenario must simulate: {}",
        ans.text
    );
    assert_eq!(
        simulation_rows(&room.pool, version_id).await,
        1,
        "row 4: a priced simulation writes one campaign_simulations row"
    );
    assert_eq!(
        version_state(&room.pool, version_id).await,
        ("SIMULATED".to_string(), true),
        "row 4: a priced simulation moves DRAFT to SIMULATED and stamps simulated_at"
    );

    let result: Value = sqlx::query_scalar(
        "SELECT result FROM campaign_simulations WHERE campaign_version_id = $1",
    )
    .bind(version_id)
    .fetch_one(&room.pool)
    .await
    .expect("read stored result");
    let applied = result["applied"]
        .as_array()
        .unwrap_or_else(|| panic!("row 4: stored result has no `applied` array: {result}"));
    assert_eq!(
        applied.len(),
        1,
        "row 4: the +0.25 USD rule applies to the priced scenario: {result}"
    );
    let rate_before = d(applied[0]["rateBefore"]
        .as_str()
        .unwrap_or_else(|| panic!("row 4: applied line has no rateBefore: {result}")));
    let rate_after = d(applied[0]["rateAfter"]
        .as_str()
        .unwrap_or_else(|| panic!("row 4: applied line has no rateAfter: {result}")));
    assert_ne!(
        rate_before,
        Decimal::ONE,
        "row 4: the stored rate must never be the 1.0 fallback: {result}"
    );
    assert_eq!(
        rate_before,
        d("33.50"),
        "row 4: the stored rateBefore must equal the published buy rate: {result}"
    );
    assert_eq!(
        rate_after,
        d("33.75"),
        "row 4: the stored rateAfter must be the published buy rate plus 0.25: {result}"
    );

    room.finish().await;
}
