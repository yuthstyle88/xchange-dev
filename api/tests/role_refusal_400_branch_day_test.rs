//! #4552 (#4512 B): branchDay role refusals answer 400 (`roleNotAllowed` / `roleNotAllowedHere`),
//! while `outOfScope` stays 403.

mod common;
mod rates_re4c_common;

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_shift::application::branch_day::service::BranchDayService;
use xc_shift::application::shift::service::ShiftService;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::custody::custody_ports::PgCustodyPorts;
use xchange::infrastructure::idempotency::store::IdempotencyKey;
use xchange::presentation::dto::v3::branch_day::{
    AttendanceClockInput, CustomerHoldCreateInput, CustomerHoldReleaseInput, DayCutoffInput,
    DayStartServiceInput, DayTargetInput, DraftBeginInput,
};
use xchange::presentation::http::v3::branch_day::{
    AttendanceClockCommand, CustomerHoldCreateCommand, CustomerHoldReleaseCommand,
    DayCutoffCommand, DayStartServiceCommand, DayTargetCommand, DraftBeginCommand,
};
use xchange::presentation::http::v3::command::CommandAction;
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const MANAGER: &str = "BRANCH_MANAGER@XC-B01";
const TELLER: &str = "TELLER@XC-B01";
const FINANCE: &str = "FINANCE@XC-B01";
const REVIEWER: &str = "VERIFICATION_REVIEWER@XC-B01";
const CUSTOMER: &str = "CU-LIVE-1";
const RATE_VERSION: &str = "2026-10-11T01:02:03.456Z";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn user_id(spec: &str) -> &'static str {
    match spec {
        MANAGER => "00000000-0000-4000-8000-0000000000b1",
        TELLER => "00000000-0000-4000-8000-0000000000b2",
        FINANCE => "00000000-0000-4000-8000-0000000000b4",
        REVIEWER => "00000000-0000-4000-8000-0000000000b5",
        _ => "00000000-0000-4000-8000-0000000000b7",
    }
}

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .expect("TEST_DATABASE_URL or DATABASE_URL must be set");
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let schema = unique("v3bd400");
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
            .max_connections(8)
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
        let db = Self { pool, schema };
        db.exec("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B09', 'B09')")
            .await;
        for spec in [MANAGER, TELLER, FINANCE, REVIEWER] {
            db.exec(&format!(
                "INSERT INTO users (id, organization_id, name, status, created_by) \
                 VALUES ('{}', '{ORG}', '{spec}', 'ACTIVE', 'test')",
                user_id(spec)
            ))
            .await;
        }
        db.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('{CUSTOMER}', '{ORG}', '{CUSTOMER}', 'Somchai', 'LIVE', now())"
        ))
        .await;
        rates_re4c_common::set_counter_rate(&db.pool, B01, "USD", "35.0", "36.0").await;
        db.exec(&format!(
            "UPDATE rate_publications SET published_at = '{RATE_VERSION}' \
             WHERE id = (SELECT current_publication_id FROM branches WHERE id = '{B01}')"
        ))
        .await;
        db
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
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

struct HeaderUsers;

fn user_of(spec: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = match role {
        "BRANCH_MANAGER" => Role::BranchManager,
        "TELLER" => Role::Teller,
        "FINANCE" => Role::Finance,
        "VERIFICATION_REVIEWER" => Role::VerificationReviewer,
        "CUSTOMER" => Role::Customer,
        other => panic!("unknown role {other}"),
    };
    Some(User::new(
        user_id(spec),
        ORG,
        vec![ScopedRole::new(
            role,
            Some(BranchId::new(branch)),
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

async fn post(db: &TestDb, who: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("k")))
        .insert_header((AS, who))
        .set_payload(body.to_string())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let payload = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, payload)
}

#[actix_web::test]
async fn branch_day_role_refusals_answer_400_per_route_and_command() {
    let db = TestDb::start().await;

    let routes: Vec<(&str, &str, &str, Value)> = vec![
        (
            "dayStartService",
            "/api/branch-day/start",
            TELLER,
            json!({
                "branch": B01,
                "equipmentReady": true,
                "ratesVerified": true,
                "rateVersion": RATE_VERSION,
            }),
        ),
        (
            "dayCutoff",
            "/api/branch-day/cutoff",
            TELLER,
            json!({ "branch": B01 }),
        ),
        (
            "dayTarget",
            "/api/branch-day/target",
            MANAGER,
            json!({ "branch": B01, "amounts": { "USD": "1000.00" } }),
        ),
        (
            "clock",
            "/api/attendance",
            "CUSTOMER@XC-B01",
            json!({ "branch": B01, "direction": "IN" }),
        ),
        (
            "holdCreate",
            "/api/customer-holds",
            FINANCE,
            json!({ "branch": B01, "customerId": CUSTOMER, "reason": "passport check" }),
        ),
        (
            "holdRelease",
            "/api/customer-holds/hold-1/release",
            TELLER,
            json!({ "branch": B01, "note": "checked" }),
        ),
        (
            "draftBegin",
            "/api/tx-drafts",
            FINANCE,
            json!({ "branch": B01, "side": "BUY", "currency": "USD", "amount": "150.00" }),
        ),
    ];

    for (name, uri, lacking_role, body) in &routes {
        let (status, payload) = post(&db, lacking_role, uri, body.clone()).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{name} ({uri}) as {lacking_role} must answer 400, got {status}: {payload}"
        );
        assert_eq!(
            payload["error"]["code"],
            "roleNotAllowedHere",
            "{name} ({uri}) error.code: {payload}"
        );
    }

    // Exercise `map_service_error` in `branch_day.rs` across all 7 commands.
    let branch_tx = BranchTx::new(Transactor::new(db.pool.clone()));
    let shifts = ShiftService::new(
        Arc::new(
            xc_cash::infrastructure::ledger::ledger_shift_repository::ledger_shift_repository(
                db.pool.clone(),
            ),
        ),
        branch_tx,
    );
    let repo = Arc::new(PgCustodyRepository::new(db.pool.clone()));
    let ports = Arc::new(PgCustodyPorts);
    let service = Arc::new(BranchDayService::new(repo, shifts, ports.clone()));

    let branch = BranchId::new(B01);
    let teller = user_of(TELLER).expect("teller");
    let manager = user_of(MANAGER).expect("manager");
    let finance = user_of(FINANCE).expect("finance");
    let customer = user_of("CUSTOMER@XC-B01").expect("customer");
    let key = || IdempotencyKey::new(unique("k"));

    let mut tx = db.pool.begin().await.expect("tx");

    let err = DayStartServiceCommand::new(service.clone(), ports)
        .run(
            &mut tx,
            &branch,
            &teller,
            DayStartServiceInput {
                branch: B01.into(),
                equipment_ready: true,
                rates_verified: true,
                rate_version: RATE_VERSION.into(),
            },
            key(),
        )
        .await
        .expect_err("dayStartService as teller");
    assert_eq!(
        (err.status, err.code),
        (StatusCode::BAD_REQUEST, "roleNotAllowed")
    );

    let err = DayCutoffCommand::new(service.clone())
        .run(
            &mut tx,
            &branch,
            &teller,
            DayCutoffInput { branch: B01.into() },
            key(),
        )
        .await
        .expect_err("dayCutoff as teller");
    assert_eq!(
        (err.status, err.code),
        (StatusCode::BAD_REQUEST, "roleNotAllowed")
    );

    let mut amounts = BTreeMap::new();
    amounts.insert("USD".to_string(), "100.00".to_string());
    let err = DayTargetCommand::new(service.clone())
        .run(
            &mut tx,
            &branch,
            &manager,
            DayTargetInput {
                branch: B01.into(),
                amounts,
            },
            key(),
        )
        .await
        .expect_err("dayTarget as manager");
    assert_eq!(
        (err.status, err.code),
        (StatusCode::BAD_REQUEST, "roleNotAllowed")
    );

    let err = AttendanceClockCommand::new(service.clone())
        .run(
            &mut tx,
            &branch,
            &customer,
            AttendanceClockInput {
                branch: B01.into(),
                direction: "IN".into(),
            },
            key(),
        )
        .await
        .expect_err("clock as customer");
    assert_eq!(
        (err.status, err.code),
        (StatusCode::BAD_REQUEST, "roleNotAllowed")
    );

    let err = CustomerHoldCreateCommand::new(service.clone())
        .run(
            &mut tx,
            &branch,
            &finance,
            CustomerHoldCreateInput {
                branch: B01.into(),
                customer_id: CUSTOMER.into(),
                reason: "check".into(),
            },
            key(),
        )
        .await
        .expect_err("holdCreate as finance");
    assert_eq!(
        (err.status, err.code),
        (StatusCode::BAD_REQUEST, "roleNotAllowed")
    );

    let err = CustomerHoldReleaseCommand::new(service.clone())
        .run(
            &mut tx,
            &branch,
            &teller,
            CustomerHoldReleaseInput {
                id: "hold-1".into(),
                branch: B01.into(),
                note: "checked".into(),
            },
            key(),
        )
        .await
        .expect_err("holdRelease as teller");
    assert_eq!(
        (err.status, err.code),
        (StatusCode::BAD_REQUEST, "roleNotAllowed")
    );

    let err = DraftBeginCommand::new(service.clone())
        .run(
            &mut tx,
            &branch,
            &finance,
            DraftBeginInput {
                branch: B01.into(),
                side: "BUY".into(),
                currency: "USD".into(),
                amount: "100.00".into(),
            },
            key(),
        )
        .await
        .expect_err("draftBegin as finance");
    assert_eq!(
        (err.status, err.code),
        (StatusCode::BAD_REQUEST, "roleNotAllowed")
    );

    drop(tx);
    db.finish().await;
}

#[actix_web::test]
async fn branch_day_out_of_scope_answers_403() {
    let db = TestDb::start().await;

    let (status, payload) = post(
        &db,
        "BRANCH_MANAGER@XC-B09",
        "/api/branch-day/cutoff",
        json!({ "branch": B01 }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "dayCutoff outOfScope must answer 403, got {status}: {payload}"
    );
    assert_eq!(
        payload["error"]["code"],
        "outOfScope",
        "dayCutoff error.code: {payload}"
    );

    let branch_tx = BranchTx::new(Transactor::new(db.pool.clone()));
    let shifts = ShiftService::new(
        Arc::new(
            xc_cash::infrastructure::ledger::ledger_shift_repository::ledger_shift_repository(
                db.pool.clone(),
            ),
        ),
        branch_tx,
    );
    let repo = Arc::new(PgCustodyRepository::new(db.pool.clone()));
    let ports = Arc::new(PgCustodyPorts);
    let service = Arc::new(BranchDayService::new(repo, shifts, ports));
    let branch = BranchId::new(B01);
    let other_finance = user_of("FINANCE@XC-B09").expect("finance@XC-B09");
    let mut tx = db.pool.begin().await.expect("tx");
    let mut amounts = BTreeMap::new();
    amounts.insert("USD".to_string(), "100.00".to_string());
    let err = DayTargetCommand::new(service)
        .run(
            &mut tx,
            &branch,
            &other_finance,
            DayTargetInput {
                branch: B01.into(),
                amounts,
            },
            IdempotencyKey::new(unique("k")),
        )
        .await
        .expect_err("dayTarget as finance@XC-B09");
    assert_eq!((err.status, err.code), (StatusCode::FORBIDDEN, "outOfScope"));
    drop(tx);

    db.finish().await;
}
