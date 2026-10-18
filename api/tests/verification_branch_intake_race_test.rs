//! #2876: two branch intakes of one customer's open online case, at two branches at the same moment.
//!
//! **The bug** (main `b0afbb56`): `BranchIntake::open` (`crates/xc-verification/src/application/branch_intake.rs`)
//! read the open case with a plain `SELECT` and decided #2705's V3 rule ("an open case already filed at another
//! branch answers `caseLocked`") on that unlocked read. Two intakes at B01 and B02 both read `branch_id` NULL, both
//! submitted the case, the later UPDATE set its own branch, and the case kept both branches' documents.
//!
//! **The test** makes the race deterministic: it locks the case row itself, starts both intakes, waits until both
//! waiting behind its lock (the `pg_blocking_pids` chain), then releases it. Exactly one intake must win.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`. Each test works in a schema of
//! its own.

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use uuid::Uuid;
use xc_auth::domain::access::actor::Actor as AccessActor;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::AppState;
use xchange::presentation::http::branch_verification_intake::ROUTE;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

mod common;

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const B01: &str = "B01";
const B02: &str = "B02";
const HOST: &str = "desk.example";
const ORIGIN: &str = "https://desk.example";
const AS: &str = "x-test-signed-in-as";
const MARKER: &[u8] = b"VF12-HTTP-SECRET-BYTES-3b8c";
const MUST_FINISH: Duration = Duration::from_secs(60);

static NEXT: AtomicU64 = AtomicU64::new(0);
static STARTING: Semaphore = Semaphore::const_new(4);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn png() -> Vec<u8> {
    let mut b = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    b.extend_from_slice(MARKER);
    b.extend_from_slice(&unique("png").into_bytes());
    b
}

// ── SessionReader mock ──────────────────────────────────────────────────────

struct HeaderUsers;

fn user_of(spec: &str) -> Option<AccessActor> {
    if spec.is_empty() {
        return None;
    }
    let mut parts = spec.splitn(4, '|');
    let id = parts.next()?;
    let role = parts.next()?;
    let branch = parts.next()?;
    let org = parts.next().unwrap_or(ORG_1);

    let role = Role::parse(role)?;
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(AccessActor::new(
        id,
        org,
        vec![ScopedRole::new(role, branch, from, None)],
        None,
    ))
}

impl SessionReader for HeaderUsers {
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
            .unwrap_or("")
            .to_owned();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ── multipart helper ────────────────────────────────────────────────────────

fn build_multipart(
    boundary: &str,
    customer_id: Option<&str>,
    document_type: Option<&str>,
    file: Option<(&str, &str, &[u8])>,
) -> Vec<u8> {
    let mut body = Vec::new();
    if let Some(c) = customer_id {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"customerId\"\r\n\r\n{c}\r\n"
            )
            .as_bytes(),
        );
    }
    if let Some(d) = document_type {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"documentType\"\r\n\r\n{d}\r\n"
            )
            .as_bytes(),
        );
    }
    if let Some((field_name, filename, bytes)) = file {
        let mime = if filename.ends_with(".png") {
            "image/png"
        } else if filename.ends_with(".pdf") {
            "application/pdf"
        } else {
            "image/jpeg"
        };
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{field_name}\"; filename=\"{filename}\"\r\nContent-Type: {mime}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

// ── database & test harness ─────────────────────────────────────────────────

struct HttpAnswer {
    status: StatusCode,
    text: String,
    json: Value,
}

struct TestEnv {
    pool: PgPool,
    schema: String,
}

impl TestEnv {
    async fn start() -> Self {
        common::install_test_pii_keys();
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set. Point it at a test database ending in `_test`")
        });
        let options =
            PgConnectOptions::from_str(&url).expect("TEST_DATABASE_URL is a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("vf2705");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to db");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to schema");

        {
            let _slot = STARTING.acquire().await.expect("slot");
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
            Migrator::new(dir.as_path())
                .await
                .unwrap_or_else(|e| panic!("read migrations: {e}"))
                .run(&pool)
                .await
                .unwrap_or_else(|e| panic!("run migrations: {e}"));
        }

        Self { pool, schema }
    }

    async fn finish(self) {
        let TestEnv { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop schema");
        pool.close().await;
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn count(&self, sql: &str) -> i64 {
        let (n,): (i64,) = sqlx::query_as(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
        n
    }

    async fn user(&self, org: &str) -> Uuid {
        let id = Uuid::new_v4();
        self.exec(&format!(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ('{id}', '{org}'::uuid, '{}', 'ACTIVE', 'test')",
            unique("user")
        ))
        .await;
        id
    }

    async fn customer(&self, org: &str) -> (String, Uuid) {
        let user = self.user(org).await;
        let id = unique("CX");
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) \
             VALUES ('{id}', '{org}'::uuid, '{id}', 'TestCustomer', 'LIVE', now(), '{user}')"
        ))
        .await;
        (id, user)
    }

    async fn branch(&self, org: &str, branch_id: &str) {
        self.exec(&format!(
            "INSERT INTO branches (id, name, organization_id) VALUES ('{branch_id}', '{branch_id}', '{org}'::uuid) \
             ON CONFLICT DO NOTHING"
        ))
        .await;
    }

    async fn force_open_case(&self, customer_id: &str, org: &str, status: &str) -> Uuid {
        let id = Uuid::new_v4();
        let decided = !matches!(status, "DRAFT" | "SUBMITTED" | "UNDER_REVIEW");
        let reasoned = matches!(status, "REJECTED" | "NEEDS_UPDATE" | "REVOKED");
        let (reviewed_by, decided_at) = if decided {
            let user = self.user(org).await;
            (format!("'{user}'::uuid"), "now()")
        } else {
            ("NULL".to_owned(), "NULL")
        };
        let reason = if reasoned { "'OTHER'" } else { "NULL" };
        let submitted = if status == "DRAFT" { "NULL" } else { "now()" };
        self.exec(&format!(
            "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, \
             policy_version, version, submitted_at, decided_at, reviewed_by, decision_reason_code, created_at) \
             VALUES ('{id}', '{org}'::uuid, '{customer_id}', 'STAFF', '{status}', 1, 1, {submitted}, {decided_at}, {reviewed_by}, {reason}, now())"
        ))
        .await;
        id
    }

    async fn call_intake(
        &self,
        who_spec: Option<&str>,
        idempotency_key: Option<&str>,
        boundary: &str,
        payload: Vec<u8>,
    ) -> HttpAnswer {
        let state = AppState::new(Arc::new(common::settings(&[])), self.pool.clone());
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);

        let app = atest::init_service(
            App::new()
                .configure(move |cfg| xchange::bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;

        let mut req = atest::TestRequest::post()
            .uri(ROUTE)
            .insert_header((header::HOST, HOST))
            .insert_header(("origin", ORIGIN))
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("x-forwarded-proto", "https"))
            .insert_header((
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            ))
            .set_payload(payload);

        if let Some(spec) = who_spec {
            req = req.insert_header((AS, spec));
        }
        if let Some(k) = idempotency_key {
            req = req.insert_header(("idempotency-key", k));
        }

        let resp = atest::call_service(&app, req.to_request()).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

        HttpAnswer { status, text, json }
    }
}

// ── #2705 helpers ────────────────────────────────────────────────────────────

impl TestEnv {
    /// The stored state of a case: status, branch_id, version, document count.
    async fn case_row(&self, id: Uuid) -> (String, Option<String>, i32, i64) {
        let (status, branch, version): (String, Option<String>, i32) = sqlx::query_as(
            "SELECT status, branch_id, version FROM verification_cases WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("fetch the case");
        let docs = self
            .count(&format!(
                "SELECT count(*) FROM verification_documents WHERE case_id = '{id}'"
            ))
            .await;
        (status, branch, version, docs)
    }
}

fn error_code(a: &HttpAnswer) -> &str {
    a.json["error"]["code"]
        .as_str()
        .unwrap_or_else(|| a.json["error"].as_str().unwrap_or(""))
}

/// A reviewer of `branch` in ORG_1, and the spec that signs them in.
async fn reviewer(env: &TestEnv, branch: &str) -> String {
    let id = env.user(ORG_1).await;
    format!("{id}|VERIFICATION_REVIEWER|{branch}|{ORG_1}")
}

async fn intake(env: &TestEnv, spec: &str, customer_id: &str) -> HttpAnswer {
    let boundary = "bound2705";
    let payload = build_multipart(
        boundary,
        Some(customer_id),
        Some("PASSPORT"),
        Some(("file", "passport.png", &png())),
    );
    env.call_intake(Some(spec), Some(&unique("idem")), boundary, payload)
        .await
}

/// The number of backends currently waiting, directly or through another waiter, on `holder`'s locks. A second
/// row-lock waiter queues behind the first one, so it is blocked by that waiter rather than by `holder` itself.
async fn blocked_by(pool: &PgPool, holder: i32) -> i64 {
    sqlx::query_scalar(
        "WITH RECURSIVE chain(pid) AS ( \
             SELECT pid FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) \
             UNION \
             SELECT a.pid FROM pg_stat_activity a JOIN chain c ON c.pid = ANY(pg_blocking_pids(a.pid)) \
         ) SELECT count(*) FROM chain",
    )
    .bind(holder)
    .fetch_one(pool)
    .await
    .expect("read pg_stat_activity")
}

// ── #2876 ───────────────────────────────────────────────────────────────────

/// An open online DRAFT case (`branch_id` NULL); reviewers of B01 and B02 run intake for it at the same moment.
/// Exactly one answers 200 and files the case under its branch; the other answers 400 `caseLocked`. The case is
/// SUBMITTED with exactly one document.
#[actix_web::test]
async fn concurrent_intakes_at_two_branches_file_the_case_once() {
    let env = TestEnv::start().await;
    env.branch(ORG_1, B01).await;
    env.branch(ORG_1, B02).await;
    let (customer_id, _) = env.customer(ORG_1).await;
    let case_id = env.force_open_case(&customer_id, ORG_1, "DRAFT").await;
    let at_b01 = reviewer(&env, B01).await;
    let at_b02 = reviewer(&env, B02).await;

    // Hold the case row so both intakes reach it before either can finish.
    let mut holder = env.pool.begin().await.expect("begin the holder");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .expect("the holder's pid");
    sqlx::query("SELECT id FROM verification_cases WHERE id = $1 FOR UPDATE")
        .bind(case_id)
        .execute(&mut *holder)
        .await
        .expect("lock the case row");

    let release = async {
        let deadline = Instant::now() + Duration::from_secs(30);
        while blocked_by(&env.pool, holder_pid).await < 2 {
            assert!(
                Instant::now() < deadline,
                "both intakes must wait on the case row the test holds"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        holder.commit().await.expect("release the case row");
    };
    let (b01, b02, ()) = tokio::join!(
        intake(&env, &at_b01, &customer_id),
        intake(&env, &at_b02, &customer_id),
        release
    );

    let (winner, winner_branch, loser) = match (b01.status, b02.status) {
        (StatusCode::OK, _) => (&b01, B01, &b02),
        (_, StatusCode::OK) => (&b02, B02, &b01),
        _ => panic!(
            "one intake must win; got B01 {} {} and B02 {} {}",
            b01.status, b01.text, b02.status, b02.text
        ),
    };
    assert_eq!(
        loser.status,
        StatusCode::BAD_REQUEST,
        "the other intake must be refused, not also submit the case: winner {}, loser {}",
        winner.text,
        loser.text
    );
    assert_eq!(error_code(loser), "caseLocked", "{}", loser.text);

    let (status, branch, _, docs) = env.case_row(case_id).await;
    assert_eq!(status, "SUBMITTED");
    assert_eq!(
        branch.as_deref(),
        Some(winner_branch),
        "the case is filed under the winning intake's branch"
    );
    assert_eq!(docs, 1, "only the winning intake's document is attached");
    assert_eq!(
        env.count("SELECT count(*) FROM verification_cases").await,
        1,
        "no other case is opened"
    );
    env.finish().await;
}
