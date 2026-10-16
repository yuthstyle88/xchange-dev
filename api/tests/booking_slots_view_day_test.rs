//! Ticket #2345 — `booking_slots` view:
//! - a malformed `date` filter (`?date=2026-13-45`, `?date=tomorrow`) answers 400 `invalidRequest`
//!   with `field: "date"`, not 200 with today's slots;
//! - when `date` is omitted, "today" is computed in the branch's own `timezone`, not hard-coded
//!   `Asia/Bangkok`.

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
use chrono::{NaiveDate, Utc};
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const OWNER: &str = "owner";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The booking_slots view tests need a real PostgreSQL and do not skip. Point it at \
                 an existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`."
        );
        let schema = unique("v3bookingslotsday");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
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
            .expect("apply api/migrations to the test schema");
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01')")
            .execute(&pool)
            .await
            .expect("insert XC-B01 into branches");
        Self { pool, schema }
    }

    async fn seed_slot(
        &self,
        branch_id: &str,
        service_date: NaiveDate,
        hour_offset: i64,
        status: &str,
    ) -> Uuid {
        let slot_id = Uuid::new_v4();
        let start_at = Utc::now() + chrono::Duration::hours(hour_offset);
        let end_at = start_at + chrono::Duration::hours(1);
        sqlx::query(
            "INSERT INTO booking_slots \
               (id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status) \
             SELECT $1, organization_id, id, $2, $3, $4, 5, 1, $5 \
             FROM branches WHERE id = $6",
        )
        .bind(slot_id)
        .bind(service_date)
        .bind(start_at)
        .bind(end_at)
        .bind(status)
        .bind(branch_id)
        .execute(&self.pool)
        .await
        .expect("insert booking_slot");
        slot_id
    }

    async fn set_branch_timezone(&self, branch_id: &str, tz: &str) {
        sqlx::query("UPDATE branches SET timezone = $1 WHERE id = $2")
            .bind(tz)
            .bind(branch_id)
            .execute(&self.pool)
            .await
            .expect("update branch timezone");
    }

    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

struct HeaderSessions;

fn sessions_of(spec: &str) -> Sessions {
    if spec == OWNER {
        return Sessions {
            owner: Some(Actor::owner("owner-1")),
            staff: None,
        };
    }
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = Role::parse(parts.next().expect("a role")).expect("a known role");
    let branch = parts.next().expect("a branch");
    Sessions {
        owner: None,
        staff: Actor::staff(
            format!("employee-{}", role.as_str()),
            role,
            BranchId::new(branch),
        ),
    }
}

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { sessions_of(&spec) })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn get_view(db: &TestDb, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::GET)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, OWNER))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN));
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn slot_ids(ans: &Answer) -> Vec<String> {
    ans.body["data"]["slots"]
        .as_array()
        .unwrap_or_else(|| panic!("expected data.slots array in {}", ans.text))
        .iter()
        .map(|s| {
            s["id"]
                .as_str()
                .unwrap_or_else(|| panic!("expected slot id string in {}", ans.text))
                .to_owned()
        })
        .collect()
}

// Row 1: `date=2026-13-45` -> 400 `invalidRequest`, field `date`
#[actix_web::test]
async fn row_1_out_of_range_date_answers_400_invalid_request_on_date() {
    let db = TestDb::start().await;
    let bkk_today = Utc::now()
        .with_timezone(&chrono_tz::Asia::Bangkok)
        .date_naive();
    db.seed_slot(B01, bkk_today, 1, "OPEN").await;

    let ans = get_view(
        &db,
        &format!("/api/v3/views/booking_slots?branch={B01}&date=2026-13-45"),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "date=2026-13-45 must answer 400; got {} {}",
        ans.status,
        ans.text
    );
    assert_eq!(ans.body["error"]["code"], "invalidRequest", "{}", ans.text);
    assert_eq!(ans.body["error"]["field"], "date", "{}", ans.text);

    db.finish().await;
}

// Row 2: `date=tomorrow` -> 400 `invalidRequest`, field `date`
#[actix_web::test]
async fn row_2_non_iso_date_answers_400_invalid_request_on_date() {
    let db = TestDb::start().await;
    let bkk_today = Utc::now()
        .with_timezone(&chrono_tz::Asia::Bangkok)
        .date_naive();
    db.seed_slot(B01, bkk_today, 1, "OPEN").await;

    let ans = get_view(
        &db,
        &format!("/api/v3/views/booking_slots?branch={B01}&date=tomorrow"),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "date=tomorrow must answer 400; got {} {}",
        ans.status,
        ans.text
    );
    assert_eq!(ans.body["error"]["code"], "invalidRequest", "{}", ans.text);
    assert_eq!(ans.body["error"]["field"], "date", "{}", ans.text);

    db.finish().await;
}

// Row 3: `date=2026-10-20` (valid) with an OPEN slot that day -> 200, that slot (keep)
#[actix_web::test]
async fn row_3_valid_date_lists_open_slot_for_that_day() {
    let db = TestDb::start().await;
    let target_date = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
    let expected_slot = db.seed_slot(B01, target_date, 24, "OPEN").await;

    let ans = get_view(
        &db,
        &format!("/api/v3/views/booking_slots?branch={B01}&date=2026-10-20"),
    )
    .await;

    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(slot_ids(&ans), vec![expected_slot.to_string()]);
    assert_eq!(ans.body["data"]["slots"][0]["available"], 4);

    db.finish().await;
}

// Row 4: no `date`, branch timezone `Asia/Bangkok` -> today in Bangkok (keep)
#[actix_web::test]
async fn row_4_no_date_with_bangkok_branch_lists_bangkok_today() {
    let db = TestDb::start().await;
    db.set_branch_timezone(B01, "Asia/Bangkok").await;
    let bkk_today = Utc::now()
        .with_timezone(&chrono_tz::Asia::Bangkok)
        .date_naive();
    let expected_slot = db.seed_slot(B01, bkk_today, 1, "OPEN").await;

    let ans = get_view(&db, &format!("/api/v3/views/booking_slots?branch={B01}")).await;

    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(slot_ids(&ans), vec![expected_slot.to_string()]);

    db.finish().await;
}

// Row 5: no `date`, branch timezone whose local date != Bangkok's now
// (pick `Pacific/Kiritimati` or `Etc/GMT+12`, whichever differs at test time) ->
// the branch-local day's slots; Bangkok's day's slots are not listed
#[actix_web::test]
async fn row_5_no_date_uses_branch_timezone_instead_of_bangkok() {
    let db = TestDb::start().await;
    let now = Utc::now();
    let bkk_date = now.with_timezone(&chrono_tz::Asia::Bangkok).date_naive();

    let (tz_name, branch_date) = [
        (
            "Etc/GMT+12",
            now.with_timezone(&chrono_tz::Etc::GMTPlus12).date_naive(),
        ),
        (
            "Pacific/Kiritimati",
            now.with_timezone(&chrono_tz::Pacific::Kiritimati)
                .date_naive(),
        ),
    ]
    .into_iter()
    .find(|(_, d)| *d != bkk_date)
    .expect("either Etc/GMT+12 (UTC-12) or Pacific/Kiritimati (UTC+14) differs from Asia/Bangkok (UTC+7) at every instant");

    db.set_branch_timezone(B01, tz_name).await;
    let branch_slot = db.seed_slot(B01, branch_date, 1, "OPEN").await;
    let _bangkok_slot = db.seed_slot(B01, bkk_date, 2, "OPEN").await;

    let ans = get_view(&db, &format!("/api/v3/views/booking_slots?branch={B01}")).await;

    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(
        slot_ids(&ans),
        vec![branch_slot.to_string()],
        "must list the branch-local day ({branch_date} in {tz_name}) and not Bangkok's day ({bkk_date}); got {}",
        ans.text
    );

    db.finish().await;
}

// Row 6: valid `date`, CLOSED slot that day -> not listed (keep)
#[actix_web::test]
async fn row_6_closed_slot_on_valid_date_is_not_listed() {
    let db = TestDb::start().await;
    let target_date = NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
    db.seed_slot(B01, target_date, 24, "CLOSED").await;

    let ans = get_view(
        &db,
        &format!("/api/v3/views/booking_slots?branch={B01}&date=2026-10-21"),
    )
    .await;

    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert!(
        slot_ids(&ans).is_empty(),
        "CLOSED slot must not be listed; got {}",
        ans.text
    );

    db.finish().await;
}
