//! Up-front tests for #2782 (ruling A, BE controller claude/1656900d, 2026-10-17):
//! `campaignVersionSave` refuses a `rules` replacement while any current rule of that draft is
//! referenced by a row of `coupons`, `benefit_reservations` or `benefit_redemptions`.
//!
//! Why: the replacement deletes the draft's rules, and those three tables point at `benefit_rules(id)`
//! with no `ON DELETE` (`api/migrations/20261205161000_benefit_budgets.sql`). On `main` such a save
//! answers 500 `database`. The ruling: 400 `validationFailed` on field `rules`, with a Thai message,
//! decided **before any write**, so nothing of the request is stored.
//!
//! Written before the fix by the test writer; the implementer must not edit, delete or weaken them.
//!
//! Behaviour rows:
//! | case | answer |
//! |---|---|
//! | an ISSUED coupon on the draft's rule, save with `rules` | 400 `validationFailed`, field `rules`, Thai message; nothing written |
//! | a VOID coupon on the draft's rule (any status counts: the row still points at the rule) | same |
//! | a reservation on the draft's rule, no coupon | same |
//! | a redemption on the draft's rule, its reservation on another rule | same |
//! | the draft is SIMULATED, a coupon on its rule | same |
//! | `rules: []` (removes every rule), a coupon on the draft's rule | same |
//! | a coupon only on a rule of **another** campaign | 200, the draft's rules stored as sent |
//! | a coupon only on a rule of an **older, approved** version of the same campaign | 200, stored as sent |
//! | no reference at all | 200, stored as sent (keep) |
//! | a coupon on the draft's rule, save **without** `rules` | 200, the terms saved, the rules and the coupon untouched (keep) |
//!
//! "Nothing written" = the version's terms (status, dates, budget, stack group, priority, `edited_at`,
//! `version_hash`), its branches, its rules (ids, calculations, `rule_no`) and every referencing row
//! read back exactly as before the request.
//!
//! The reservation and redemption rows are stored with `session_replication_role = replica` for that
//! one insert (as `xc-cash/tests/reconcile_places_test.rs` does), so they need no quote, transaction or
//! reservation chain of their own; their `rule_id` is a real rule. The save itself runs with every
//! constraint on.

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
        let schema = unique("rules_in_use");
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

const IN_USE_FIELD: &str = "rules";
const NEW_CALC: &str = r#"{"delta":"0.10"}"#;

/// Everything a refused save must leave as it was.
#[derive(Debug, PartialEq)]
struct Written {
    /// status, starts_at, ends_at, budget_thb, stack_group, priority, edited_at, version_hash
    terms: (
        String,
        String,
        String,
        String,
        String,
        i32,
        String,
        Option<String>,
    ),
    branches: Vec<String>,
    /// (id, benefit_type, calculation, rule_no)
    rules: Vec<(Uuid, String, Value, i32)>,
    /// (table, row id, rule id)
    references: Vec<(String, Uuid, Uuid)>,
}

async fn written(pool: &PgPool, version_id: Uuid) -> Written {
    let terms = sqlx::query_as(
        "SELECT status, starts_at::text, ends_at::text, budget_thb::text, stack_group, priority, \
                edited_at::text, version_hash \
         FROM campaign_versions WHERE id = $1",
    )
    .bind(version_id)
    .fetch_one(pool)
    .await
    .expect("read the version's terms");
    let branches = sqlx::query_scalar(
        "SELECT branch_id FROM campaign_branches WHERE campaign_version_id = $1 ORDER BY branch_id",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
    .expect("read the version's branches");
    let rules = sqlx::query_as(
        "SELECT id, benefit_type, calculation, rule_no FROM benefit_rules \
         WHERE campaign_version_id = $1 ORDER BY rule_no",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
    .expect("read the version's rules");
    let references = sqlx::query_as(
        "SELECT 'coupons', id, rule_id FROM coupons \
         UNION ALL SELECT 'benefit_reservations', id, rule_id FROM benefit_reservations \
         UNION ALL SELECT 'benefit_redemptions', id, rule_id FROM benefit_redemptions \
         ORDER BY 1, 2",
    )
    .fetch_all(pool)
    .await
    .expect("read the referencing rows");
    Written {
        terms,
        branches,
        rules,
        references,
    }
}

async fn rule_ids(pool: &PgPool, version_id: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT id FROM benefit_rules WHERE campaign_version_id = $1 ORDER BY rule_no",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
    .expect("read rule ids")
}

/// A coupon in `status` on `rule`, as `couponIssue` would store it.
async fn coupon_on(pool: &PgPool, rule: Uuid, status: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO coupons (id, organization_id, rule_id, code_hash, customer_id, expires_at, status, \
             issued_by, issued_at) \
         VALUES ($1, $2::uuid, $3, $4, NULL, now() + interval '30 days', $5, $6::uuid, now())",
    )
    .bind(id)
    .bind(ORG)
    .bind(rule)
    .bind(unique("code").into_bytes())
    .bind(status)
    .bind(USER)
    .execute(pool)
    .await
    .expect("a coupon");
    id
}

/// Runs `sql` (bound to `binds`) with foreign-key triggers off for that one statement. Only for rows
/// whose other references (quote, transaction, reservation) this file has no reason to build; the
/// `rule_id` they carry is a real rule.
async fn insert_with_only_the_rule_real(pool: &PgPool, sql: &str, binds: (Uuid, Uuid, Uuid, Uuid)) {
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *tx)
        .await
        .expect("triggers off for this insert");
    sqlx::query(AssertSqlSafe(sql.to_owned()))
        .bind(binds.0)
        .bind(ORG)
        .bind(binds.1)
        .bind(binds.2)
        .bind(binds.3)
        .execute(&mut *tx)
        .await
        .expect("insert the referencing row");
    tx.commit().await.expect("commit");
}

/// A RESERVED reservation on `rule` of `version`.
async fn reservation_on(pool: &PgPool, rule: Uuid, version: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    insert_with_only_the_rule_real(
        pool,
        "INSERT INTO benefit_reservations (id, organization_id, rule_id, campaign_version_id, quote_id, \
             amount_thb, status, expires_at, created_at) \
         VALUES ($1, $2::uuid, $3, $4, $5, 10, 'RESERVED', now() + interval '15 minutes', now())",
        (id, rule, version, Uuid::new_v4()),
    )
    .await;
    id
}

/// A REDEEMED redemption on `rule` of `version`; its reservation and transaction are not built.
async fn redemption_on(pool: &PgPool, rule: Uuid, version: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    insert_with_only_the_rule_real(
        pool,
        "INSERT INTO benefit_redemptions (id, organization_id, rule_id, campaign_version_id, reservation_id, \
             transaction_id, benefit_class, amount_thb, status, reversal_id, redeemed_at) \
         VALUES ($1, $2::uuid, $3, $4, $5, gen_random_uuid(), 'PRICE', 10, 'REDEEMED', NULL, now())",
        (id, rule, version, Uuid::new_v4()),
    )
    .await;
    id
}

fn replacement() -> Value {
    body_with(Some(vec![rule(
        "RATE_IMPROVEMENT",
        serde_json::from_str(NEW_CALC).expect("calc"),
    )]))
}

fn has_thai(text: &str) -> bool {
    text.chars().any(|c| ('\u{0E00}'..='\u{0E7F}').contains(&c))
}

/// Sends `body` and expects the in-use refusal with nothing written.
async fn refused_in_use(
    room: &Room,
    campaign_id: Uuid,
    version_id: Uuid,
    body: &Value,
    case: &str,
) {
    let before = written(&room.pool, version_id).await;
    assert!(
        !before.references.is_empty(),
        "{case}: the setup must reference a rule"
    );

    let ans = save(room, campaign_id, body).await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "{case}: a rules replacement while a rule is in use must be refused with 400: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "{case}: {}", ans.text);
    assert_eq!(ans.field(), IN_USE_FIELD, "{case}: {}", ans.text);
    assert!(
        has_thai(ans.message()),
        "{case}: the message is Thai for the user: {}",
        ans.text
    );
    assert_eq!(
        written(&room.pool, version_id).await,
        before,
        "{case}: a refused save writes nothing — terms, branches, rules and references as before"
    );
}

/// Sends a replacement and expects it stored as sent: one rule, `NEW_CALC`, `rule_no` 1.
async fn stored_as_sent(room: &Room, campaign_id: Uuid, version_id: Uuid, case: &str) {
    let ans = save(room, campaign_id, &replacement()).await;
    assert_eq!(ans.status, StatusCode::OK, "{case}: {}", ans.text);
    let rules = written(&room.pool, version_id).await.rules;
    assert_eq!(rules.len(), 1, "{case}: {rules:?}");
    assert_eq!(
        (&rules[0].1, &rules[0].2, rules[0].3),
        (
            &"RATE_IMPROVEMENT".to_owned(),
            &serde_json::from_str::<Value>(NEW_CALC).expect("calc"),
            1
        ),
        "{case}: stored as sent"
    );
}

// ── refused: a current rule of the draft is referenced ──

#[actix_web::test]
async fn an_issued_coupon_on_the_drafts_rule_refuses_the_replacement() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;
    let rule = rule_ids(&room.pool, version_id).await[0];
    coupon_on(&room.pool, rule, "ISSUED").await;

    refused_in_use(
        &room,
        campaign_id,
        version_id,
        &replacement(),
        "issued coupon",
    )
    .await;

    room.finish().await;
}

#[actix_web::test]
async fn a_void_coupon_still_refuses_the_replacement() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;
    let rule = rule_ids(&room.pool, version_id).await[0];
    coupon_on(&room.pool, rule, "VOID").await;

    refused_in_use(
        &room,
        campaign_id,
        version_id,
        &replacement(),
        "void coupon",
    )
    .await;

    room.finish().await;
}

#[actix_web::test]
async fn a_reservation_on_the_drafts_rule_refuses_the_replacement() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;
    let rule = rule_ids(&room.pool, version_id).await[0];
    reservation_on(&room.pool, rule, version_id).await;

    refused_in_use(
        &room,
        campaign_id,
        version_id,
        &replacement(),
        "reservation",
    )
    .await;

    room.finish().await;
}

#[actix_web::test]
async fn a_redemption_on_the_drafts_rule_refuses_the_replacement() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;
    let rule = rule_ids(&room.pool, version_id).await[0];
    redemption_on(&room.pool, rule, version_id).await;

    refused_in_use(&room, campaign_id, version_id, &replacement(), "redemption").await;

    room.finish().await;
}

#[actix_web::test]
async fn a_simulated_draft_with_a_coupon_refuses_the_replacement() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;
    sqlx::query("UPDATE campaign_versions SET status = 'SIMULATED' WHERE id = $1")
        .bind(version_id)
        .execute(&room.pool)
        .await
        .expect("simulated");
    let rule = rule_ids(&room.pool, version_id).await[0];
    coupon_on(&room.pool, rule, "ISSUED").await;

    refused_in_use(
        &room,
        campaign_id,
        version_id,
        &replacement(),
        "simulated draft",
    )
    .await;

    room.finish().await;
}

#[actix_web::test]
async fn an_empty_rules_list_is_a_replacement_and_is_refused_too() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;
    let rule = rule_ids(&room.pool, version_id).await[0];
    coupon_on(&room.pool, rule, "ISSUED").await;

    refused_in_use(
        &room,
        campaign_id,
        version_id,
        &body_with(Some(vec![])),
        "rules: []",
    )
    .await;

    room.finish().await;
}

// ── accepted: nothing references a current rule of this draft ──

#[actix_web::test]
async fn a_coupon_on_another_campaigns_rule_does_not_refuse() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;
    let (_, other_version) = seed_draft(&room.pool).await;
    let other_rule = rule_ids(&room.pool, other_version).await[0];
    coupon_on(&room.pool, other_rule, "ISSUED").await;

    stored_as_sent(&room, campaign_id, version_id, "coupon on another campaign").await;
    assert_eq!(
        rule_ids(&room.pool, other_version).await,
        vec![other_rule],
        "the other campaign's rule is untouched"
    );

    room.finish().await;
}

#[actix_web::test]
async fn a_coupon_on_an_older_approved_version_of_the_same_campaign_does_not_refuse() {
    let room = Room::start().await;
    let (campaign_id, draft_id) = seed_draft(&room.pool).await;
    // the seeded version becomes version 2; version 1 is an approved one with its own rule and coupon
    let approved = Uuid::new_v4();
    sqlx::query("UPDATE campaign_versions SET version = 2 WHERE id = $1")
        .bind(draft_id)
        .execute(&room.pool)
        .await
        .expect("renumber the draft");
    sqlx::query(
        "INSERT INTO campaign_versions ( \
             id, organization_id, campaign_id, version, status, starts_at, ends_at, budget_thb, \
             stack_group, priority, version_hash, created_by, created_at, edited_at \
         ) VALUES ($1, $2::uuid, $3, 1, 'APPROVED', now() - interval '10 days', now() + interval '10 days', \
                   $4::numeric, 'RATE', 1, 'hash-v1', $5::uuid, now(), now())",
    )
    .bind(approved)
    .bind(ORG)
    .bind(campaign_id)
    .bind(SEEDED_BUDGET)
    .bind(USER)
    .execute(&room.pool)
    .await
    .expect("an approved version 1");
    let approved_rule = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, \
             calculation, caps, requires_coupon, rule_no) \
         VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', '{}'::jsonb, $3::jsonb, '{}'::jsonb, true, 1)",
    )
    .bind(approved_rule)
    .bind(approved)
    .bind(SEEDED_CALC)
    .execute(&room.pool)
    .await
    .expect("the approved version's rule");
    coupon_on(&room.pool, approved_rule, "ISSUED").await;

    stored_as_sent(
        &room,
        campaign_id,
        draft_id,
        "coupon on an older approved version",
    )
    .await;
    assert_eq!(
        rule_ids(&room.pool, approved).await,
        vec![approved_rule],
        "the approved version's rule is untouched"
    );

    room.finish().await;
}

#[actix_web::test]
async fn with_no_reference_the_replacement_is_stored_as_sent() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    stored_as_sent(&room, campaign_id, version_id, "no reference").await;

    room.finish().await;
}

#[actix_web::test]
async fn a_save_without_rules_still_saves_the_terms_while_a_coupon_exists() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;
    let rule = rule_ids(&room.pool, version_id).await[0];
    let coupon = coupon_on(&room.pool, rule, "ISSUED").await;
    let before = written(&room.pool, version_id).await;

    let ans = save(&room, campaign_id, &body_with(None)).await;

    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    let after = written(&room.pool, version_id).await;
    assert_eq!(after.rules, before.rules, "the rules are untouched");
    assert_eq!(
        after.references, before.references,
        "the coupon is untouched"
    );
    assert!(
        after
            .references
            .contains(&("coupons".to_owned(), coupon, rule))
    );
    assert_eq!(
        after.terms.3, "20000.00",
        "the terms are saved: {:?}",
        after.terms
    );

    room.finish().await;
}
