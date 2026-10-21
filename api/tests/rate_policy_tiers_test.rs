// Written before the implementation by the test writer gemini/studio-9 (controller claude/553782). Do not edit, delete or weaken.
//! Up-front tests for VT-4 (xchange#1648):
//! Value tiers switch (`valueTiersOn`), four tier rows (`valueTiers`), and sharp-move threshold
//! (`sharpMoveAlertPct`) on `POST /api/rate-policies` (`ratePolicySave`) and
//! `GET /api/v3/views/rate_policy` view.
//!
//! Behaviour table from ticket §9:
//! 1. EXECUTIVE saves ON with 4 rising bands, margins within guard:
//!    saved; view returns valueTiersOn = true and the 4 rows.
//! 2. ON with 3 rows, or bands not rising, or tier 4 with an upper:
//!    validationFailed, nothing written.
//! 3. ON where adjust + margin of one tier/side lies outside the guard:
//!    validationFailed.
//! 4. sharpMoveAlertPct = "2.5" -> reads back "2.5000"; null reads back null; "0" or negative:
//!    saved / saved / validationFailed.
//! 5. RATE_MANAGER (no EXECUTIVE row) changes the switch, a tier row or the threshold:
//!    RoleNotAllowedHere 400.
//! 6. RATE_MANAGER changes only buyAdjustPct, tiers untouched:
//!    saved (today's rule kept).
//! 7. a version saved OFF with 4 rows:
//!    rows kept; view returns valueTiersOn = false and the rows.
//! 8. an old version (before VT-1):
//!    view returns valueTiersOn = false, valueTiers = [], sharpMoveAlertPct = null.

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
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::Role;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const OWNER: &str = "owner";
const RATES: &str = "staff:RATES:XC-B01";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

// ------------------------------------------------------------ database

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The policy tiers route tests need a real PostgreSQL and do not skip. Point it at \
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
        let schema = unique("v3ratepolicytiers");
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
            .expect("insert test branch into branches");
        Self { pool, schema }
    }

    /// Every policy version of `branch`, oldest first, each as its row's JSON text.
    async fn versions_of(&self, branch: &str) -> Vec<Value> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT row_to_json(p)::text FROM rate_policy_versions p \
             WHERE p.branch_id = $1 ORDER BY p.version",
        )
        .bind(branch)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("read the policy versions of {branch}: {e}"));
        rows.iter()
            .map(|r| serde_json::from_str(r).expect("row_to_json is JSON"))
            .collect()
    }

    /// Drop the schema. Reached only by a test that passed.
    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ callers

/// Who is signed in, from the request header `x-test-as`: `owner` or `staff:<ROLE>:<branch>`.
struct HeaderCallers;

fn staff_roles(role: &str) -> &'static [Role] {
    match role {
        "FRONT" => &[Role::Teller],
        "BACKEND" | "MANAGER" => &[Role::BranchManager],
        "RATES" => &[Role::RateManager],
        "FINANCE" => &[Role::Finance],
        "SYSTEM" => &[Role::Admin],
        "MONITOR" | "DOCUMENTS" | "DOCUMENT_REVIEW" => &[Role::VerificationReviewer],
        other => panic!("unknown staff role {other:?}"),
    }
}

fn callers_of(spec: &str) -> Callers {
    if spec == OWNER {
        return test_callers::callers(Some(test_callers::owner("owner-1")), None);
    }
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = parts.next().expect("a role");
    let branch = parts.next().expect("a branch");
    test_callers::callers(
        None,
        Some(test_callers::staff(
            &format!("employee-{role}"),
            staff_roles(role),
            BranchId::new(branch),
        )),
    )
}

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { callers_of(&spec) })
    }
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(db: &TestDb, who: &str, method: Method, uri: &str, body: Option<Value>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who));
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", unique("key")))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// `POST /api/rate-policies` as `who`, naming `branch`.
async fn save(db: &TestDb, who: &str, branch: &str, body: Value) -> Answer {
    let mut body = body;
    body["branch"] = json!(branch);
    call(db, who, Method::POST, "/api/rate-policies", Some(body)).await
}

/// The `rate_policy` view's `policy` as `who` sees it on `branch`.
async fn current_policy(db: &TestDb, who: &str, branch: &str) -> Value {
    let a = call(
        db,
        who,
        Method::GET,
        &format!("/api/v3/views/rate_policy?branch={branch}"),
        None,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "read rate_policy: {}", a.text);
    a.body["data"]["policy"].clone()
}

fn assert_refused_as_outside_the_role(a: &Answer, what: &str) {
    let err = PolicyError::RoleNotAllowedHere;
    assert_eq!(
        a.status.as_u16(),
        err.status(),
        "{what}: expected 400 RoleNotAllowedHere; body {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["code"],
        json!(V3Error::from(err).code),
        "{what}: expected error code roleNotAllowedHere; body {}",
        a.text
    );
}

// ------------------------------------------------------------ fixtures

fn frame_guards() -> Value {
    json!({
        "USD": {
            "buy": { "min_pct": "-1.0000", "max_pct": "1.0000" },
            "sell": { "min_pct": "-1.0000", "max_pct": "1.0000" }
        },
        "JPY": {
            "buy": { "min_pct": "-2.0000", "max_pct": "0.5000" },
            "sell": { "min_pct": "-0.5000", "max_pct": "2.0000" }
        }
    })
}

fn valid_four_tiers() -> Value {
    json!([
        {
            "tierNo": 1,
            "upperInclusiveThb": "20000.00",
            "buyMarginPct": "0.0000",
            "sellMarginPct": "0.0000"
        },
        {
            "tierNo": 2,
            "upperInclusiveThb": "100000.00",
            "buyMarginPct": "0.1000",
            "sellMarginPct": "-0.1000"
        },
        {
            "tierNo": 3,
            "upperInclusiveThb": "500000.00",
            "buyMarginPct": "0.2000",
            "sellMarginPct": "-0.2000"
        },
        {
            "tierNo": 4,
            "upperInclusiveThb": null,
            "buyMarginPct": "0.3000",
            "sellMarginPct": "-0.3000"
        }
    ])
}

fn base_frame() -> Value {
    json!({
        "buyAdjustPct": "0",
        "sellAdjustPct": "0",
        "guards": frame_guards(),
        "maxReferenceAgeHours": 24,
        "onStale": "LIMIT",
        "staleLimitThb": "5000.00",
        "reason": "base frame",
    })
}

// ------------------------------------------------------------ tests (§9 rows)

/// Row 1: EXECUTIVE saves ON with 4 rising bands, margins within guard
/// Result: saved; view returns `valueTiersOn = true` and the 4 rows.
#[actix_web::test]
async fn row1_executive_saves_on_with_4_rising_bands() {
    let db = TestDb::start().await;
    let mut frame = base_frame();
    frame["valueTiersOn"] = json!(true);
    frame["valueTiers"] = valid_four_tiers();
    frame["reason"] = json!("executive turns value tiers on");

    let a = save(&db, OWNER, B01, frame).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "EXECUTIVE save with 4 rising tiers: {}",
        a.text
    );
    assert_eq!(a.body["version"], json!(1), "first version: {}", a.text);

    let policy = current_policy(&db, OWNER, B01).await;
    assert_eq!(
        policy["valueTiersOn"],
        json!(true),
        "view must return valueTiersOn = true: {policy}"
    );

    let tiers = policy["valueTiers"]
        .as_array()
        .expect("view must return valueTiers array");
    assert_eq!(tiers.len(), 4, "view must return exactly 4 tier rows");

    assert_eq!(tiers[0]["tierNo"], json!(1));
    assert_eq!(tiers[0]["upperInclusiveThb"], json!("20000.00"));
    assert_eq!(tiers[0]["buyMarginPct"], json!("0.0000"));
    assert_eq!(tiers[0]["sellMarginPct"], json!("0.0000"));

    assert_eq!(tiers[1]["tierNo"], json!(2));
    assert_eq!(tiers[1]["upperInclusiveThb"], json!("100000.00"));
    assert_eq!(tiers[1]["buyMarginPct"], json!("0.1000"));
    assert_eq!(tiers[1]["sellMarginPct"], json!("-0.1000"));

    assert_eq!(tiers[2]["tierNo"], json!(3));
    assert_eq!(tiers[2]["upperInclusiveThb"], json!("500000.00"));
    assert_eq!(tiers[2]["buyMarginPct"], json!("0.2000"));
    assert_eq!(tiers[2]["sellMarginPct"], json!("-0.2000"));

    assert_eq!(tiers[3]["tierNo"], json!(4));
    assert_eq!(tiers[3]["upperInclusiveThb"], Value::Null);
    assert_eq!(tiers[3]["buyMarginPct"], json!("0.3000"));
    assert_eq!(tiers[3]["sellMarginPct"], json!("-0.3000"));

    db.finish().await;
}

/// Row 2: ON with 3 rows, or bands not rising, or tier 4 with an upper
/// Result: `validationFailed`, nothing written.
#[actix_web::test]
async fn row2_refuses_invalid_tier_counts_or_bands() {
    let db = TestDb::start().await;

    // Subcase A: 3 rows instead of 4
    let three_tiers = json!([
        { "tierNo": 1, "upperInclusiveThb": "20000.00", "buyMarginPct": "0.0000", "sellMarginPct": "0.0000" },
        { "tierNo": 2, "upperInclusiveThb": "100000.00", "buyMarginPct": "0.1000", "sellMarginPct": "-0.1000" },
        { "tierNo": 3, "upperInclusiveThb": null, "buyMarginPct": "0.2000", "sellMarginPct": "-0.2000" }
    ]);

    // Subcase B: bands not rising (tier 2 upper <= tier 1 upper)
    let not_rising_tiers = json!([
        { "tierNo": 1, "upperInclusiveThb": "20000.00", "buyMarginPct": "0.0000", "sellMarginPct": "0.0000" },
        { "tierNo": 2, "upperInclusiveThb": "15000.00", "buyMarginPct": "0.1000", "sellMarginPct": "-0.1000" },
        { "tierNo": 3, "upperInclusiveThb": "500000.00", "buyMarginPct": "0.2000", "sellMarginPct": "-0.2000" },
        { "tierNo": 4, "upperInclusiveThb": null, "buyMarginPct": "0.3000", "sellMarginPct": "-0.3000" }
    ]);

    // Subcase C: tier 4 with an upper bound
    let tier4_with_upper = json!([
        { "tierNo": 1, "upperInclusiveThb": "20000.00", "buyMarginPct": "0.0000", "sellMarginPct": "0.0000" },
        { "tierNo": 2, "upperInclusiveThb": "100000.00", "buyMarginPct": "0.1000", "sellMarginPct": "-0.1000" },
        { "tierNo": 3, "upperInclusiveThb": "500000.00", "buyMarginPct": "0.2000", "sellMarginPct": "-0.2000" },
        { "tierNo": 4, "upperInclusiveThb": "1000000.00", "buyMarginPct": "0.3000", "sellMarginPct": "-0.3000" }
    ]);

    let cases = [
        ("3 rows", three_tiers),
        ("bands not rising", not_rising_tiers),
        ("tier 4 with upper bound", tier4_with_upper),
    ];

    for (desc, invalid_tiers) in cases {
        let mut frame = base_frame();
        frame["valueTiersOn"] = json!(true);
        frame["valueTiers"] = invalid_tiers;
        frame["reason"] = json!(format!("invalid tier test: {desc}"));

        let a = save(&db, OWNER, B01, frame).await;
        assert_eq!(
            a.status,
            StatusCode::BAD_REQUEST,
            "{desc}: expected 400 validationFailed, got {}: {}",
            a.status,
            a.text
        );
        assert_eq!(
            a.body["error"]["code"],
            json!("validationFailed"),
            "{desc}: error code must be validationFailed: {}",
            a.text
        );
        assert_eq!(
            a.body["error"]["field"],
            json!("valueTiers"),
            "{desc}: error field must be valueTiers: {}",
            a.text
        );
    }

    assert!(
        db.versions_of(B01).await.is_empty(),
        "nothing written on invalid tier validation failure"
    );

    db.finish().await;
}

/// Row 3: ON where `adjust + margin` of one tier/side lies outside the guard
/// Result: `validationFailed`, nothing written.
#[actix_web::test]
async fn row3_refuses_tier_margin_outside_guard() {
    let db = TestDb::start().await;

    // USD buy guard is [-1.0000, 1.0000].
    // buyAdjustPct = "0.0000", buyMarginPct = "1.5000" -> adjust + margin = 1.5000 > 1.0000.
    let mut out_of_guard_tiers = valid_four_tiers();
    out_of_guard_tiers[1]["buyMarginPct"] = json!("1.5000");

    let mut frame = base_frame();
    frame["valueTiersOn"] = json!(true);
    frame["valueTiers"] = out_of_guard_tiers;
    frame["reason"] = json!("tier margin outside guard");

    let a = save(&db, OWNER, B01, frame).await;
    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "expected 400 validationFailed when tier outside guard, got {}: {}",
        a.status,
        a.text
    );
    assert_eq!(
        a.body["error"]["code"],
        json!("validationFailed"),
        "error code must be validationFailed: {}",
        a.text
    );

    assert!(
        db.versions_of(B01).await.is_empty(),
        "nothing written when tier margin is outside guard"
    );

    db.finish().await;
}

/// Row 4: `sharpMoveAlertPct` = `"2.5"` -> reads back `"2.5000"`; `null` reads back `null`; `"0"` or negative -> `validationFailed`
/// Result: saved / saved / `validationFailed`.
#[actix_web::test]
async fn row4_sharp_move_threshold() {
    let db = TestDb::start().await;

    // Subcase 1: "2.5" -> saved, reads back "2.5000"
    let mut frame1 = base_frame();
    frame1["sharpMoveAlertPct"] = json!("2.5");
    frame1["reason"] = json!("set sharp move threshold to 2.5");
    let a1 = save(&db, OWNER, B01, frame1).await;
    assert_eq!(
        a1.status,
        StatusCode::OK,
        "save sharpMoveAlertPct 2.5: {}",
        a1.text
    );
    let p1 = current_policy(&db, OWNER, B01).await;
    assert_eq!(
        p1["sharpMoveAlertPct"],
        json!("2.5000"),
        "sharpMoveAlertPct '2.5' must read back formatted as '2.5000': {p1}"
    );

    // Subcase 2: null -> saved, reads back null
    let mut frame2 = base_frame();
    frame2["sharpMoveAlertPct"] = Value::Null;
    frame2["reason"] = json!("clear sharp move threshold");
    let a2 = save(&db, OWNER, B01, frame2).await;
    assert_eq!(
        a2.status,
        StatusCode::OK,
        "save sharpMoveAlertPct null: {}",
        a2.text
    );
    let p2 = current_policy(&db, OWNER, B01).await;
    assert_eq!(
        p2["sharpMoveAlertPct"],
        Value::Null,
        "sharpMoveAlertPct null must read back as null: {p2}"
    );

    let versions_before = db.versions_of(B01).await;

    // Subcase 3: "0" -> validationFailed
    let mut frame_zero = base_frame();
    frame_zero["sharpMoveAlertPct"] = json!("0");
    frame_zero["reason"] = json!("zero sharp move threshold");
    let a_zero = save(&db, OWNER, B01, frame_zero).await;
    assert_eq!(
        a_zero.status,
        StatusCode::BAD_REQUEST,
        "expected 400 for sharpMoveAlertPct = 0: {}",
        a_zero.text
    );
    assert_eq!(
        a_zero.body["error"]["code"],
        json!("validationFailed"),
        "error code must be validationFailed: {}",
        a_zero.text
    );

    // Subcase 4: negative -> validationFailed
    let mut frame_neg = base_frame();
    frame_neg["sharpMoveAlertPct"] = json!("-1.5");
    frame_neg["reason"] = json!("negative sharp move threshold");
    let a_neg = save(&db, OWNER, B01, frame_neg).await;
    assert_eq!(
        a_neg.status,
        StatusCode::BAD_REQUEST,
        "expected 400 for negative sharpMoveAlertPct: {}",
        a_neg.text
    );
    assert_eq!(
        a_neg.body["error"]["code"],
        json!("validationFailed"),
        "error code must be validationFailed: {}",
        a_neg.text
    );

    assert_eq!(
        db.versions_of(B01).await,
        versions_before,
        "invalid threshold values must write nothing"
    );

    db.finish().await;
}

/// Row 5: RATE_MANAGER (no EXECUTIVE row) changes the switch, a tier row or the threshold
/// Result: `RoleNotAllowedHere` 400.
#[actix_web::test]
async fn row5_rate_manager_cannot_change_switch_tier_threshold() {
    let db = TestDb::start().await;

    // First, EXECUTIVE enters frame with valueTiersOn = true, 4 tiers, sharpMoveAlertPct = "2.5000"
    let mut frame = base_frame();
    frame["valueTiersOn"] = json!(true);
    frame["valueTiers"] = valid_four_tiers();
    frame["sharpMoveAlertPct"] = json!("2.5000");
    frame["reason"] = json!("executive sets frame");
    let init = save(&db, OWNER, B01, frame).await;
    assert_eq!(
        init.status,
        StatusCode::OK,
        "EXECUTIVE setup: {}",
        init.text
    );

    let current = current_policy(&db, RATES, B01).await;
    let before = db.versions_of(B01).await;

    // Subcase A: RATE_MANAGER changes switch (true -> false)
    let mut change_switch = current.clone();
    change_switch["valueTiersOn"] = json!(false);
    change_switch["reason"] = json!("rates changes switch");
    let a1 = save(&db, RATES, B01, change_switch).await;
    assert_refused_as_outside_the_role(&a1, "RATE_MANAGER changes switch");
    assert_eq!(
        db.versions_of(B01).await,
        before,
        "no version written when switch changed by RATE_MANAGER"
    );

    // Subcase B: RATE_MANAGER changes a tier row (modifies buyMarginPct)
    let mut change_tier = current.clone();
    let mut modified_tiers = valid_four_tiers();
    modified_tiers[0]["buyMarginPct"] = json!("0.0500");
    change_tier["valueTiers"] = modified_tiers;
    change_tier["reason"] = json!("rates changes tier");
    let a2 = save(&db, RATES, B01, change_tier).await;
    assert_refused_as_outside_the_role(&a2, "RATE_MANAGER changes tier row");
    assert_eq!(
        db.versions_of(B01).await,
        before,
        "no version written when tier row changed by RATE_MANAGER"
    );

    // Subcase C: RATE_MANAGER changes sharp-move threshold
    let mut change_threshold = current.clone();
    change_threshold["sharpMoveAlertPct"] = json!("3.0000");
    change_threshold["reason"] = json!("rates changes threshold");
    let a3 = save(&db, RATES, B01, change_threshold).await;
    assert_refused_as_outside_the_role(&a3, "RATE_MANAGER changes threshold");
    assert_eq!(
        db.versions_of(B01).await,
        before,
        "no version written when threshold changed by RATE_MANAGER"
    );

    db.finish().await;
}

/// Row 6: RATE_MANAGER changes only `buyAdjustPct`, tiers untouched
/// Result: saved (today's rule kept).
#[actix_web::test]
async fn row6_rate_manager_may_change_buy_adjust_pct() {
    let db = TestDb::start().await;

    // First, EXECUTIVE enters frame with valueTiersOn = true, 4 tiers, sharpMoveAlertPct = "2.5000"
    let mut frame = base_frame();
    frame["valueTiersOn"] = json!(true);
    frame["valueTiers"] = valid_four_tiers();
    frame["sharpMoveAlertPct"] = json!("2.5000");
    frame["reason"] = json!("executive sets frame");
    let init = save(&db, OWNER, B01, frame).await;
    assert_eq!(
        init.status,
        StatusCode::OK,
        "EXECUTIVE setup: {}",
        init.text
    );

    let current = current_policy(&db, RATES, B01).await;

    // RATE_MANAGER moves only buyAdjustPct
    let mut rates_body = current.clone();
    rates_body["buyAdjustPct"] = json!("-0.2500");
    rates_body["reason"] = json!("rates moves buy adjustment");

    let a = save(&db, RATES, B01, rates_body).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "RATE_MANAGER changing only buyAdjustPct must succeed: {}",
        a.text
    );
    assert_eq!(a.body["version"], json!(2), "version 2: {}", a.text);

    let seen = current_policy(&db, RATES, B01).await;
    assert_eq!(seen["version"], json!(2));
    assert_eq!(seen["buyAdjustPct"], json!("-0.2500"));
    assert_eq!(
        seen["valueTiersOn"],
        json!(true),
        "valueTiersOn preserved across RATE_MANAGER save: {seen}"
    );
    assert_eq!(
        seen["sharpMoveAlertPct"],
        json!("2.5000"),
        "sharpMoveAlertPct preserved across RATE_MANAGER save: {seen}"
    );

    let tiers = seen["valueTiers"]
        .as_array()
        .expect("valueTiers array preserved");
    assert_eq!(tiers.len(), 4);
    assert_eq!(tiers[0]["upperInclusiveThb"], json!("20000.00"));

    db.finish().await;
}

/// Row 7: a version saved OFF with 4 rows
/// Result: rows kept; view returns `valueTiersOn = false` and the rows.
#[actix_web::test]
async fn row7_off_version_keeps_its_rows() {
    let db = TestDb::start().await;

    let mut frame = base_frame();
    frame["valueTiersOn"] = json!(false);
    frame["valueTiers"] = valid_four_tiers();
    frame["reason"] = json!("executive saves with tiers off but keeps rows");

    let a = save(&db, OWNER, B01, frame).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "save version OFF with 4 rows: {}",
        a.text
    );

    let policy = current_policy(&db, OWNER, B01).await;
    assert_eq!(
        policy["valueTiersOn"],
        json!(false),
        "view must return valueTiersOn = false: {policy}"
    );

    let tiers = policy["valueTiers"]
        .as_array()
        .expect("view must return valueTiers array even when OFF");
    assert_eq!(tiers.len(), 4, "rows kept when valueTiersOn is false");
    assert_eq!(tiers[0]["tierNo"], json!(1));
    assert_eq!(tiers[0]["upperInclusiveThb"], json!("20000.00"));
    assert_eq!(tiers[3]["tierNo"], json!(4));
    assert_eq!(tiers[3]["upperInclusiveThb"], Value::Null);

    db.finish().await;
}

/// Row 8: an old version (before VT-1)
/// Result: view returns `valueTiersOn = false`, `valueTiers = []`, `sharpMoveAlertPct = null`.
#[actix_web::test]
async fn row8_old_version_reads_defaults() {
    let db = TestDb::start().await;

    // Directly seed an old version into rate_policy_versions without any rows in rate_policy_value_tiers
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO rate_policy_versions \
         (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
          max_reference_age_hours, on_stale, stale_limit_thb, valid_from, \
          approved_by, reason, source_version, value_tiers_on, sharp_move_alert_pct) \
         VALUES ($1, $2, 1, 0, 0, $3, 24, 'LIMIT', 5000.00, now(), 'owner-1', 'old version seed', NULL, false, NULL)",
    )
    .bind(id)
    .bind(B01)
    .bind(frame_guards())
    .execute(&db.pool)
    .await
    .expect("seed old version directly in database");

    let policy = current_policy(&db, OWNER, B01).await;
    assert_eq!(policy["version"], json!(1));
    assert_eq!(
        policy["valueTiersOn"],
        json!(false),
        "old version must read valueTiersOn = false: {policy}"
    );
    assert_eq!(
        policy["valueTiers"],
        json!([]),
        "old version must read valueTiers = []: {policy}"
    );
    assert_eq!(
        policy["sharpMoveAlertPct"],
        Value::Null,
        "old version must read sharpMoveAlertPct = null: {policy}"
    );

    db.finish().await;
}
