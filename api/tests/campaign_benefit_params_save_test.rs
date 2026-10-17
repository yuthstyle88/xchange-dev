//! Up-front tests: `campaignVersionSave` refuses malformed benefit-rule params (ticket A).
//!
//! Design `docs/design/blueprint/marketing-campaigns.md`:
//! - §4.5 `campaignVersionSave` (DRAFT only: dates, branches, budget, caps, stack, priority, **rules**, snapshot);
//! - §4.1 `benefit_rules.benefit_type` "validated against the registry", `calculation` "the calculator's
//!   params, decimals as strings";
//! - D-6 benefit types are one trait (`BenefitCalculator`), one file per type (♻️).
//!
//! Written before the fix by the test writer; the implementer must not edit, delete or weaken them.
//!
//! Request shape `[proposed]`: `rules` is an optional array on the save body. Each item is
//! `{ benefitType, eligibility, calculation, caps, requiresCoupon }` (the write half of the
//! `campaign` view's `CampaignRuleDetail`). `benefit_class` comes from the calculator kind,
//! not from the request. `rules` present ⇒ it replaces the draft's rules in order (`rule_no` 1..n).
//! `rules` absent ⇒ the stored rules are untouched.
//!
//! Field names `[proposed]`: `V3Error.field` is `&'static str`, so the field is the path
//! without the index (`rules.calculation.percent`). The message names the index (`rules[1]`).
//!
//! Behaviour rows:
//! | case | answer |
//! |---|---|
//! | FEE_WAIVER `percent` not a decimal string ("abc") | 400 `validationFailed`, `rules.calculation.percent`, nothing written |
//! | FEE_WAIVER `percent` not a string (`true`) | same, `rules.calculation.percent` |
//! | FEE_WAIVER `amount` not a decimal string | same, `rules.calculation.amount` |
//! | FEE_WAIVER `fixed` not a string (`{}`) | same, `rules.calculation.fixed` |
//! | FEE_WAIVER with none of `percent`/`amount`/`fixed` | same, `rules.calculation` |
//! | RATE_IMPROVEMENT `delta` "abc" / `true` | same, `rules.calculation.delta` |
//! | RATE_IMPROVEMENT `percent` "abc" / `true` | same, `rules.calculation.percent` |
//! | RATE_IMPROVEMENT with neither `delta` nor `percent` | same, `rules.calculation` |
//! | a param given as a JSON number, not a decimal string (ruling R3) | same, `rules.calculation.<key>` |
//! | `benefitType` the registry does not know | same, `rules.benefitType` |
//! | the 2nd rule malformed | the message names `rules[1]`; the 1st rule is not written either |
//! | each well-formed kind/key | 200, rule stored with that calculation and the kind's class |
//! | no `rules` in the body | 200, the stored rules untouched (as today) |

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
use xc_kernel::ids::BranchId;
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
const SEEDED_BUDGET: &str = "10000.00";
const SEEDED_CALC: &str = r#"{"delta":"0.11"}"#;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
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
        let schema = unique("benefit_save");
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

fn marketing() -> Actor {
    Actor::new(
        "user-MARKETING",
        ORG,
        vec![ScopedRole::new(
            Role::Marketing,
            None::<BranchId>,
            chrono::DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
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
        let is_marketing = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v == "MARKETING");
        let user = is_marketing.then(marketing);
        Box::pin(async move {
            Callers {
                owner: None,
                staff: None,
                staff_user: user.clone(),
                user,
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
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }

    fn field(&self) -> &str {
        self.body["error"]["field"].as_str().unwrap_or("")
    }

    fn message(&self) -> &str {
        self.body["error"]["message"].as_str().unwrap_or("")
    }
}

async fn save(room: &Room, campaign_id: Uuid, body: &Value) -> Answer {
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
        .method(Method::PUT)
        .uri(&format!("/api/campaigns/{campaign_id}/draft"))
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((AS, "MARKETING"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header(("idempotency-key", unique("save")))
        .set_payload(body.to_string());

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// A DRAFT campaign on MAIN with one stored, well-formed rule (`rule_no` 1, `SEEDED_CALC`).
async fn seed_draft(pool: &PgPool) -> (Uuid, Uuid) {
    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, current_version_id, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, NULL, $4::uuid, now())",
    )
    .bind(campaign_id)
    .bind(ORG)
    .bind(unique("campaign"))
    .bind(USER)
    .execute(pool)
    .await
    .expect("seed campaign");

    sqlx::query(
        "INSERT INTO campaign_versions ( \
             id, organization_id, campaign_id, version, status, starts_at, ends_at, budget_thb, \
             stack_group, priority, version_hash, created_by, created_at, edited_at \
         ) VALUES ($1, $2::uuid, $3, 1, 'DRAFT', now() - interval '1 hour', now() + interval '10 days', \
                   $4::numeric, 'RATE', 1, NULL, $5::uuid, now(), now())",
    )
    .bind(version_id)
    .bind(ORG)
    .bind(campaign_id)
    .bind(SEEDED_BUDGET)
    .bind(USER)
    .execute(pool)
    .await
    .expect("seed version");

    sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
        .bind(version_id)
        .bind(campaign_id)
        .execute(pool)
        .await
        .expect("set current version");

    sqlx::query(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) VALUES ($1, 0, 0)",
    )
    .bind(version_id)
    .execute(pool)
    .await
    .expect("seed budget");

    sqlx::query(
        "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, \
             calculation, caps, requires_coupon, rule_no) \
         VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', '{\"currencies\":[\"USD\"]}'::jsonb, $3::jsonb, \
             '{}'::jsonb, false, 1)",
    )
    .bind(Uuid::new_v4())
    .bind(version_id)
    .bind(SEEDED_CALC)
    .execute(pool)
    .await
    .expect("seed rule");

    (campaign_id, version_id)
}

fn rule(benefit_type: &str, calculation: Value) -> Value {
    json!({
        "benefitType": benefit_type,
        "eligibility": { "currencies": ["USD"] },
        "calculation": calculation,
        "caps": {},
        "requiresCoupon": false
    })
}

fn body_with(rules: Option<Vec<Value>>) -> Value {
    let mut body = json!({
        "startsAt": "2026-11-01T00:00:00Z",
        "endsAt": "2026-12-01T00:00:00Z",
        "budgetThb": "20000.00",
        "stackGroup": "RATE",
        "priority": 2,
        "branches": [MAIN]
    });
    if let Some(rules) = rules {
        body["rules"] = Value::Array(rules);
    }
    body
}

/// `(benefit_type, benefit_class, calculation, rule_no)` of the version's rules, in `rule_no` order.
async fn stored_rules(pool: &PgPool, version_id: Uuid) -> Vec<(String, String, Value, i32)> {
    sqlx::query_as(
        "SELECT benefit_type, benefit_class, calculation, rule_no FROM benefit_rules \
         WHERE campaign_version_id = $1 ORDER BY rule_no",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
    .expect("read stored rules")
}

async fn stored_budget(pool: &PgPool, version_id: Uuid) -> Decimal {
    sqlx::query_scalar("SELECT budget_thb FROM campaign_versions WHERE id = $1")
        .bind(version_id)
        .fetch_one(pool)
        .await
        .expect("read budget")
}

/// Saves one malformed rule and expects 400 `validationFailed` on `field`, with nothing written:
/// the seeded rule is still the only one, and the version's budget is unchanged.
async fn refused(benefit_type: &str, calculation: Value, field: &str) {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        &body_with(Some(vec![rule(benefit_type, calculation.clone())])),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "{benefit_type} {calculation} must be refused with 400: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "{}", ans.text);
    assert_eq!(ans.field(), field, "{}", ans.text);
    assert_eq!(
        stored_rules(&room.pool, version_id).await,
        vec![(
            "RATE_IMPROVEMENT".to_owned(),
            "PRICE".to_owned(),
            serde_json::from_str::<Value>(SEEDED_CALC).expect("seed calc"),
            1
        )],
        "a refused save must not replace or add rules"
    );
    assert_eq!(
        stored_budget(&room.pool, version_id).await,
        Decimal::from_str(SEEDED_BUDGET).expect("decimal"),
        "a refused save must not write the version's terms either"
    );

    room.finish().await;
}

/// Saves one well-formed rule and expects 200 with exactly that rule stored as `rule_no` 1.
async fn stored(benefit_type: &str, calculation: Value) {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        &body_with(Some(vec![rule(benefit_type, calculation.clone())])),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "{benefit_type} {calculation} is well formed and must save: {}",
        ans.text
    );
    assert_eq!(
        stored_rules(&room.pool, version_id).await,
        vec![(benefit_type.to_owned(), "PRICE".to_owned(), calculation, 1)],
        "the saved rules replace the draft's rules, class taken from the calculator kind"
    );

    room.finish().await;
}

// ── FEE_WAIVER refusals (reader sites fee_waiver.rs:25, :29, :41, :45, :48) ──

#[actix_web::test]
async fn fee_waiver_percent_not_a_decimal_string_is_refused() {
    refused(
        "FEE_WAIVER",
        json!({ "percent": "abc" }),
        "rules.calculation.percent",
    )
    .await;
}

#[actix_web::test]
async fn fee_waiver_percent_not_a_string_is_refused() {
    refused(
        "FEE_WAIVER",
        json!({ "percent": true }),
        "rules.calculation.percent",
    )
    .await;
}

#[actix_web::test]
async fn fee_waiver_amount_not_a_decimal_string_is_refused() {
    refused(
        "FEE_WAIVER",
        json!({ "amount": "abc" }),
        "rules.calculation.amount",
    )
    .await;
}

#[actix_web::test]
async fn fee_waiver_fixed_not_a_string_is_refused() {
    refused(
        "FEE_WAIVER",
        json!({ "fixed": {} }),
        "rules.calculation.fixed",
    )
    .await;
}

#[actix_web::test]
async fn fee_waiver_without_percent_amount_or_fixed_is_refused() {
    refused("FEE_WAIVER", json!({ "pct": "0.5" }), "rules.calculation").await;
}

// ── RATE_IMPROVEMENT refusals (reader sites rate_improvement.rs:25, :29, :33, :37, :40) ──

#[actix_web::test]
async fn rate_improvement_delta_not_a_decimal_string_is_refused() {
    refused(
        "RATE_IMPROVEMENT",
        json!({ "delta": "abc" }),
        "rules.calculation.delta",
    )
    .await;
}

#[actix_web::test]
async fn rate_improvement_delta_not_a_string_is_refused() {
    refused(
        "RATE_IMPROVEMENT",
        json!({ "delta": true }),
        "rules.calculation.delta",
    )
    .await;
}

#[actix_web::test]
async fn rate_improvement_percent_not_a_decimal_string_is_refused() {
    refused(
        "RATE_IMPROVEMENT",
        json!({ "percent": "abc" }),
        "rules.calculation.percent",
    )
    .await;
}

#[actix_web::test]
async fn rate_improvement_percent_not_a_string_is_refused() {
    refused(
        "RATE_IMPROVEMENT",
        json!({ "percent": [] }),
        "rules.calculation.percent",
    )
    .await;
}

#[actix_web::test]
async fn rate_improvement_without_delta_or_percent_is_refused() {
    refused(
        "RATE_IMPROVEMENT",
        json!({ "type": "FIXED_RATE" }),
        "rules.calculation",
    )
    .await;
}

// ── JSON numbers are malformed (ruling R3: design §4.1 "decimals as strings") ──

#[actix_web::test]
async fn fee_waiver_percent_as_json_number_is_refused() {
    refused(
        "FEE_WAIVER",
        json!({ "percent": 0.5 }),
        "rules.calculation.percent",
    )
    .await;
}

#[actix_web::test]
async fn fee_waiver_amount_as_json_integer_is_refused() {
    refused(
        "FEE_WAIVER",
        json!({ "amount": 30 }),
        "rules.calculation.amount",
    )
    .await;
}

#[actix_web::test]
async fn fee_waiver_fixed_as_json_number_is_refused() {
    refused(
        "FEE_WAIVER",
        json!({ "fixed": 25.5 }),
        "rules.calculation.fixed",
    )
    .await;
}

#[actix_web::test]
async fn rate_improvement_delta_as_json_integer_is_refused() {
    refused(
        "RATE_IMPROVEMENT",
        json!({ "delta": 1 }),
        "rules.calculation.delta",
    )
    .await;
}

#[actix_web::test]
async fn rate_improvement_percent_as_json_number_is_refused() {
    refused(
        "RATE_IMPROVEMENT",
        json!({ "percent": 0.01 }),
        "rules.calculation.percent",
    )
    .await;
}

// ── registry and position ──

#[actix_web::test]
async fn unknown_benefit_type_is_refused() {
    refused("CASHBACK", json!({ "delta": "0.10" }), "rules.benefitType").await;
}

#[actix_web::test]
async fn second_rule_malformed_names_its_index_and_writes_neither_rule() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        &body_with(Some(vec![
            rule("FEE_WAIVER", json!({ "percent": "0.5" })),
            rule("RATE_IMPROVEMENT", json!({ "delta": "abc" })),
        ])),
    )
    .await;

    assert_eq!(ans.status, StatusCode::BAD_REQUEST, "{}", ans.text);
    assert_eq!(ans.code(), "validationFailed", "{}", ans.text);
    assert_eq!(ans.field(), "rules.calculation.delta", "{}", ans.text);
    assert!(
        ans.message().contains("rules[1]"),
        "the message must name the failing rule's index: {}",
        ans.text
    );
    let rules = stored_rules(&room.pool, version_id).await;
    assert_eq!(rules.len(), 1, "only the seeded rule remains: {rules:?}");
    assert_eq!(rules[0].0, "RATE_IMPROVEMENT", "{rules:?}");

    room.finish().await;
}

// ── well-formed params are stored (new behaviour: on main the `rules` field is dropped) ──

#[actix_web::test]
async fn fee_waiver_percent_well_formed_is_stored() {
    stored("FEE_WAIVER", json!({ "percent": "0.5" })).await;
}

#[actix_web::test]
async fn fee_waiver_amount_well_formed_is_stored() {
    stored("FEE_WAIVER", json!({ "amount": "30.00" })).await;
}

#[actix_web::test]
async fn fee_waiver_fixed_well_formed_is_stored() {
    stored("FEE_WAIVER", json!({ "fixed": "30" })).await;
}

#[actix_web::test]
async fn rate_improvement_delta_well_formed_is_stored() {
    stored(
        "RATE_IMPROVEMENT",
        json!({ "type": "FIXED_RATE", "delta": "0.25" }),
    )
    .await;
}

#[actix_web::test]
async fn rate_improvement_percent_well_formed_is_stored() {
    stored("RATE_IMPROVEMENT", json!({ "percent": "0.01" })).await;
}

#[actix_web::test]
async fn two_well_formed_rules_are_stored_in_order() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        &body_with(Some(vec![
            rule("FEE_WAIVER", json!({ "percent": "1" })),
            rule("RATE_IMPROVEMENT", json!({ "delta": "0.20" })),
        ])),
    )
    .await;

    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(
        stored_rules(&room.pool, version_id).await,
        vec![
            (
                "FEE_WAIVER".to_owned(),
                "PRICE".to_owned(),
                json!({ "percent": "1" }),
                1
            ),
            (
                "RATE_IMPROVEMENT".to_owned(),
                "PRICE".to_owned(),
                json!({ "delta": "0.20" }),
                2
            ),
        ]
    );

    room.finish().await;
}

// ── keep-pin: a save without `rules` behaves as today ──

#[actix_web::test]
async fn save_without_rules_keeps_the_stored_rules() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(&room, campaign_id, &body_with(None)).await;

    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(ans.body["version"], 1, "{}", ans.text);
    assert_eq!(
        stored_rules(&room.pool, version_id).await,
        vec![(
            "RATE_IMPROVEMENT".to_owned(),
            "PRICE".to_owned(),
            serde_json::from_str::<Value>(SEEDED_CALC).expect("seed calc"),
            1
        )]
    );
    assert_eq!(
        stored_budget(&room.pool, version_id).await,
        Decimal::from_str("20000.00").expect("decimal"),
        "the terms are saved"
    );

    room.finish().await;
}
