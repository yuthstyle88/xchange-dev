//! Ticket #994 (APP-4 route half): HTTP routes for planner drafts, preview, and app settings
//! (design `docs/design/blueprint/customer-app.md` §4.4, §8, §9.1; lane controller rulings on #994).
//!
//! Written before the implementation by test-writer gemini/studio-9 on claude/ad05bf8d's order.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! What these tests pin:
//! 1. `plannerDraftSave` create, update with `expectedVersion`, and stale version (`staleVersion` 409).
//! 2. `plannerDraftSave` idempotency: same key replays draft; different body gives `idempotencyConflict` (409).
//! 3. First app command issues `xc_app_sid` cookie (`HttpOnly; Secure; SameSite=Lax`), and subsequent command with that cookie keeps the session.
//! 4. `my_planner_drafts` keeps sessions apart (session A vs session B, customer vs session, and anonymous call).
//! 5. `plannerDraftDelete` by another owner gives `outOfScope` (403), draft is preserved.
//! 6. `planner_preview` with no session computes estimate and writes no rows to `quotes`, `bookings`, or `stock_movements`.
//! 7. `appSettingsSave` over HTTP: roles without `Permission::SystemSettings` (TELLER, CUSTOMER) get `roleNotAllowedHere` (400),
//!    while ADMIN and EXECUTIVE pass (200) and upsert the organization's settings.
//! 8. Rate limit: 20 draft writes allowed per session per minute; the 21st write gives `rateLimited` (429).
//!
//! Red on `main` by result: none of the five route actions is registered on `main`, so requests
//! return 404 Not Found (or fail to match expected status/code/headers).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test` (never port 6381).
//! Each test runs in an isolated PostgreSQL schema.

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

use xc_app::domain::app_session::{APP_SESSION_COOKIE, AppSessionId};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

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

// ------------------------------------------------------------ database harness

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
        let schema = unique("app4_routes");
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

        common::install_test_pii_keys();

        let room = Self {
            pool,
            schema,
            guard: Arc::new(AppSessionGuard::new(20)),
        };

        // Seed USD rate publication for planner_preview
        room.publish_usd().await;

        room
    }

    async fn insert_customer(&self, id: &str) {
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($1, $2::uuid, $1, 'Test Customer', 'LIVE', now()) ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(ORG)
        .execute(&self.pool)
        .await
        .expect("insert customer");
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn publish_usd(&self) {
        let approval: Uuid = sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
             proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES (gen_random_uuid(), 'RATE_PUBLISH', $1, $2, 'h', 'p', '{}', 'test', now(), 'APPROVED', $2) \
             RETURNING id",
        )
        .bind(MAIN)
        .bind(unique("pub"))
        .fetch_one(&self.pool)
        .await
        .expect("approved request for publication");

        let publication: Uuid = sqlx::query_scalar(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
             VALUES (gen_random_uuid(), $1, 1, now(), $2, 'test') RETURNING id",
        )
        .bind(MAIN)
        .bind(approval)
        .fetch_one(&self.pool)
        .await
        .expect("rate publication");

        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, unit_size, buy_rate, sell_rate, source_snapshot) \
             VALUES (gen_random_uuid(), $1, 'USD', 1, 33.50, 34.00, $2)",
        )
        .bind(publication)
        .bind(json!({ "buy_mode": "AUTO", "sell_mode": "AUTO" }))
        .execute(&self.pool)
        .await
        .expect("published USD line");

        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(publication)
            .bind(MAIN)
            .execute(&self.pool)
            .await
            .expect("make publication current");
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

// ------------------------------------------------------------ callers & session reader

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
        let staff_user = user.as_ref().and_then(|u| {
            if u.roles.iter().any(|r| r.role != Role::Customer) {
                Some(u.clone())
            } else {
                None
            }
        });
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

// ------------------------------------------------------------ HTTP test client

struct Answer {
    status: StatusCode,
    set_cookie: Vec<String>,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> String {
        self.body["error"]["code"].as_str().unwrap_or("").to_owned()
    }
}

async fn call(
    room: &Room,
    who: Option<&str>,
    cookie: Option<&str>,
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
    if let Some(c) = cookie {
        req = req.insert_header((header::COOKIE, format!("{APP_SESSION_COOKIE}={c}")));
    }
    if let Some((b, key)) = body {
        req = req
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header(("idempotency-key", key.to_owned()))
            .set_payload(b.to_string());
    }

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let set_cookie = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .filter_map(|v| v.to_str().ok().map(str::to_owned))
        .collect();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

    Answer {
        status,
        set_cookie,
        text,
        body,
    }
}

// ------------------------------------------------------------ up-front tests

#[actix_web::test]
async fn planner_draft_save_create_update_and_stale_version() {
    let room = Room::start().await;
    let session_id = AppSessionId::generate();

    // 1. Create draft
    let create_body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "500.0000" },
        "expectedVersion": null,
    });
    let res = call(
        &room,
        None,
        Some(session_id.as_str()),
        Method::POST,
        "/api/app/planner-drafts",
        Some((create_body, "create-key-1")),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "create draft succeeded: {}",
        res.text
    );
    let draft_id = res.body["id"].as_str().expect("draft id string").to_owned();
    assert_eq!(
        res.body["version"].as_i64(),
        Some(1),
        "created at version 1"
    );

    // 2. Update draft with expectedVersion 1
    let update_body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "600.0000" },
        "expectedVersion": 1,
    });
    let res = call(
        &room,
        None,
        Some(session_id.as_str()),
        Method::PUT,
        &format!("/api/app/planner-drafts/{draft_id}"),
        Some((update_body, "update-key-1")),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "update draft succeeded: {}",
        res.text
    );
    assert_eq!(
        res.body["version"].as_i64(),
        Some(2),
        "updated to version 2"
    );

    // 3. Stale update with expectedVersion 1 (when current is 2)
    let stale_body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "700.0000" },
        "expectedVersion": 1,
    });
    let res = call(
        &room,
        None,
        Some(session_id.as_str()),
        Method::PUT,
        &format!("/api/app/planner-drafts/{draft_id}"),
        Some((stale_body, "stale-key-1")),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::CONFLICT,
        "stale update returns 409 Conflict"
    );
    assert_eq!(res.code(), "staleVersion", "error code is staleVersion");

    room.finish().await;
}

#[actix_web::test]
async fn planner_draft_save_idempotency_same_key_replays_and_different_body_conflicts() {
    let room = Room::start().await;
    let session_id = AppSessionId::generate();

    let body1 = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "500.0000" },
        "expectedVersion": null,
    });

    // 1. Initial save
    let res1 = call(
        &room,
        None,
        Some(session_id.as_str()),
        Method::POST,
        "/api/app/planner-drafts",
        Some((body1.clone(), "idem-key-1")),
    )
    .await;
    assert_eq!(res1.status, StatusCode::OK, "first save succeeded");
    let id1 = res1.body["id"].as_str().expect("id string");

    // 2. Replay with identical body and key
    let res2 = call(
        &room,
        None,
        Some(session_id.as_str()),
        Method::POST,
        "/api/app/planner-drafts",
        Some((body1, "idem-key-1")),
    )
    .await;
    assert_eq!(res2.status, StatusCode::OK, "replay succeeded");
    assert_eq!(
        res2.body["id"].as_str(),
        Some(id1),
        "replayed same draft id"
    );

    // 3. Same key with different body -> idempotencyConflict
    let body2 = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_SELL",
        "amount": { "quantity": "999.0000" },
        "expectedVersion": null,
    });
    let res3 = call(
        &room,
        None,
        Some(session_id.as_str()),
        Method::POST,
        "/api/app/planner-drafts",
        Some((body2, "idem-key-1")),
    )
    .await;
    assert_eq!(res3.status, StatusCode::CONFLICT, "conflict returns 409");
    assert_eq!(res3.code(), "idempotencyConflict");

    room.finish().await;
}

#[actix_web::test]
async fn first_app_command_issues_xc_app_sid_cookie() {
    let room = Room::start().await;

    let body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "100.0000" },
        "expectedVersion": null,
    });

    // First command without cookie or session
    let res1 = call(
        &room,
        None,
        None,
        Method::POST,
        "/api/app/planner-drafts",
        Some((body.clone(), "anon-key-1")),
    )
    .await;
    assert_eq!(
        res1.status,
        StatusCode::OK,
        "anonymous save succeeded: {}",
        res1.text
    );

    // Verify Set-Cookie header contains xc_app_sid with proper flags
    let cookie_header = res1
        .set_cookie
        .iter()
        .find(|c| c.contains("xc_app_sid="))
        .unwrap_or_else(|| {
            panic!(
                "Set-Cookie header missing xc_app_sid: {:?}",
                res1.set_cookie
            )
        });
    assert!(
        cookie_header.contains("HttpOnly")
            && cookie_header.contains("Secure")
            && cookie_header.contains("SameSite=Lax"),
        "cookie missing required flags: {cookie_header}"
    );

    let sid = cookie_header
        .split(';')
        .next()
        .and_then(|p| p.strip_prefix("xc_app_sid="))
        .expect("cookie value");

    // Second command with the issued cookie
    let res2 = call(
        &room,
        None,
        Some(sid),
        Method::POST,
        "/api/app/planner-drafts",
        Some((body, "anon-key-2")),
    )
    .await;
    assert_eq!(
        res2.status,
        StatusCode::OK,
        "second save with cookie succeeded"
    );

    // View drafts for this session: both drafts should belong to this session
    let view_res = call(
        &room,
        None,
        Some(sid),
        Method::GET,
        "/api/v3/views/my_planner_drafts",
        None,
    )
    .await;
    assert_eq!(
        view_res.status,
        StatusCode::OK,
        "my_planner_drafts succeeded"
    );
    let drafts = view_res.body["data"]["drafts"]
        .as_array()
        .or_else(|| view_res.body["data"].as_array())
        .expect("drafts array in view");
    assert_eq!(drafts.len(), 2, "session has 2 drafts");

    room.finish().await;
}

#[actix_web::test]
async fn my_planner_drafts_keeps_sessions_apart_and_isolated_from_customers() {
    let room = Room::start().await;
    let session_a = AppSessionId::generate();
    let session_b = AppSessionId::generate();
    let customer_id = "CX-APP4-1";
    room.insert_customer(customer_id).await;

    let body_a = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "100.0000" },
        "expectedVersion": null,
    });
    let body_b = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "200.0000" },
        "expectedVersion": null,
    });
    let body_c = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "300.0000" },
        "expectedVersion": null,
    });

    let res_a = call(
        &room,
        None,
        Some(session_a.as_str()),
        Method::POST,
        "/api/app/planner-drafts",
        Some((body_a, "key-a")),
    )
    .await;
    assert_eq!(res_a.status, StatusCode::OK);
    let id_a = res_a.body["id"].as_str().expect("id_a");

    let res_b = call(
        &room,
        None,
        Some(session_b.as_str()),
        Method::POST,
        "/api/app/planner-drafts",
        Some((body_b, "key-b")),
    )
    .await;
    assert_eq!(res_b.status, StatusCode::OK);
    let id_b = res_b.body["id"].as_str().expect("id_b");

    let res_c = call(
        &room,
        Some(&format!("CUSTOMER:{customer_id}")),
        None,
        Method::POST,
        "/api/app/planner-drafts",
        Some((body_c, "key-c")),
    )
    .await;
    assert_eq!(res_c.status, StatusCode::OK);
    let id_c = res_c.body["id"].as_str().expect("id_c");

    // Session A view
    let v_a = call(
        &room,
        None,
        Some(session_a.as_str()),
        Method::GET,
        "/api/v3/views/my_planner_drafts",
        None,
    )
    .await;
    assert_eq!(v_a.status, StatusCode::OK);
    let drafts_a = v_a.body["data"]["drafts"]
        .as_array()
        .or_else(|| v_a.body["data"].as_array())
        .expect("drafts array");
    assert_eq!(drafts_a.len(), 1);
    assert_eq!(drafts_a[0]["id"].as_str(), Some(id_a));

    // Session B view
    let v_b = call(
        &room,
        None,
        Some(session_b.as_str()),
        Method::GET,
        "/api/v3/views/my_planner_drafts",
        None,
    )
    .await;
    assert_eq!(v_b.status, StatusCode::OK);
    let drafts_b = v_b.body["data"]["drafts"]
        .as_array()
        .or_else(|| v_b.body["data"].as_array())
        .expect("drafts array");
    assert_eq!(drafts_b.len(), 1);
    assert_eq!(drafts_b[0]["id"].as_str(), Some(id_b));

    // Customer view
    let v_c = call(
        &room,
        Some(&format!("CUSTOMER:{customer_id}")),
        None,
        Method::GET,
        "/api/v3/views/my_planner_drafts",
        None,
    )
    .await;
    assert_eq!(v_c.status, StatusCode::OK);
    let drafts_c = v_c.body["data"]["drafts"]
        .as_array()
        .or_else(|| v_c.body["data"].as_array())
        .expect("drafts array");
    assert_eq!(drafts_c.len(), 1);
    assert_eq!(drafts_c[0]["id"].as_str(), Some(id_c));

    // Anonymous view (no session, no customer)
    let v_anon = call(
        &room,
        None,
        None,
        Method::GET,
        "/api/v3/views/my_planner_drafts",
        None,
    )
    .await;
    assert_eq!(v_anon.status, StatusCode::OK);
    let drafts_anon = v_anon.body["data"]["drafts"]
        .as_array()
        .or_else(|| v_anon.body["data"].as_array())
        .expect("drafts array");
    assert!(drafts_anon.is_empty(), "anonymous view has empty list");

    room.finish().await;
}

#[actix_web::test]
async fn planner_draft_delete_by_another_owner_gives_out_of_scope() {
    let room = Room::start().await;
    let session_a = AppSessionId::generate();
    let session_b = AppSessionId::generate();

    let body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "100.0000" },
        "expectedVersion": null,
    });
    let res = call(
        &room,
        None,
        Some(session_a.as_str()),
        Method::POST,
        "/api/app/planner-drafts",
        Some((body, "create-del-key")),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    let draft_id = res.body["id"].as_str().expect("id string");

    let count_before = room
        .count(&format!(
            "SELECT count(*) FROM planner_drafts WHERE id = '{draft_id}'::uuid"
        ))
        .await;
    assert_eq!(count_before, 1);

    // Session B attempts to delete Session A's draft
    let del_res = call(
        &room,
        None,
        Some(session_b.as_str()),
        Method::POST,
        &format!("/api/app/planner-drafts/{draft_id}/delete"),
        Some((json!({}), "del-attempt-key")),
    )
    .await;
    assert_eq!(
        del_res.status,
        StatusCode::FORBIDDEN,
        "delete by other owner refused: {}",
        del_res.text
    );
    assert_eq!(del_res.code(), "outOfScope");

    // Verify draft was preserved
    let count_after = room
        .count(&format!(
            "SELECT count(*) FROM planner_drafts WHERE id = '{draft_id}'::uuid"
        ))
        .await;
    assert_eq!(count_after, 1, "draft is kept");

    room.finish().await;
}

#[actix_web::test]
async fn planner_preview_with_no_session_writes_no_rows() {
    let room = Room::start().await;

    let quotes_before = room.count("SELECT count(*) FROM quotes").await;
    let bookings_before = room.count("SELECT count(*) FROM bookings").await;
    let stock_before = room.count("SELECT count(*) FROM stock_movements").await;

    let preview_body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "100" },
    });

    let res = call(
        &room,
        None,
        None,
        Method::POST,
        "/api/v3/compute/planner_preview",
        Some((preview_body, "preview-key-1")),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "planner_preview succeeded: {}",
        res.text
    );
    assert!(
        res.body.get("thb").is_some() || res.body.get("data").is_some(),
        "preview result has estimate"
    );

    let quotes_after = room.count("SELECT count(*) FROM quotes").await;
    let bookings_after = room.count("SELECT count(*) FROM bookings").await;
    let stock_after = room.count("SELECT count(*) FROM stock_movements").await;

    assert_eq!(quotes_before, quotes_after, "no quote rows written");
    assert_eq!(bookings_before, bookings_after, "no booking rows written");
    assert_eq!(stock_before, stock_after, "no stock movement rows written");

    room.finish().await;
}

#[actix_web::test]
async fn app_settings_save_role_permissions_and_upsert() {
    let room = Room::start().await;

    let settings_payload = json!({
        "plannerDraftTtlSeconds": 86400,
    });

    // 1. TELLER is refused with roleNotAllowedHere
    let res_teller = call(
        &room,
        Some("TELLER"),
        None,
        Method::PUT,
        "/api/app-settings",
        Some((settings_payload.clone(), "settings-key-1")),
    )
    .await;
    assert_eq!(res_teller.status, StatusCode::BAD_REQUEST);
    assert_eq!(res_teller.code(), "roleNotAllowedHere");

    // 2. CUSTOMER is refused with roleNotAllowedHere
    let res_cust = call(
        &room,
        Some("CUSTOMER:C-01"),
        None,
        Method::PUT,
        "/api/app-settings",
        Some((settings_payload.clone(), "settings-key-2")),
    )
    .await;
    assert_eq!(res_cust.status, StatusCode::BAD_REQUEST);
    assert_eq!(res_cust.code(), "roleNotAllowedHere");

    // 3. ADMIN passes and updates settings
    let res_admin = call(
        &room,
        Some("ADMIN"),
        None,
        Method::PUT,
        "/api/app-settings",
        Some((settings_payload, "settings-key-3")),
    )
    .await;
    assert_eq!(
        res_admin.status,
        StatusCode::OK,
        "admin save succeeded: {}",
        res_admin.text
    );

    let ttl: i32 = sqlx::query_scalar(
        "SELECT planner_draft_ttl_seconds FROM customer_app_settings WHERE organization_id = $1::uuid",
    )
    .bind(ORG)
    .fetch_one(&room.pool)
    .await
    .expect("read settings ttl");
    assert_eq!(ttl, 86400);

    // 4. EXECUTIVE also holds SystemSettings and passes
    let exec_payload = json!({
        "plannerDraftTtlSeconds": 172800,
    });
    let res_exec = call(
        &room,
        Some("EXECUTIVE"),
        None,
        Method::PUT,
        "/api/app-settings",
        Some((exec_payload, "settings-key-4")),
    )
    .await;
    assert_eq!(
        res_exec.status,
        StatusCode::OK,
        "executive save succeeded: {}",
        res_exec.text
    );

    let ttl2: i32 = sqlx::query_scalar(
        "SELECT planner_draft_ttl_seconds FROM customer_app_settings WHERE organization_id = $1::uuid",
    )
    .bind(ORG)
    .fetch_one(&room.pool)
    .await
    .expect("read settings ttl");
    assert_eq!(ttl2, 172800);

    room.finish().await;
}

#[actix_web::test]
async fn the_21st_draft_write_in_a_minute_gives_rate_limited_429() {
    let room = Room::start().await;
    let session_id = AppSessionId::generate();

    for i in 1..=20 {
        let body = json!({
            "branch": MAIN,
            "currency": "USD",
            "customerSide": "CUSTOMER_BUY",
            "amount": { "quantity": "100.0000" },
            "expectedVersion": null,
        });
        let res = call(
            &room,
            None,
            Some(session_id.as_str()),
            Method::POST,
            "/api/app/planner-drafts",
            Some((body, &format!("rate-key-{i}"))),
        )
        .await;
        assert_eq!(res.status, StatusCode::OK, "write {i} admitted");
    }

    // The 21st draft write within 60s
    let body_21 = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "100.0000" },
        "expectedVersion": null,
    });
    let res_21 = call(
        &room,
        None,
        Some(session_id.as_str()),
        Method::POST,
        "/api/app/planner-drafts",
        Some((body_21, "rate-key-21")),
    )
    .await;
    assert_eq!(
        res_21.status,
        StatusCode::TOO_MANY_REQUESTS,
        "21st write rejected as 429: {}",
        res_21.text
    );
    assert_eq!(res_21.code(), "rateLimited");

    room.finish().await;
}
