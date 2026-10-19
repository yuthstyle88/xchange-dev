//! #2031: up-front tests for what `campaignCreate` (`POST /api/campaigns`) and
//! `campaignVersionSave` (`PUT /api/campaigns/{id}/draft`) accept as a version's branches, dates and
//! budget.
//!
//! Written before the fix by the ticket writer (claude/studio-claude-3, on the BE lane controller
//! claude/1656900d's order of 2026-10-16). The ruling is #2031 §3, and the shared check is
//! `CampaignBranchScope::first_outside_org_in` (design `marketing-campaigns.md` §6). The implementer
//! must not edit, delete or weaken these tests.
//!
//! The caller is always an org-A MARKETING user. "Nothing written" means: no `campaigns` row with
//! the request's name (create), or the seeded version's terms and `campaign_branches` unchanged
//! (draft save).
//!
//! Create rows:
//! - C1. `branches` names org B's branch: 400 `validationFailed`, field `branches`; nothing written.
//! - C2. `branches` names an id that is no branch: 400 `validationFailed`, field `branches`;
//!   nothing written.
//! - C3. an own branch first, then org B's: 400 `validationFailed`, field `branches`; nothing
//!   written (no partial insert).
//! - C4. `endsAt` equals `startsAt`: 400 `validationFailed`, field `endsAt`; nothing written.
//! - C5. `endsAt` before `startsAt`: 400 `validationFailed`, field `endsAt`; nothing written.
//! - C6. `budgetThb` = "-0.01": 400 `validationFailed`, field `budgetThb`; nothing written.
//! - C7 (keep). Two own branches and `budgetThb` = "0.00": 200, and exactly those two branches
//!   are stored.
//!
//! Draft-save rows (seeded DRAFT version: targets `MAIN`, budget 10000.00):
//! - S1. `branches` names org B's branch: 400 `validationFailed`, field `branches`; unchanged.
//! - S2. `branches` names an id that is no branch: 400 `validationFailed`, field `branches`;
//!   unchanged.
//! - S3. `endsAt` before `startsAt`: 400 `validationFailed`, field `endsAt`; unchanged.
//! - S4. `budgetThb` = "-5.00": 400 `validationFailed`, field `budgetThb`; unchanged.
//! - S5 (keep). Two own branches, budget 20000.00: 200; the branches are replaced by exactly those
//!   two and the budget is 20000.00.
//!
//! `campaignSimulate`'s branch rows are pinned by #1992's `campaign_simulate_branch_scope_test.rs`.
//! They must stay green when its inline query moves into the shared function.

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
/// Organisation A: the caller's organisation, seeded by the migrations (owns `MAIN`).
const ORG_A: &str = "00000000-0000-0000-0000-000000000001";
/// Organisation B: a second organisation the caller has nothing to do with.
const ORG_B: &str = "00000000-0000-0000-0000-0000000000b2";
const MAIN: &str = "MAIN";
/// A second org-A branch.
const A_SECOND: &str = "XC-A02";
/// A branch of org B.
const B_BRANCH: &str = "ORGB-B01";
/// An id that names no branch at all.
const NO_BRANCH: &str = "NO-SUCH-BRANCH";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const STARTS: &str = "2026-11-01T00:00:00Z";
const ENDS: &str = "2026-12-01T00:00:00Z";

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
        let schema = unique("campaign_branches_scope");
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

        // A second org-A branch (organization_id defaults to org A).
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, 'A02')")
            .bind(A_SECOND)
            .execute(&pool)
            .await
            .expect("seed org-A branch");

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

/// An org-A campaign with one DRAFT version: Nov 2026, budget 10000.00, targets `MAIN` only.
async fn seed_draft(pool: &PgPool) -> (Uuid, Uuid) {
    let org_id = Uuid::parse_str(ORG_A).unwrap();
    let user_id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, current_version_id, created_by, created_at) \
         VALUES ($1, $2, $3, NULL, $4, now())",
    )
    .bind(campaign_id)
    .bind(org_id)
    .bind(unique("seeded"))
    .bind(user_id)
    .execute(pool)
    .await
    .expect("seed campaign");

    sqlx::query(
        "INSERT INTO campaign_versions ( \
             id, organization_id, campaign_id, version, status, starts_at, ends_at, budget_thb, \
             stack_group, priority, version_hash, created_by, created_at, edited_at \
         ) VALUES ($1, $2, $3, 1, 'DRAFT', $4::timestamptz, $5::timestamptz, 10000.00, 'RATE', 1, \
                   NULL, $6, now(), now())",
    )
    .bind(version_id)
    .bind(org_id)
    .bind(campaign_id)
    .bind(STARTS)
    .bind(ENDS)
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

    sqlx::query("INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2)")
        .bind(version_id)
        .bind(MAIN)
        .execute(pool)
        .await
        .expect("seed campaign_branches");

    (campaign_id, version_id)
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

async fn send(room: &Room, method: Method, uri: &str, payload: Value) -> Answer {
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
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((AS, "MARKETING"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header(("idempotency-key", unique("branches_scope")))
        .set_payload(payload.to_string());

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn create_body(name: &str, starts: &str, ends: &str, budget: &str, branches: &[&str]) -> Value {
    json!({
        "name": name,
        "startsAt": starts,
        "endsAt": ends,
        "budgetThb": budget,
        "stackGroup": "RATE",
        "priority": 1,
        "branches": branches,
    })
}

fn save_body(starts: &str, ends: &str, budget: &str, branches: &[&str]) -> Value {
    json!({
        "startsAt": starts,
        "endsAt": ends,
        "budgetThb": budget,
        "stackGroup": "RATE",
        "priority": 2,
        "branches": branches,
    })
}

async fn create(room: &Room, body: Value) -> Answer {
    send(room, Method::POST, "/api/campaigns", body).await
}

async fn save(room: &Room, campaign_id: Uuid, body: Value) -> Answer {
    let uri = format!("/api/campaigns/{campaign_id}/draft");
    send(room, Method::PUT, &uri, body).await
}

fn assert_refused(ans: &Answer, field: &str, row: &str) {
    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "{row}: must be refused with 400; got {} body={}",
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
}

async fn campaigns_named(pool: &PgPool, name: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM campaigns WHERE name = $1")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("count campaigns")
}

async fn foreign_targets(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM campaign_branches WHERE branch_id = $1")
        .bind(B_BRANCH)
        .fetch_one(pool)
        .await
        .expect("count org-B campaign_branches")
}

/// A refused create wrote nothing: no campaign by that name and no row naming org B's branch.
async fn assert_create_wrote_nothing(room: &Room, name: &str, row: &str) {
    assert_eq!(
        campaigns_named(&room.pool, name).await,
        0,
        "{row}: no campaign may be stored"
    );
    assert_eq!(
        foreign_targets(&room.pool).await,
        0,
        "{row}: no campaign_branches row may name org B's branch"
    );
}

async fn branches_of(pool: &PgPool, version_id: Uuid) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT branch_id FROM campaign_branches WHERE campaign_version_id = $1 ORDER BY branch_id",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
    .expect("read campaign_branches")
}

/// The seeded version's stored terms: (budget, priority, branches).
async fn terms_of(pool: &PgPool, version_id: Uuid) -> (Decimal, i32, Vec<String>) {
    let (budget, priority): (Decimal, i32) =
        sqlx::query_as("SELECT budget_thb, priority FROM campaign_versions WHERE id = $1")
            .bind(version_id)
            .fetch_one(pool)
            .await
            .expect("read version terms");
    (budget, priority, branches_of(pool, version_id).await)
}

/// A refused draft save left the seeded version as it was.
async fn assert_draft_unchanged(room: &Room, version_id: Uuid, row: &str) {
    let (budget, priority, branches) = terms_of(&room.pool, version_id).await;
    assert_eq!(
        budget,
        Decimal::from_str("10000.00").unwrap(),
        "{row}: the budget must be unchanged"
    );
    assert_eq!(priority, 1, "{row}: the priority must be unchanged");
    assert_eq!(
        branches,
        vec![MAIN.to_string()],
        "{row}: the version's branches must be unchanged"
    );
}

// ------------------------------------------------------------ create rows

#[actix_web::test]
async fn c1_create_naming_another_orgs_branch_is_refused_on_branches() {
    let room = Room::start().await;
    let name = unique("c1");

    let ans = create(
        &room,
        create_body(&name, STARTS, ENDS, "1000.00", &[B_BRANCH]),
    )
    .await;

    assert_refused(&ans, "branches", "C1 (org-B branch)");
    assert_create_wrote_nothing(&room, &name, "C1").await;
    room.finish().await;
}

#[actix_web::test]
async fn c2_create_naming_no_branch_is_refused_on_branches() {
    let room = Room::start().await;
    let name = unique("c2");

    let ans = create(
        &room,
        create_body(&name, STARTS, ENDS, "1000.00", &[NO_BRANCH]),
    )
    .await;

    assert_refused(&ans, "branches", "C2 (unknown branch)");
    assert_create_wrote_nothing(&room, &name, "C2").await;
    room.finish().await;
}

#[actix_web::test]
async fn c3_create_with_an_own_then_a_foreign_branch_writes_nothing() {
    let room = Room::start().await;
    let name = unique("c3");

    let ans = create(
        &room,
        create_body(&name, STARTS, ENDS, "1000.00", &[MAIN, B_BRANCH]),
    )
    .await;

    assert_refused(&ans, "branches", "C3 (own then org-B)");
    assert_create_wrote_nothing(&room, &name, "C3").await;
    room.finish().await;
}

#[actix_web::test]
async fn c4_create_ending_when_it_starts_is_refused_on_ends_at() {
    let room = Room::start().await;
    let name = unique("c4");

    let ans = create(
        &room,
        create_body(&name, STARTS, STARTS, "1000.00", &[MAIN]),
    )
    .await;

    assert_refused(&ans, "endsAt", "C4 (endsAt = startsAt)");
    assert_create_wrote_nothing(&room, &name, "C4").await;
    room.finish().await;
}

#[actix_web::test]
async fn c5_create_ending_before_it_starts_is_refused_on_ends_at() {
    let room = Room::start().await;
    let name = unique("c5");

    let ans = create(&room, create_body(&name, ENDS, STARTS, "1000.00", &[MAIN])).await;

    assert_refused(&ans, "endsAt", "C5 (endsAt < startsAt)");
    assert_create_wrote_nothing(&room, &name, "C5").await;
    room.finish().await;
}

#[actix_web::test]
async fn c6_create_with_a_negative_budget_is_refused_on_budget_thb() {
    let room = Room::start().await;
    let name = unique("c6");

    let ans = create(&room, create_body(&name, STARTS, ENDS, "-0.01", &[MAIN])).await;

    assert_refused(&ans, "budgetThb", "C6 (budget -0.01)");
    assert_create_wrote_nothing(&room, &name, "C6").await;
    room.finish().await;
}

#[actix_web::test]
async fn c7_create_with_own_branches_and_a_zero_budget_is_stored() {
    let room = Room::start().await;
    let name = unique("c7");

    let ans = create(
        &room,
        create_body(&name, STARTS, ENDS, "0.00", &[MAIN, A_SECOND]),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "C7: own branches and a zero budget must be accepted: {}",
        ans.text
    );
    let id = Uuid::parse_str(ans.body["id"].as_str().expect("C7: an id in the answer"))
        .expect("C7: the id is a uuid");
    let version_id: Uuid =
        sqlx::query_scalar("SELECT id FROM campaign_versions WHERE campaign_id = $1")
            .bind(id)
            .fetch_one(&room.pool)
            .await
            .expect("C7: the version is stored");
    assert_eq!(
        branches_of(&room.pool, version_id).await,
        vec![MAIN.to_string(), A_SECOND.to_string()],
        "C7: exactly the two named branches are stored"
    );
    room.finish().await;
}

// ------------------------------------------------------------ draft-save rows

#[actix_web::test]
async fn s1_save_naming_another_orgs_branch_is_refused_on_branches() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        save_body(STARTS, ENDS, "20000.00", &[B_BRANCH]),
    )
    .await;

    assert_refused(&ans, "branches", "S1 (org-B branch)");
    assert_draft_unchanged(&room, version_id, "S1").await;
    room.finish().await;
}

#[actix_web::test]
async fn s2_save_naming_no_branch_is_refused_on_branches() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        save_body(STARTS, ENDS, "20000.00", &[MAIN, NO_BRANCH]),
    )
    .await;

    assert_refused(&ans, "branches", "S2 (unknown branch)");
    assert_draft_unchanged(&room, version_id, "S2").await;
    room.finish().await;
}

#[actix_web::test]
async fn s3_save_ending_before_it_starts_is_refused_on_ends_at() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        save_body(ENDS, STARTS, "20000.00", &[MAIN]),
    )
    .await;

    assert_refused(&ans, "endsAt", "S3 (endsAt < startsAt)");
    assert_draft_unchanged(&room, version_id, "S3").await;
    room.finish().await;
}

#[actix_web::test]
async fn s4_save_with_a_negative_budget_is_refused_on_budget_thb() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        save_body(STARTS, ENDS, "-5.00", &[MAIN]),
    )
    .await;

    assert_refused(&ans, "budgetThb", "S4 (budget -5.00)");
    assert_draft_unchanged(&room, version_id, "S4").await;
    room.finish().await;
}

#[actix_web::test]
async fn s5_save_with_own_branches_replaces_them() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        save_body(STARTS, ENDS, "20000.00", &[MAIN, A_SECOND]),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "S5: own branches must be accepted: {}",
        ans.text
    );
    let (budget, priority, branches) = terms_of(&room.pool, version_id).await;
    assert_eq!(
        budget,
        Decimal::from_str("20000.00").unwrap(),
        "S5: budget saved"
    );
    assert_eq!(priority, 2, "S5: priority saved");
    assert_eq!(
        branches,
        vec![MAIN.to_string(), A_SECOND.to_string()],
        "S5: the branches are replaced by exactly the two named"
    );
    room.finish().await;
}
