//! Ticket #1565: a retried first anonymous save orphans its draft — the retry mints a second session cookie
//!
//! Up-front regression tests written before implementation by test-writer gemini/pool-5 on
//! lane controller claude/ad05bf8d's order.
//!
//! Cases:
//! 1. First anonymous planner draft save with Idempotency-Key K and no cookie, then retry with K and no cookie:
//!    - Cookie returned by retry can read (`my_planner_drafts`), update, and delete that draft.
//!    - Exactly one `planner_drafts` row in database.
//!    - Its `session_hash` matches lower-hex SHA-256 of the retry cookie value.
//! 2. Retry with K and the first cookie keeps that cookie (no new Set-Cookie, same draft replayed).
//! 3. Retry with K, a different body and no cookie returns `idempotencyConflict` and no Set-Cookie.
//! 4. Two different keys with no cookie create two different sessions.

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

use xc_app::domain::app_session::{APP_SESSION_COOKIE, AppSessionId};
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

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
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
        let schema = unique("app4_anon_retry");
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

        room.publish_usd().await;
        room
    }

    async fn publish_usd(&self) {
        let approval: Uuid = sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
             proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
             VALUES (gen_random_uuid(), 'RATE_PUBLISH', $1, $2, 'h', 'p', '{}', 'test', now(), 'APPROVED', $2, '00000000-0000-0000-0000-000000000001') \
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

fn extract_app_sid(set_cookie: &[String]) -> Option<String> {
    set_cookie.iter().find_map(|c| {
        for part in c.split(';') {
            let trimmed = part.trim();
            if let Some(val) = trimmed.strip_prefix(&format!("{APP_SESSION_COOKIE}=")) {
                return Some(val.to_owned());
            }
        }
        None
    })
}

// ------------------------------------------------------------ regression tests

/// Case 1: First anonymous planner draft save with Idempotency-Key K and no cookie,
/// then a retry with K and NO cookie:
/// - The cookie the retry returns can read (`my_planner_drafts`), update, and delete that draft.
/// - Exactly one `planner_drafts` row exists in the database.
/// - Its `session_hash` = lower-hex SHA-256 of the retry cookie value.
///
/// On main, this fails by assertion: the retry mints a new random session cookie which
/// differs from the draft's session_hash, leaving the draft orphaned.
#[actix_web::test]
async fn case_1_anonymous_retry_without_cookie_owns_draft_and_binds_session_hash() {
    let room = Room::start().await;
    let idem_key = unique("idem_k1");

    let initial_body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "500.0000" },
        "expectedVersion": null,
    });

    // 1. First anonymous planner draft save with Idempotency-Key K and no cookie
    let res1 = call(
        &room,
        None,
        None,
        Method::POST,
        "/api/app/planner-drafts",
        Some((initial_body.clone(), &idem_key)),
    )
    .await;
    assert_eq!(
        res1.status,
        StatusCode::OK,
        "first save succeeded: {}",
        res1.text
    );
    let draft_id = res1.body["id"]
        .as_str()
        .expect("draft id in res1")
        .to_owned();
    assert_eq!(res1.body["version"].as_i64(), Some(1));
    let first_cookie =
        extract_app_sid(&res1.set_cookie).expect("first save returns xc_app_sid cookie");

    // 2. Retry with K and NO cookie (lost response replay)
    let res2 = call(
        &room,
        None,
        None,
        Method::POST,
        "/api/app/planner-drafts",
        Some((initial_body, &idem_key)),
    )
    .await;
    assert_eq!(
        res2.status,
        StatusCode::OK,
        "retry save succeeded: {}",
        res2.text
    );
    assert_eq!(
        res2.body["id"].as_str(),
        Some(draft_id.as_str()),
        "retry replayed same draft id"
    );
    let retry_cookie = extract_app_sid(&res2.set_cookie).expect("retry returns xc_app_sid cookie");

    // 3. Exactly one planner_drafts row in the database
    let draft_count: i64 = sqlx::query_scalar("SELECT count(*) FROM planner_drafts")
        .fetch_one(&room.pool)
        .await
        .expect("count planner_drafts");
    assert_eq!(draft_count, 1, "exactly one planner_drafts row exists");

    // 4. Its session_hash = lower-hex SHA-256 of the retry cookie value
    let retry_session = AppSessionId::parse(&retry_cookie).unwrap_or_else(|| {
        panic!("retry cookie '{retry_cookie}' is not a valid 64-hex AppSessionId")
    });
    let expected_hash = retry_session.hash();

    let db_session_hash: Option<String> =
        sqlx::query_scalar("SELECT session_hash FROM planner_drafts WHERE id = $1::uuid")
            .bind(Uuid::parse_str(&draft_id).expect("uuid"))
            .fetch_one(&room.pool)
            .await
            .expect("fetch session_hash from db");

    assert_eq!(
        db_session_hash.as_deref(),
        Some(expected_hash.as_str()),
        "planner_drafts.session_hash must match lower-hex SHA-256 of retry cookie (first={first_cookie}, retry={retry_cookie})"
    );

    // 5. The cookie the retry returns can read (my_planner_drafts)
    let view_res = call(
        &room,
        None,
        Some(&retry_cookie),
        Method::GET,
        "/api/v3/views/my_planner_drafts",
        None,
    )
    .await;
    assert_eq!(
        view_res.status,
        StatusCode::OK,
        "my_planner_drafts succeeded: {}",
        view_res.text
    );
    let drafts = view_res.body["data"]["drafts"]
        .as_array()
        .or_else(|| view_res.body["data"].as_array())
        .expect("drafts array in view");
    assert_eq!(drafts.len(), 1, "retry session can see the draft");
    assert_eq!(drafts[0]["id"].as_str(), Some(draft_id.as_str()));

    // 6. The cookie the retry returns can update that draft
    let update_body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "600.0000" },
        "expectedVersion": 1,
    });
    let update_key = unique("update_k");
    let update_res = call(
        &room,
        None,
        Some(&retry_cookie),
        Method::PUT,
        &format!("/api/app/planner-drafts/{draft_id}"),
        Some((update_body, &update_key)),
    )
    .await;
    assert_eq!(
        update_res.status,
        StatusCode::OK,
        "update succeeded: {}",
        update_res.text
    );
    assert_eq!(
        update_res.body["version"].as_i64(),
        Some(2),
        "updated to version 2"
    );

    // 7. The cookie the retry returns can delete that draft
    let delete_key = unique("delete_k");
    let delete_res = call(
        &room,
        None,
        Some(&retry_cookie),
        Method::POST,
        &format!("/api/app/planner-drafts/{draft_id}/delete"),
        Some((json!({}), &delete_key)),
    )
    .await;
    assert_eq!(
        delete_res.status,
        StatusCode::OK,
        "delete succeeded: {}",
        delete_res.text
    );

    let draft_count_after: i64 = sqlx::query_scalar("SELECT count(*) FROM planner_drafts")
        .fetch_one(&room.pool)
        .await
        .expect("count planner_drafts after delete");
    assert_eq!(draft_count_after, 0, "draft was deleted");

    room.finish().await;
}

/// Case 2: Retry with K AND the first cookie keeps that cookie (no new Set-Cookie, same draft).
/// Passes on main as a guard.
#[actix_web::test]
async fn case_2_anonymous_retry_with_first_cookie_keeps_cookie_and_draft() {
    let room = Room::start().await;
    let idem_key = unique("idem_k2");

    let body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "500.0000" },
        "expectedVersion": null,
    });

    // 1. First anonymous draft save with K and no cookie
    let res1 = call(
        &room,
        None,
        None,
        Method::POST,
        "/api/app/planner-drafts",
        Some((body.clone(), &idem_key)),
    )
    .await;
    assert_eq!(
        res1.status,
        StatusCode::OK,
        "first save succeeded: {}",
        res1.text
    );
    let draft_id = res1.body["id"]
        .as_str()
        .expect("draft id in res1")
        .to_owned();
    let first_cookie =
        extract_app_sid(&res1.set_cookie).expect("first save returns xc_app_sid cookie");

    // 2. Retry with K AND the first cookie
    let res2 = call(
        &room,
        None,
        Some(&first_cookie),
        Method::POST,
        "/api/app/planner-drafts",
        Some((body, &idem_key)),
    )
    .await;
    assert_eq!(
        res2.status,
        StatusCode::OK,
        "retry succeeded: {}",
        res2.text
    );
    assert_eq!(
        res2.body["id"].as_str(),
        Some(draft_id.as_str()),
        "same draft replayed"
    );

    // Keeps that cookie: no new Set-Cookie issued
    let retry_cookie = extract_app_sid(&res2.set_cookie);
    assert!(
        retry_cookie.is_none(),
        "retry with existing cookie must not issue a new Set-Cookie, got: {:?}",
        retry_cookie
    );

    // Still owns and can read the draft
    let view_res = call(
        &room,
        None,
        Some(&first_cookie),
        Method::GET,
        "/api/v3/views/my_planner_drafts",
        None,
    )
    .await;
    assert_eq!(view_res.status, StatusCode::OK);
    let drafts = view_res.body["data"]["drafts"]
        .as_array()
        .or_else(|| view_res.body["data"].as_array())
        .expect("drafts array in view");
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0]["id"].as_str(), Some(draft_id.as_str()));

    room.finish().await;
}

/// Case 3: Retry with K, a different body and no cookie => idempotencyConflict and no Set-Cookie.
/// Passes on main as a guard.
#[actix_web::test]
async fn case_3_anonymous_retry_different_body_without_cookie_conflicts_and_no_cookie() {
    let room = Room::start().await;
    let idem_key = unique("idem_k3");

    let body1 = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "500.0000" },
        "expectedVersion": null,
    });

    // 1. First anonymous draft save with K and no cookie
    let res1 = call(
        &room,
        None,
        None,
        Method::POST,
        "/api/app/planner-drafts",
        Some((body1, &idem_key)),
    )
    .await;
    assert_eq!(
        res1.status,
        StatusCode::OK,
        "first save succeeded: {}",
        res1.text
    );
    assert!(extract_app_sid(&res1.set_cookie).is_some());

    // 2. Retry with K, a DIFFERENT body and NO cookie
    let body2 = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_SELL",
        "amount": { "quantity": "999.0000" },
        "expectedVersion": null,
    });
    let res2 = call(
        &room,
        None,
        None,
        Method::POST,
        "/api/app/planner-drafts",
        Some((body2, &idem_key)),
    )
    .await;
    assert_eq!(
        res2.status,
        StatusCode::CONFLICT,
        "conflicting retry must return 409 Conflict: {}",
        res2.text
    );
    assert_eq!(res2.code(), "idempotencyConflict");

    // No Set-Cookie
    let conflict_cookie = extract_app_sid(&res2.set_cookie);
    assert!(
        conflict_cookie.is_none(),
        "conflicting retry must not issue Set-Cookie, got: {:?}",
        conflict_cookie
    );

    room.finish().await;
}

/// Case 4: Two different keys with no cookie => two different sessions.
/// Passes on main as a guard.
#[actix_web::test]
async fn case_4_two_different_keys_without_cookie_create_two_different_sessions() {
    let room = Room::start().await;
    let key1 = unique("key_1");
    let key2 = unique("key_2");

    let body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "100.0000" },
        "expectedVersion": null,
    });

    // 1. First anonymous draft save with key1 and no cookie
    let res1 = call(
        &room,
        None,
        None,
        Method::POST,
        "/api/app/planner-drafts",
        Some((body.clone(), &key1)),
    )
    .await;
    assert_eq!(
        res1.status,
        StatusCode::OK,
        "save 1 succeeded: {}",
        res1.text
    );
    let id1 = res1.body["id"].as_str().expect("id1").to_owned();
    let sid1 = extract_app_sid(&res1.set_cookie).expect("sid1");

    // 2. Second anonymous draft save with key2 and no cookie
    let res2 = call(
        &room,
        None,
        None,
        Method::POST,
        "/api/app/planner-drafts",
        Some((body, &key2)),
    )
    .await;
    assert_eq!(
        res2.status,
        StatusCode::OK,
        "save 2 succeeded: {}",
        res2.text
    );
    let id2 = res2.body["id"].as_str().expect("id2").to_owned();
    let sid2 = extract_app_sid(&res2.set_cookie).expect("sid2");

    // Must be two different sessions
    assert_ne!(
        sid1, sid2,
        "different keys without cookie must create different sessions"
    );

    // Each session sees only its own draft
    let v1 = call(
        &room,
        None,
        Some(&sid1),
        Method::GET,
        "/api/v3/views/my_planner_drafts",
        None,
    )
    .await;
    assert_eq!(v1.status, StatusCode::OK);
    let drafts1 = v1.body["data"]["drafts"]
        .as_array()
        .or_else(|| v1.body["data"].as_array())
        .expect("drafts1");
    assert_eq!(drafts1.len(), 1);
    assert_eq!(drafts1[0]["id"].as_str(), Some(id1.as_str()));

    let v2 = call(
        &room,
        None,
        Some(&sid2),
        Method::GET,
        "/api/v3/views/my_planner_drafts",
        None,
    )
    .await;
    assert_eq!(v2.status, StatusCode::OK);
    let drafts2 = v2.body["data"]["drafts"]
        .as_array()
        .or_else(|| v2.body["data"].as_array())
        .expect("drafts2");
    assert_eq!(drafts2.len(), 1);
    assert_eq!(drafts2[0]["id"].as_str(), Some(id2.as_str()));

    room.finish().await;
}
