//! Up-front tests for APP-3 (#993): customer reach of my_notifications/notificationRead + Q-1.
//!
//! Behaviour table from lane controller claude/ad05bf8d on #993:
//! 1. A CUSTOMER-only user (no staff role) with an app session reads GET /api/v3/views/my_notifications → 200, with their own rows only.
//! 2. That user's notificationRead on their own row → 200. On another user's row → outOfScope.
//! 3. After notificationRead, polling my_notifications with since=<the version before the read> → changed:true, and the row shows readAt.
//! 4. After markAllRead, the same poll → changed:true.
//! 5. A second notificationRead of the same row → the version does not move: changed:false with since=<version after the first read>.
//! 6. No session → signIn, as today.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_app::domain::app_session::{APP_SESSION_COOKIE, AppSessionId};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const B1: &str = "XC-B01";
const AS: &str = "x-test-as";

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
        let schema = unique("app3_cust_notify");
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
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, 'B01') ON CONFLICT DO NOTHING")
            .bind(B1)
            .execute(&pool)
            .await
            .expect("a branch");
        Room { pool, schema }
    }

    async fn finish(self) {
        let Room { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ callers

fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

/// A CUSTOMER-only user: no v2 owner or staff actor, Role::Customer, customer_id set.
fn customer(user_id: &str) -> Actor {
    Actor::new(
        user_id,
        ORG,
        vec![ScopedRole::new(Role::Customer, None, epoch(), None)],
        Some(user_id),
    )
}

struct HeaderCallers;

fn user_of(req: &HttpRequest) -> Option<Actor> {
    let spec = req.headers().get(AS)?.to_str().ok()?.to_owned();
    match spec.as_str() {
        "customer" => Some(customer("customer-1")),
        other => other.strip_prefix("customer:").map(customer),
    }
}

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
        let user = user_of(req);
        Box::pin(async move {
            Callers {
                user,
                staff_user: None,
            }
        })
    }
}

// ------------------------------------------------------------ app & request helpers

struct Answer {
    status: StatusCode,
    body: Value,
}

#[derive(Default, Clone)]
struct FromCaller {
    who: Option<String>,
    cookie: Option<String>,
}

fn anonymous() -> FromCaller {
    FromCaller::default()
}

fn customer_session(user_id: &str) -> FromCaller {
    let id = AppSessionId::generate();
    FromCaller {
        who: Some(format!("customer:{user_id}")),
        cookie: Some(id.as_str().to_owned()),
    }
}

async fn call(room: &Room, req: atest::TestRequest, from: FromCaller) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| {
                bootstrap::configure(cfg, state);
            })
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = req.insert_header((header::HOST, HOST));
    if let Some(who) = from.who {
        req = req.insert_header((AS, who));
    }
    if let Some(id) = from.cookie {
        req = req.insert_header((header::COOKIE, format!("{APP_SESSION_COOKIE}={id}")));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    Answer { status, body }
}

fn command_request(route: &str, body: Value) -> atest::TestRequest {
    atest::TestRequest::post()
        .uri(route)
        .insert_header(("x-xchange-request", "1"))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("idempotency-key", unique("key")))
        .set_json(body)
}

async fn create_user(room: &Room, name: &str) -> String {
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, $2, 'ACTIVE', 'app3-test') RETURNING id::text",
    )
    .bind(ORG)
    .bind(name)
    .fetch_one(&room.pool)
    .await
    .expect("a user")
}

async fn seed_notification(room: &Room, user_id: &str) -> String {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO notifications (id, organization_id, recipient_user_id, event_id, template_key, \
           template_version, locale) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, 'app3.test', 1, 'th')",
    )
    .bind(&id)
    .bind(ORG)
    .bind(user_id)
    .bind(unique("event"))
    .execute(&room.pool)
    .await
    .expect("a notification");
    id
}

async fn get_read_at(room: &Room, id: &str) -> Option<DateTime<Utc>> {
    sqlx::query_scalar("SELECT read_at FROM notifications WHERE id = $1::uuid")
        .bind(id)
        .fetch_one(&room.pool)
        .await
        .expect("the notification")
}

// ------------------------------------------------------------ behaviour table tests

/// Behaviour row 1:
/// A CUSTOMER-only user (no staff role) with an app session reads GET /api/v3/views/my_notifications → 200, with their own rows only.
#[actix_web::test]
async fn row1_customer_reads_my_notifications_sees_own_rows_only() {
    common::install_test_pii_keys();
    let room = Room::start().await;

    let user_a = create_user(&room, "Customer A").await;
    let user_b = create_user(&room, "Customer B").await;

    let notif_a1 = seed_notification(&room, &user_a).await;
    let notif_a2 = seed_notification(&room, &user_a).await;
    let notif_b1 = seed_notification(&room, &user_b).await;

    let resp = call(
        &room,
        atest::TestRequest::get().uri(&format!("/api/v3/views/my_notifications?branch={B1}")),
        customer_session(&user_a),
    )
    .await;

    assert_eq!(resp.status, StatusCode::OK, "{}", resp.body);
    assert_eq!(resp.body["changed"], true);
    let items = resp.body["data"]["items"].as_array().expect("items array");
    assert_eq!(
        items.len(),
        2,
        "only customer A's notifications are returned"
    );
    let ids: Vec<&str> = items.iter().filter_map(|i| i["id"].as_str()).collect();
    assert!(ids.contains(&notif_a1.as_str()));
    assert!(ids.contains(&notif_a2.as_str()));
    assert!(
        !ids.contains(&notif_b1.as_str()),
        "customer B's notification must not appear"
    );

    room.finish().await;
}

/// Behaviour row 2:
/// That user's notificationRead on their own row → 200. On another user's row → outOfScope.
#[actix_web::test]
async fn row2_customer_notification_read_own_row_200_other_row_out_of_scope() {
    common::install_test_pii_keys();
    let room = Room::start().await;

    let user_a = create_user(&room, "Customer A").await;
    let user_b = create_user(&room, "Customer B").await;

    let notif_a = seed_notification(&room, &user_a).await;
    let notif_b = seed_notification(&room, &user_b).await;

    // 1. notificationRead on own row -> 200
    let resp_own = call(
        &room,
        command_request(
            &format!("/api/app/notifications/{notif_a}/read"),
            json!({ "branch": B1 }),
        ),
        customer_session(&user_a),
    )
    .await;
    assert_eq!(resp_own.status, StatusCode::OK, "{}", resp_own.body);
    assert!(
        resp_own.body["readAt"].is_string(),
        "readAt must be present in output: {}",
        resp_own.body
    );
    assert!(
        get_read_at(&room, &notif_a).await.is_some(),
        "db read_at must be set"
    );

    // 2. notificationRead on another user's row -> outOfScope (403)
    let resp_other = call(
        &room,
        command_request(
            &format!("/api/app/notifications/{notif_b}/read"),
            json!({ "branch": B1 }),
        ),
        customer_session(&user_a),
    )
    .await;
    assert_eq!(
        resp_other.status,
        StatusCode::FORBIDDEN,
        "{}",
        resp_other.body
    );
    assert_eq!(
        resp_other.body["error"]["code"], "outOfScope",
        "{}",
        resp_other.body
    );
    assert!(
        get_read_at(&room, &notif_b).await.is_none(),
        "other user's notification must stay unread"
    );

    room.finish().await;
}

/// Behaviour row 3:
/// After notificationRead, polling my_notifications with since=<the version before the read> → changed:true, and the row shows readAt.
#[actix_web::test]
async fn row3_after_notification_read_poll_since_shows_changed_true_and_read() {
    common::install_test_pii_keys();
    let room = Room::start().await;

    let user = create_user(&room, "Customer").await;
    let notif = seed_notification(&room, &user).await;

    // Initial read to get the version before the read
    let initial = call(
        &room,
        atest::TestRequest::get().uri(&format!("/api/v3/views/my_notifications?branch={B1}")),
        customer_session(&user),
    )
    .await;
    assert_eq!(initial.status, StatusCode::OK, "{}", initial.body);
    assert_eq!(initial.body["changed"], true);
    let v_before = initial.body["version"].as_i64().expect("version number");
    assert_eq!(initial.body["data"]["items"][0]["read"], false);

    // Perform notificationRead
    let read_resp = call(
        &room,
        command_request(
            &format!("/api/app/notifications/{notif}/read"),
            json!({ "branch": B1 }),
        ),
        customer_session(&user),
    )
    .await;
    assert_eq!(read_resp.status, StatusCode::OK, "{}", read_resp.body);

    // Poll with since = v_before
    let poll = call(
        &room,
        atest::TestRequest::get().uri(&format!(
            "/api/v3/views/my_notifications?branch={B1}&since={v_before}"
        )),
        customer_session(&user),
    )
    .await;
    assert_eq!(poll.status, StatusCode::OK, "{}", poll.body);
    assert_eq!(
        poll.body["changed"], true,
        "polling with version before read must report changed:true under Q-1 (got {})",
        poll.body
    );
    let items = poll.body["data"]["items"]
        .as_array()
        .expect("items present when changed:true");
    assert_eq!(
        items[0]["read"], true,
        "the notification must now show read:true"
    );

    room.finish().await;
}

/// Behaviour row 4:
/// After markAllRead, the same poll → changed:true.
#[actix_web::test]
async fn row4_after_mark_all_read_poll_since_shows_changed_true() {
    common::install_test_pii_keys();
    let room = Room::start().await;

    let user = create_user(&room, "Customer").await;
    let _notif1 = seed_notification(&room, &user).await;
    let _notif2 = seed_notification(&room, &user).await;

    // Initial read to get version before markAllRead
    let initial = call(
        &room,
        atest::TestRequest::get().uri(&format!("/api/v3/views/my_notifications?branch={B1}")),
        customer_session(&user),
    )
    .await;
    assert_eq!(initial.status, StatusCode::OK, "{}", initial.body);
    let v_before = initial.body["version"].as_i64().expect("version number");

    // Perform notificationReadAll
    let read_all_resp = call(
        &room,
        command_request("/api/app/notifications/read-all", json!({ "branch": B1 })),
        customer_session(&user),
    )
    .await;
    assert_eq!(
        read_all_resp.status,
        StatusCode::OK,
        "{}",
        read_all_resp.body
    );
    assert_eq!(read_all_resp.body["count"], 2);

    // Poll with since = v_before
    let poll = call(
        &room,
        atest::TestRequest::get().uri(&format!(
            "/api/v3/views/my_notifications?branch={B1}&since={v_before}"
        )),
        customer_session(&user),
    )
    .await;
    assert_eq!(poll.status, StatusCode::OK, "{}", poll.body);
    assert_eq!(
        poll.body["changed"], true,
        "polling with version before markAllRead must report changed:true under Q-1 (got {})",
        poll.body
    );
    let items = poll.body["data"]["items"]
        .as_array()
        .expect("items present when changed:true");
    assert!(
        items.iter().all(|i| i["read"] == true),
        "all items must show read:true"
    );

    room.finish().await;
}

/// Behaviour row 5:
/// A second notificationRead of the same row → the version does not move: changed:false with since=<version after the first read>.
#[actix_web::test]
async fn row5_second_notification_read_version_does_not_move() {
    common::install_test_pii_keys();
    let room = Room::start().await;

    let user = create_user(&room, "Customer").await;
    let notif = seed_notification(&room, &user).await;

    // First notificationRead
    let read1 = call(
        &room,
        command_request(
            &format!("/api/app/notifications/{notif}/read"),
            json!({ "branch": B1 }),
        ),
        customer_session(&user),
    )
    .await;
    assert_eq!(read1.status, StatusCode::OK, "{}", read1.body);

    // Read view to get version after the first read
    let view_after_first = call(
        &room,
        atest::TestRequest::get().uri(&format!("/api/v3/views/my_notifications?branch={B1}")),
        customer_session(&user),
    )
    .await;
    assert_eq!(
        view_after_first.status,
        StatusCode::OK,
        "{}",
        view_after_first.body
    );
    let v_after_first = view_after_first.body["version"]
        .as_i64()
        .expect("version number");

    // Second notificationRead of the same row
    let read2 = call(
        &room,
        command_request(
            &format!("/api/app/notifications/{notif}/read"),
            json!({ "branch": B1 }),
        ),
        customer_session(&user),
    )
    .await;
    assert_eq!(read2.status, StatusCode::OK, "{}", read2.body);

    // Poll with since = v_after_first
    let poll = call(
        &room,
        atest::TestRequest::get().uri(&format!(
            "/api/v3/views/my_notifications?branch={B1}&since={v_after_first}"
        )),
        customer_session(&user),
    )
    .await;
    assert_eq!(poll.status, StatusCode::OK, "{}", poll.body);
    assert_eq!(
        poll.body["changed"], false,
        "second read of same row must not advance version: changed:false (got {})",
        poll.body
    );
    assert!(
        poll.body["data"].is_null(),
        "data must be null when changed:false"
    );

    room.finish().await;
}

/// Behaviour row 6:
/// No session → signIn, as today.
#[actix_web::test]
async fn row6_no_session_answers_sign_in() {
    common::install_test_pii_keys();
    let room = Room::start().await;

    let dummy_id = Uuid::new_v4().to_string();

    // 1. my_notifications view without session
    let resp_view = call(
        &room,
        atest::TestRequest::get().uri(&format!("/api/v3/views/my_notifications?branch={B1}")),
        anonymous(),
    )
    .await;
    assert_eq!(
        resp_view.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        resp_view.body
    );
    assert_eq!(
        resp_view.body["error"]["code"], "signIn",
        "{}",
        resp_view.body
    );

    // 2. notificationRead command without session
    let resp_read = call(
        &room,
        command_request(
            &format!("/api/app/notifications/{dummy_id}/read"),
            json!({ "branch": B1 }),
        ),
        anonymous(),
    )
    .await;
    assert_eq!(
        resp_read.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        resp_read.body
    );
    assert_eq!(
        resp_read.body["error"]["code"], "signIn",
        "{}",
        resp_read.body
    );

    // 3. notificationReadAll command without session
    let resp_read_all = call(
        &room,
        command_request("/api/app/notifications/read-all", json!({ "branch": B1 })),
        anonymous(),
    )
    .await;
    assert_eq!(
        resp_read_all.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        resp_read_all.body
    );
    assert_eq!(
        resp_read_all.body["error"]["code"], "signIn",
        "{}",
        resp_read_all.body
    );

    room.finish().await;
}
