//! RA-3 (#2026, design `docs/design/blueprint/broker-seam.md` §4.3): pins the branch check in
//! `supersedeReferral` (`api/src/presentation/http/v3/commands/supersede_referral.rs:64`).
//!
//! A supersede must name the capture's own branch; naming another branch answers 403 `outOfScope`
//! without mutating `referral_attributions` or writing an audit event.
//! Needs `TEST_DATABASE_URL`; nothing skips.

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
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const TELLER_B09: &str = "TELLER@XC-B09";
const EXECUTIVE: &str = "EXECUTIVE@*";

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

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("refsupersedebranch");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B09', 'B09')")
            .execute(&pool)
            .await
            .expect("insert the branches");
        Self { pool, schema }
    }

    /// A LIVE customer and a DEMO exchange of theirs at `branch` (a DEMO exchange needs no shift);
    /// answers the exchange's id.
    async fn exchange(&self, branch: &str, customer: &str) -> String {
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT $1, b.organization_id, $1, $1, 'LIVE', now() FROM branches b WHERE b.id = $2 \
             ON CONFLICT DO NOTHING",
        )
        .bind(customer)
        .bind(branch)
        .execute(&self.pool)
        .await
        .expect("insert a customer");
        let document_ref = unique("DOC");
        sqlx::query_scalar(
            "INSERT INTO transactions (document_ref, branch_id, mode, submission_id, payload_hash, customer_id, customer_name, \
                 nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, recorded_by, \
                 status, completed_at) \
             VALUES ($1, $2, 'DEMO', $3, 'payload-hash', $4, 'Customer', 'THA', 'owner', '', '', \
                 DATE '2026-10-06', now(), now(), 'owner', 'COMPLETED', now()) RETURNING id::text",
        )
        .bind(&document_ref)
        .bind(branch)
        .bind(format!("submission-of-{document_ref}"))
        .bind(customer)
        .fetch_one(&self.pool)
        .await
        .expect("insert an exchange")
    }

    /// `(status, code, captured_by, source)` of a capture.
    async fn capture(&self, id: &str) -> (String, String, String, String) {
        sqlx::query_as(
            "SELECT status, code, captured_by, source FROM referral_attributions WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("the capture row")
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
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

// ------------------------------------------------------------ callers

/// `x-test-as: <ROLE>@<branch>` (or `@*` for organisation-wide): one Blueprint role row.
struct HeaderUsers;

fn user_id(spec: &str) -> String {
    format!("user-{}", spec.replace(['@', '*'], "-"))
}

fn user_of(spec: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = match role {
        "FINANCE" => Role::Finance,
        "ADMIN" => Role::Admin,
        "EXECUTIVE" => Role::Executive,
        "BRANCH_MANAGER" => Role::BranchManager,
        "TELLER" => Role::Teller,
        "MARKETING" => Role::Marketing,
        other => panic!("unknown role {other}"),
    };
    let branch = (branch != "*").then(|| BranchId::new(branch));
    Some(User::new(
        &user_id(spec),
        "XC",
        vec![ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
}

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(db: &TestDb, who: &str, method: Method, uri: &str, body: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn attach(db: &TestDb, who: &str, branch: &str, tx: &str, code: &str) -> Answer {
    call(
        db,
        who,
        Method::POST,
        &format!("/api/transactions/{tx}/referral"),
        json!({ "branch": branch, "referralCode": code }),
    )
    .await
}

async fn supersede(
    db: &TestDb,
    who: &str,
    branch: &str,
    capture: &str,
    code: &str,
    reason: &str,
) -> Answer {
    call(
        db,
        who,
        Method::POST,
        &format!("/api/referral-attributions/{capture}/supersede"),
        json!({ "branch": branch, "referralCode": code, "reason": reason }),
    )
    .await
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn capture_id(a: &Answer) -> String {
    a.body["captureId"]
        .as_str()
        .unwrap_or_else(|| panic!("a captureId: {}", a.text))
        .to_owned()
}

// ------------------------------------------------------------ #2026 behaviour table

/// Row 1: capture on a B09 exchange (attached as `TELLER@XC-B09`); supersede as `EXECUTIVE@*`
/// naming B01 -> 403 `outOfScope`; the old capture stays `ACTIVE`, there is still one
/// `referral_attributions` row, and there is no `supersedeReferral` audit row.
#[actix_web::test]
async fn row_1_b09_capture_superseded_as_executive_naming_b01_is_403_out_of_scope() {
    let db = TestDb::start().await;
    let tx = db.exchange(B09, "C-1").await;
    let attached = attach(&db, TELLER_B09, B09, &tx, "AB12CD").await;
    assert_eq!(attached.status, StatusCode::OK, "{}", attached.text);
    let old = capture_id(&attached);

    let refused = supersede(
        &db,
        EXECUTIVE,
        B01,
        &old,
        "ZZ99YY",
        "the customer named another broker",
    )
    .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    assert_eq!(code(&refused), "outOfScope", "{}", refused.text);

    let (status, c, by, source) = db.capture(&old).await;
    assert_eq!(
        (status.as_str(), c.as_str(), by.as_str(), source.as_str()),
        ("ACTIVE", "AB12CD", user_id(TELLER_B09).as_str(), "DESK"),
        "the old capture stays ACTIVE and untouched"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM referral_attributions").await,
        1,
        "still one referral_attributions row"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM audit_events WHERE action = 'supersedeReferral'")
            .await,
        0,
        "no supersedeReferral audit row"
    );
    db.finish().await;
}

/// Row 2: the same capture (after the B01 refusal), `EXECUTIVE` naming B09 -> 200 (control).
#[actix_web::test]
async fn row_2_same_b09_capture_superseded_as_executive_naming_b09_is_200_control() {
    let db = TestDb::start().await;
    let tx = db.exchange(B09, "C-1").await;
    let old = capture_id(&attach(&db, TELLER_B09, B09, &tx, "AB12CD").await);

    let _ = supersede(&db, EXECUTIVE, B01, &old, "ZZ99YY", "wrong branch attempt").await;

    let ok = supersede(
        &db,
        EXECUTIVE,
        B09,
        &old,
        "ZZ99YY",
        "the customer named another broker",
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    let new = capture_id(&ok);
    assert_ne!(new, old);
    assert_eq!(db.capture(&old).await.0, "SUPERSEDED");
    let (status, c, by, _) = db.capture(&new).await;
    assert_eq!(
        (status.as_str(), c.as_str(), by.as_str()),
        ("ACTIVE", "ZZ99YY", user_id(EXECUTIVE).as_str())
    );
    db.finish().await;
}

/// Row 3: unknown capture uuid -> 409 `staleVersion` (`:52-54`: unknown -> `CaptureError::NotActive`).
#[actix_web::test]
async fn row_3_unknown_capture_uuid_answers_409_stale_version() {
    let db = TestDb::start().await;
    for unknown in ["00000000-0000-0000-0000-000000000000", "not-a-uuid"] {
        let a = supersede(&db, EXECUTIVE, B01, unknown, "ZZ99YY", "reason").await;
        assert_eq!(a.status, StatusCode::CONFLICT, "{unknown}: {}", a.text);
        assert_eq!(code(&a), "staleVersion", "{unknown}: {}", a.text);
    }
    db.finish().await;
}
