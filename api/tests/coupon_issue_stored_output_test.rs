// Written before the implementation by the test writer gemini/studio-13 (ticket author claude/ad05bf8d, controller claude/e92db6). Do not edit, delete or weaken.

//! Up-front tests for #1622 (replayed couponIssue returns coupon codes; fix moves
//! "codes not stored for replay" into CommandAction::stored_output).
//!
//! Behaviour table (§9 of #1622 and docs/design/blueprint/marketing-campaigns.md §6.2):
//! 1. First couponIssue response carries every code.
//! 2. The same request replayed returns the same coupon ids and the coupon objects
//!    have NO `code` key at all (assert key absent, not null).
//! 3. Rows 1-2 in two different schemas of one database one after the other:
//!    row 2 holds in both (pins the cross-schema flake).
//! 4. Another command's replay is byte-for-byte unchanged.
//! 5. After migrations, pg_trigger has no trg_strip_coupon_codes and pg_proc has no
//!    strip_coupon_codes_on_finish, and after one couponIssue call they are still absent.

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
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const MAIN: &str = "MAIN";
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
        let schema = unique("coupon_stored_output");
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
         VALUES ($1, $2, 'Coupon Stored Output Campaign', NULL, $3, now())",
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
                   50000.00, 'RATE', 1, $5, $6, now(), now())",
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

// ------------------------------------------------------------
// Row 1: First couponIssue response carries every code
// ------------------------------------------------------------

#[actix_web::test]
async fn row1_first_coupon_issue_response_carries_every_code() {
    let room = Room::start().await;
    let (campaign_id, v1_id) = seed_campaign(&room.pool, "APPROVED").await;
    let rule_id = seed_coupon_rule(&room.pool, v1_id).await;

    let uri = format!("/api/campaigns/{campaign_id}/coupons");
    let payload = json!({
        "ruleId": rule_id.to_string(),
        "count": 3,
        "expiresAt": "2026-12-31T23:59:59Z"
    });

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload, &unique("row1_issue"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "first couponIssue must succeed: {}",
        ans.text
    );

    let coupons = ans.body["coupons"]
        .as_array()
        .expect("response body must contain 'coupons' array");
    assert_eq!(coupons.len(), 3, "must issue exactly 3 coupons");

    for (i, c) in coupons.iter().enumerate() {
        let id_str = c["id"]
            .as_str()
            .unwrap_or_else(|| panic!("coupon[{i}].id must be a string"));
        assert!(
            Uuid::parse_str(id_str).is_ok(),
            "coupon[{i}].id must be a valid UUID: {id_str}"
        );

        let code = c["code"]
            .as_str()
            .unwrap_or_else(|| panic!("coupon[{i}] must carry code on first response: {c:?}"));
        assert!(
            !code.trim().is_empty(),
            "coupon[{i}].code must not be empty"
        );
    }

    room.finish().await;
}

// ------------------------------------------------------------
// Row 2: Replayed couponIssue returns same coupon ids, NO `code` key at all
// ------------------------------------------------------------

#[actix_web::test]
async fn row2_replayed_coupon_issue_returns_same_ids_with_no_code_key() {
    let room = Room::start().await;
    let (campaign_id, v1_id) = seed_campaign(&room.pool, "APPROVED").await;
    let rule_id = seed_coupon_rule(&room.pool, v1_id).await;

    let uri = format!("/api/campaigns/{campaign_id}/coupons");
    let idem_key = unique("row2_replay");
    let payload = json!({
        "ruleId": rule_id.to_string(),
        "count": 3,
        "expiresAt": "2026-12-31T23:59:59Z"
    });

    // Call 1: First issuance
    let ans1 = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload.clone(), &idem_key)),
    )
    .await;
    assert_eq!(ans1.status, StatusCode::OK, "first call must succeed");

    let coupons_1 = ans1.body["coupons"]
        .as_array()
        .expect("first response has coupons array");
    assert_eq!(coupons_1.len(), 3);
    for c in coupons_1 {
        assert!(
            c.get("code").is_some() && !c["code"].as_str().unwrap_or("").is_empty(),
            "first call must return codes: {c:?}"
        );
    }

    // Call 2: Replay with the same Idempotency-Key
    let ans2 = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload, &idem_key)),
    )
    .await;
    assert_eq!(ans2.status, StatusCode::OK, "replayed call must return 200");

    let coupons_2 = ans2.body["coupons"]
        .as_array()
        .expect("replayed response has coupons array");
    assert_eq!(
        coupons_2.len(),
        3,
        "replayed response must return the same number of coupons"
    );

    // Same coupon IDs in the same order
    for (i, (c1, c2)) in coupons_1.iter().zip(coupons_2.iter()).enumerate() {
        assert_eq!(
            c1["id"], c2["id"],
            "coupon[{i}] id must match across replay"
        );
    }

    // Row 2 requirement: coupon objects have NO `code` key at all (assert key absent, not null)
    for (i, c2) in coupons_2.iter().enumerate() {
        assert!(
            c2.get("code").is_none(),
            "coupon[{i}] in replay must have NO 'code' key at all (must be absent, not null); found: {:?}",
            c2.get("code")
        );
    }

    room.finish().await;
}

// ------------------------------------------------------------
// Row 3: Rows 1-2 in two different schemas of one database one after the other
// ------------------------------------------------------------

#[actix_web::test]
async fn row3_coupon_issue_replay_holds_across_two_schemas_in_one_database() {
    // Schema A: First schema
    let room_a = Room::start().await;
    let (c_a, v_a) = seed_campaign(&room_a.pool, "APPROVED").await;
    let r_a = seed_coupon_rule(&room_a.pool, v_a).await;

    let uri_a = format!("/api/campaigns/{c_a}/coupons");
    let key_a = unique("row3_schema_a");
    let payload_a = json!({
        "ruleId": r_a.to_string(),
        "count": 2,
        "expiresAt": "2026-12-31T23:59:59Z"
    });

    // Schema A, Call 1 (Row 1): carries codes
    let ans_a1 = call(
        &room_a,
        Some("MARKETING"),
        Method::POST,
        &uri_a,
        Some((payload_a.clone(), &key_a)),
    )
    .await;
    assert_eq!(ans_a1.status, StatusCode::OK, "schema A call 1 succeeds");
    let coupons_a1 = ans_a1.body["coupons"].as_array().expect("schema A coupons");
    assert_eq!(coupons_a1.len(), 2);
    for c in coupons_a1 {
        assert!(
            c.get("code").is_some() && !c["code"].as_str().unwrap_or("").is_empty(),
            "schema A first call must carry codes"
        );
    }

    // Schema A, Call 2 (Row 2): same IDs, NO `code` key
    let ans_a2 = call(
        &room_a,
        Some("MARKETING"),
        Method::POST,
        &uri_a,
        Some((payload_a, &key_a)),
    )
    .await;
    assert_eq!(ans_a2.status, StatusCode::OK, "schema A call 2 succeeds");
    let coupons_a2 = ans_a2.body["coupons"]
        .as_array()
        .expect("schema A replayed coupons");
    assert_eq!(coupons_a2.len(), 2);
    for (c1, c2) in coupons_a1.iter().zip(coupons_a2.iter()) {
        assert_eq!(c1["id"], c2["id"]);
        assert!(
            c2.get("code").is_none(),
            "schema A replay must have NO 'code' key at all; found: {:?}",
            c2.get("code")
        );
    }

    // Schema B: Second schema in the SAME database (room_a is not finished yet!)
    let room_b = Room::start().await;
    let (c_b, v_b) = seed_campaign(&room_b.pool, "APPROVED").await;
    let r_b = seed_coupon_rule(&room_b.pool, v_b).await;

    let uri_b = format!("/api/campaigns/{c_b}/coupons");
    let key_b = unique("row3_schema_b");
    let payload_b = json!({
        "ruleId": r_b.to_string(),
        "count": 2,
        "expiresAt": "2026-12-31T23:59:59Z"
    });

    // Schema B, Call 1 (Row 1): carries codes
    let ans_b1 = call(
        &room_b,
        Some("MARKETING"),
        Method::POST,
        &uri_b,
        Some((payload_b.clone(), &key_b)),
    )
    .await;
    assert_eq!(ans_b1.status, StatusCode::OK, "schema B call 1 succeeds");
    let coupons_b1 = ans_b1.body["coupons"].as_array().expect("schema B coupons");
    assert_eq!(coupons_b1.len(), 2);
    for c in coupons_b1 {
        assert!(
            c.get("code").is_some() && !c["code"].as_str().unwrap_or("").is_empty(),
            "schema B first call must carry codes"
        );
    }

    // Schema B, Call 2 (Row 2): same IDs, NO `code` key (pins cross-schema flake)
    let ans_b2 = call(
        &room_b,
        Some("MARKETING"),
        Method::POST,
        &uri_b,
        Some((payload_b, &key_b)),
    )
    .await;
    assert_eq!(ans_b2.status, StatusCode::OK, "schema B call 2 succeeds");
    let coupons_b2 = ans_b2.body["coupons"]
        .as_array()
        .expect("schema B replayed coupons");
    assert_eq!(coupons_b2.len(), 2);
    for (c1, c2) in coupons_b1.iter().zip(coupons_b2.iter()) {
        assert_eq!(c1["id"], c2["id"]);
        assert!(
            c2.get("code").is_none(),
            "schema B replay must have NO 'code' key at all (cross-schema trigger flake); found: {:?}",
            c2.get("code")
        );
    }

    room_b.finish().await;
    room_a.finish().await;
}

// ------------------------------------------------------------
// Row 4: Another command's replay is byte-for-byte unchanged
// ------------------------------------------------------------

#[actix_web::test]
async fn row4_another_command_replay_is_byte_for_byte_unchanged() {
    let room = Room::start().await;

    // Pick an existing idempotent v3 command with a known stored response: campaignCreate
    let payload = json!({
        "name": "Byte For Byte Replay Campaign",
        "startsAt": "2026-11-01T00:00:00Z",
        "endsAt": "2026-12-01T00:00:00Z",
        "budgetThb": "25000.00",
        "stackGroup": "DEFAULT",
        "priority": 1,
        "branches": ["MAIN"]
    });
    let idem_key = unique("row4_another_cmd");

    // Call 1
    let ans1 = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        "/api/campaigns",
        Some((payload.clone(), &idem_key)),
    )
    .await;
    assert_eq!(
        ans1.status,
        StatusCode::OK,
        "campaignCreate first call must succeed: {}",
        ans1.text
    );

    // Call 2: Replay with same key
    let ans2 = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        "/api/campaigns",
        Some((payload, &idem_key)),
    )
    .await;
    assert_eq!(
        ans2.status,
        StatusCode::OK,
        "campaignCreate replay must succeed: {}",
        ans2.text
    );

    assert_eq!(
        ans1.text, ans2.text,
        "another command's replay must be byte-for-byte unchanged"
    );
    assert_eq!(
        ans1.body, ans2.body,
        "another command's replay body must be identical"
    );

    room.finish().await;
}

// ------------------------------------------------------------
// Row 5: After migrations, pg_trigger has no trg_strip_coupon_codes and pg_proc
//        has no strip_coupon_codes_on_finish, and after one couponIssue call they
//        are still absent.
// ------------------------------------------------------------

#[actix_web::test]
async fn row5_strip_trigger_and_proc_absent_after_migration_and_after_coupon_issue() {
    let room = Room::start().await;

    // Immediately after migrations: neither trigger nor function must exist in the schema's catalogs
    let trg_count_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid \
         WHERE t.tgname = 'trg_strip_coupon_codes' AND c.relnamespace = current_schema()::regnamespace",
    )
    .fetch_one(&room.pool)
    .await
    .expect("query pg_trigger before couponIssue");
    assert_eq!(
        trg_count_before, 0,
        "after migrations, pg_trigger must have no trg_strip_coupon_codes in current schema"
    );

    let proc_count_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_proc \
         WHERE proname = 'strip_coupon_codes_on_finish' AND pronamespace = current_schema()::regnamespace",
    )
    .fetch_one(&room.pool)
    .await
    .expect("query pg_proc before couponIssue");
    assert_eq!(
        proc_count_before, 0,
        "after migrations, pg_proc must have no strip_coupon_codes_on_finish in current schema"
    );

    // Execute one couponIssue call
    let (campaign_id, v1_id) = seed_campaign(&room.pool, "APPROVED").await;
    let rule_id = seed_coupon_rule(&room.pool, v1_id).await;

    let uri = format!("/api/campaigns/{campaign_id}/coupons");
    let payload = json!({
        "ruleId": rule_id.to_string(),
        "count": 1,
        "expiresAt": "2026-12-31T23:59:59Z"
    });

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload, &unique("row5_coupon"))),
    )
    .await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "couponIssue must succeed: {}",
        ans.text
    );

    // After one couponIssue call: trigger and function must STILL be absent in current schema
    let trg_count_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid \
         WHERE t.tgname = 'trg_strip_coupon_codes' AND c.relnamespace = current_schema()::regnamespace",
    )
    .fetch_one(&room.pool)
    .await
    .expect("query pg_trigger after couponIssue");
    assert_eq!(
        trg_count_after, 0,
        "after couponIssue call, pg_trigger must still have no trg_strip_coupon_codes in current schema"
    );

    let proc_count_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_proc \
         WHERE proname = 'strip_coupon_codes_on_finish' AND pronamespace = current_schema()::regnamespace",
    )
    .fetch_one(&room.pool)
    .await
    .expect("query pg_proc after couponIssue");
    assert_eq!(
        proc_count_after, 0,
        "after couponIssue call, pg_proc must still have no strip_coupon_codes_on_finish in current schema"
    );

    room.finish().await;
}
