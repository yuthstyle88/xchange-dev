//! EX-8 #1190: wiring test proving A01 as wired (`ExecutiveOverviewView::new`)
//! no longer answers `awaitsBookingDesign` once a booking cohort exists.

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
use chrono::{Duration as ChronoDuration, NaiveDate, NaiveTime, TimeZone, Utc};
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::branch_scope::BranchScope;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_kernel::ids::BranchId;
use xchange::application::executive::ports::ExecutiveFilter;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http::v3::executive_overview::ExecutiveOverviewView;
use xchange::presentation::http::v3::org_view::OrgView;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const HOST: &str = "xc.test";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const B1: &str = "XC-B01";
const C1: &str = "00000000-0000-0000-0000-0000000000c1";
const EXECUTIVE: &str = "00000000-0000-0000-0000-0000000000e1";

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
        let schema = unique("ex8_overview_wiring");
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

    async fn seed_org(&self, id: &str, code: &str, timezone: &str) {
        sqlx::query(
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ($1::uuid, $2, $3, now()) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(code)
        .bind(timezone)
        .execute(&self.pool)
        .await
        .expect("seed org");
    }

    async fn seed_branch(&self, org_id: &str, branch_id: &str, timezone: &str) {
        self.seed_org(org_id, &unique("ORG"), timezone).await;
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id, timezone) \
             VALUES ($1, $1, $2::uuid, $3) \
             ON CONFLICT (id) DO UPDATE SET organization_id = EXCLUDED.organization_id, timezone = EXCLUDED.timezone",
        )
        .bind(branch_id)
        .bind(org_id)
        .bind(timezone)
        .execute(&self.pool)
        .await
        .expect("seed branch");

        sqlx::query(
            "INSERT INTO branch_versions (branch_id, version, organization_id, changed_at) \
             VALUES ($1, 1, $2::uuid, now()) \
             ON CONFLICT (branch_id) DO NOTHING",
        )
        .bind(branch_id)
        .bind(org_id)
        .execute(&self.pool)
        .await
        .expect("seed branch version");
    }

    async fn seed_customer(&self, org_id: &str, id: &str, mode: &str) {
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, status, created_at) \
             VALUES ($1, $2::uuid, $1, 'Test Customer', $3, 'ACTIVE', now())",
        )
        .bind(id)
        .bind(org_id)
        .bind(mode)
        .execute(&self.pool)
        .await
        .expect("seed customer");
    }

    async fn seed_slot(
        &self,
        org_id: &str,
        branch_id: &str,
        service_date: NaiveDate,
        start_time: NaiveTime,
        duration_mins: i64,
    ) -> Uuid {
        let slot_id = Uuid::new_v4();
        let start_at = service_date.and_time(start_time).and_utc();
        let end_at = start_at + ChronoDuration::minutes(duration_mins);
        sqlx::query(
            "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6, 10, 0, 'OPEN')",
        )
        .bind(slot_id)
        .bind(org_id)
        .bind(branch_id)
        .bind(service_date)
        .bind(start_at)
        .bind(end_at)
        .execute(&self.pool)
        .await
        .expect("seed slot");
        slot_id
    }

    async fn seed_transaction(
        &self,
        org_id: &str,
        branch_id: &str,
        customer_id: &str,
        mode: &str,
        value_date: NaiveDate,
    ) -> Uuid {
        let tx_id = Uuid::new_v4();
        let doc_ref = unique("DOC");
        let teller_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, 'Test Teller', 'ACTIVE', 'tester') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(teller_id)
        .bind(org_id)
        .execute(&self.pool)
        .await
        .expect("seed user");

        let sid = unique("SHIFT");
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', now(), 'tester', $1, $3)",
        )
        .bind(&sid)
        .bind(branch_id)
        .bind(teller_id)
        .execute(&self.pool)
        .await
        .expect("seed shift");

        sqlx::query(
            "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
                 customer_id, customer_name, nationality, type, code, amount, rate, principal, fee, \
                 bank_fee, received, paid, method, payment_ref, account, staff_id, booking_no, notes, \
                 value_date, paid_at, recorded_at, recorded_by, status, completed_at) \
             VALUES ($1, $2, $3, $4, $5, $6, 'hash', \
                 $7, 'Customer', 'THA', 'SELL', 'USD', 100, 32.5, 3250, 0, 0, 3250, 0, 'Cash', '', '', 'staff', '', '', \
                 $8, now(), now(), 'staff', 'COMPLETED', now())",
        )
        .bind(tx_id)
        .bind(&doc_ref)
        .bind(branch_id)
        .bind(mode)
        .bind(&sid)
        .bind(unique("sub"))
        .bind(customer_id)
        .bind(value_date)
        .execute(&self.pool)
        .await
        .expect("seed transaction");

        tx_id
    }

    #[allow(clippy::too_many_arguments)]
    async fn seed_booking(
        &self,
        org_id: &str,
        branch_id: &str,
        customer_id: &str,
        slot_id: Uuid,
        currency: &str,
        status: &str,
        transaction_id: Option<Uuid>,
    ) -> String {
        let booking_id = format!("BK-{}", Uuid::new_v4());
        let booking_no = unique("B");
        let qr = Uuid::new_v4().as_u128();
        let qr_bytes = qr.to_be_bytes();

        sqlx::query(
            "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, shop_side, \
                 requested_currency, requested_quantity, source_channel, status, qr_token_hash, confirm_expires_at, \
                 transaction_id, created_by) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6, 'BUY', $7, 100, 'APP', $8, $9, \
                 CASE WHEN $8 = 'PENDING_CUSTOMER_CONFIRMATION' THEN now() + interval '1 hour' ELSE NULL END, \
                 $10, 'tester')",
        )
        .bind(&booking_id)
        .bind(org_id)
        .bind(branch_id)
        .bind(&booking_no)
        .bind(customer_id)
        .bind(slot_id)
        .bind(currency)
        .bind(status)
        .bind(&qr_bytes[..])
        .bind(transaction_id)
        .execute(&self.pool)
        .await
        .expect("seed booking");

        booking_id
    }

    async fn seed_event(
        &self,
        org_id: &str,
        booking_id: &str,
        kind: &str,
        from_status: Option<&str>,
        to_status: &str,
    ) {
        sqlx::query(
            "INSERT INTO booking_events (id, organization_id, booking_id, kind, from_status, to_status, actor_id, at) \
             VALUES (gen_random_uuid(), $1::uuid, $2, $3, $4, $5, 'tester', now())",
        )
        .bind(org_id)
        .bind(booking_id)
        .bind(kind)
        .bind(from_status)
        .bind(to_status)
        .execute(&self.pool)
        .await
        .expect("seed booking event");
    }

    async fn branch_today(&self, branch_id: &str) -> NaiveDate {
        sqlx::query_scalar("SELECT (now() AT TIME ZONE timezone)::date FROM branches WHERE id = $1")
            .bind(branch_id)
            .fetch_one(&self.pool)
            .await
            .expect("branch today")
    }
}

/// The organisation-wide EXECUTIVE of ORG.
struct Executive;

impl SessionReader for Executive {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            Sessions {
                owner: Some(Actor::owner(EXECUTIVE)),
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move {
            let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
            let user = User::new(
                EXECUTIVE,
                ORG,
                vec![ScopedRole::new(Role::Executive, None, from, None)],
                None,
            );
            Callers {
                owner: Some(Actor::owner(EXECUTIVE)),
                staff: None,
                user: Some(user.clone()),
                staff_user: Some(user),
            }
        })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn get(room: &Room, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(Executive);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

#[actix_web::test]
async fn test_executive_overview_view_wired_booking_conversion() {
    let room = Room::start().await;
    room.seed_branch(ORG, B1, "Asia/Bangkok").await;
    room.seed_customer(ORG, C1, "LIVE").await;

    let today = room.branch_today(B1).await;
    let slot_time = NaiveTime::from_hms_opt(10, 0, 0).expect("valid time");
    let slot_id = room.seed_slot(ORG, B1, today, slot_time, 30).await;

    // Seed 1 completed booking that was confirmed
    let tx = room.seed_transaction(ORG, B1, C1, "LIVE", today).await;
    let booking_id = room
        .seed_booking(ORG, B1, C1, slot_id, "THB", "COMPLETED", Some(tx))
        .await;
    room.seed_event(
        ORG,
        &booking_id,
        "CONFIRM",
        Some("PENDING_CUSTOMER_CONFIRMATION"),
        "CONFIRMED",
    )
    .await;
    room.seed_event(
        ORG,
        &booking_id,
        "COMPLETE",
        Some("IN_SERVICE"),
        "COMPLETED",
    )
    .await;

    // 1. Direct verification of ExecutiveOverviewView::new as wired
    let view = ExecutiveOverviewView::new(room.pool.clone());
    let scope = BranchScope::Branches(vec![BranchId::new(B1)]);
    let filter = ExecutiveFilter {
        from: Some(today),
        to: Some(today),
        ..ExecutiveFilter::default()
    };
    let data = view
        .load(&scope, &filter, &[BranchId::new(B1)])
        .await
        .expect("load overview data");

    let metric = data
        .kpis
        .iter()
        .find(|m| m.key == "booking_conversion")
        .expect("booking_conversion metric must be present in kpis");

    assert_ne!(
        metric.unavailable.as_deref(),
        Some("awaitsBookingDesign"),
        "A01 as wired (ExecutiveOverviewView::new) must no longer answer awaitsBookingDesign once a booking cohort exists"
    );
    assert_eq!(
        metric.unavailable, None,
        "bookingConversion must not be unavailable once cohort exists"
    );
    assert_eq!(
        metric.value.as_deref(),
        Some("100%"),
        "bookingConversion must carry calculated conversion percentage"
    );

    // 2. HTTP endpoint verification (GET /api/executive/overview) as wired via bootstrap::configure
    let uri = format!("/api/executive/overview?branches={B1}&from={today}&to={today}");
    let a = get(&room, &uri).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "HTTP status must be 200: {}",
        a.text
    );

    let kpis = a.body["data"]["kpis"]
        .as_array()
        .unwrap_or_else(|| panic!("data.kpis must be array: {}", a.text));
    let http_kpi = kpis
        .iter()
        .find(|k| k["key"] == "booking_conversion")
        .unwrap_or_else(|| panic!("booking_conversion not found in kpis: {}", a.text));

    assert_ne!(
        http_kpi["unavailable"].as_str(),
        Some("awaitsBookingDesign"),
        "HTTP route as wired must not answer awaitsBookingDesign: {}",
        a.text
    );
    assert!(
        http_kpi["unavailable"].is_null(),
        "HTTP bookingConversion unavailable must be null: {}",
        a.text
    );
    assert_eq!(
        http_kpi["value"].as_str(),
        Some("100%"),
        "HTTP bookingConversion must be 100%: {}",
        a.text
    );

    room.finish().await;
}
