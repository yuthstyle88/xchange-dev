//! Ticket #1071, its folded-in part "the count-difference alert" (was EX-12 #1072): A01's "เงินนับไม่ตรง" alert
//! (`docs/design/blueprint/executive-finance.md` §4.3).
//!
//! Written before the implementation by the rule-A test-writer claude/dell-claude-5, on the BE lane controller
//! claude/ad05bf8d's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! One test per row of the folded section's §9 table. The alert is read where A01 answers it, in
//! `GET /api/executive/overview`'s `data.alerts`, with `kind = "countDifference"` (its §7, [required]).
//!
//! **Not pinned, by the ticket's own text:** the alert's drill-down. Its §7 makes it whichever of `cash_counts`
//! (CA-8 #624) or `finance_reconciliation` (FN-1 #1068) is on `main` at claim time, else `null`.
//!
//! **Choice of this file:** the open and reviewed cash exceptions are written in the shape the custody exceptions
//! reader on `main` decodes (`xc-shift/src/infrastructure/custody/decode.rs`, `exception_from_row`): `requestId`,
//! `status` `OPEN`/`REVIEWED`, `note`, `at`. The ticket says to read them through that reader, never a second
//! parser of `data`.
//!
//! Needs `TEST_DATABASE_URL` naming a `_test` database; nothing skips. Each test works in a schema of its own.

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
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const ORG2: &str = "00000000-0000-0000-0000-0000000000f2";
const B1: &str = "XC-B01";
const B2: &str = "XC-B02";
/// A branch of another organisation: outside the EXECUTIVE's `BranchScope`.
const B9: &str = "XC-B90";
const KIND: &str = "countDifference";
/// The organisation-wide EXECUTIVE of ORG.
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

fn org_of(branch: &str) -> &'static str {
    if branch == B9 { ORG2 } else { ORG }
}

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
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
        let schema = unique("ex12_count_diff");
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
        sqlx::query(
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ($1::uuid, 'ORG2', 'Asia/Bangkok', now())",
        )
        .bind(ORG2)
        .execute(&pool)
        .await
        .expect("a second organisation");
        for branch in [B1, B2, B9] {
            sqlx::query(
                "INSERT INTO branches (id, name, organization_id, timezone) \
                 VALUES ($1, $1, $2::uuid, 'Asia/Bangkok')",
            )
            .bind(branch)
            .bind(org_of(branch))
            .execute(&pool)
            .await
            .expect("a branch");
            sqlx::query(
                "INSERT INTO branch_versions (branch_id, version, organization_id, changed_at) \
                 VALUES ($1, 1, $2::uuid, now())",
            )
            .bind(branch)
            .bind(org_of(branch))
            .execute(&pool)
            .await
            .expect("a branch version");
        }
        Room { pool, schema }
    }

    /// A `COUNT_DIFFERENCE` approval request for `branch`, in `status`.
    async fn count_difference_request(&self, branch: &str, status: &str) -> String {
        sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
               proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
             VALUES (gen_random_uuid(), 'COUNT_DIFFERENCE', $1, $2, 'h', 'v1', \
               '{\"currency\": \"USD\", \"difference\": \"-100\"}'::jsonb, 'test', now(), $3, $2, $4::uuid) \
             RETURNING id::text",
        )
        .bind(branch)
        .bind(unique("count"))
        .bind(status)
        .bind(org_of(branch))
        .fetch_one(&self.pool)
        .await
        .expect("a COUNT_DIFFERENCE request")
    }

    /// A cash exception of `branch` in custody's `status` (`OPEN` / `REVIEWED`), in the shape the custody reader
    /// decodes.
    async fn exception(&self, branch: &str, status: &str) -> String {
        let id = unique("EX");
        let at = Utc::now().timestamp_millis();
        let reviewed = status == "REVIEWED";
        let data = json!({
            "requestId": unique("REQ"),
            "status": status,
            "cashVariance": { "USD": -10000 },
            "note": "short at close",
            "at": at,
            "reviewedBy": if reviewed { json!("finance-1") } else { Value::Null },
            "reviewedAt": if reviewed { json!(at + 1000) } else { Value::Null },
        });
        sqlx::query(
            "INSERT INTO cash_exceptions (id, branch_id, position, data, organization_id) \
             VALUES ($1, $2, (SELECT coalesce(max(position), 0) + 1 FROM cash_exceptions WHERE branch_id = $2), \
               $3, $4::uuid)",
        )
        .bind(&id)
        .bind(branch)
        .bind(data)
        .bind(org_of(branch))
        .execute(&self.pool)
        .await
        .expect("a cash exception");
        id
    }

    /// A verified CLOSE count of `branch` whose USD total has `difference`.
    async fn verified_count(&self, branch: &str, difference: &str) {
        let mut ids = Vec::new();
        for name in ["counter", "verifier"] {
            let id: String = sqlx::query_scalar(
                "INSERT INTO users (organization_id, name, status, created_by) \
                 VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
            )
            .bind(org_of(branch))
            .bind(unique(name))
            .fetch_one(&self.pool)
            .await
            .expect("a user");
            ids.push(id);
        }
        let count: String = sqlx::query_scalar(
            "INSERT INTO cash_counts (organization_id, branch_id, kind, counted_by, counted_at, idempotency_key, \
               verified_by, verified_at) \
             VALUES ($1::uuid, $2, 'CLOSE', $3::uuid, now(), $4, $5::uuid, now()) RETURNING id::text",
        )
        .bind(org_of(branch))
        .bind(branch)
        .bind(&ids[0])
        .bind(unique("key"))
        .bind(&ids[1])
        .fetch_one(&self.pool)
        .await
        .expect("a verified count");
        sqlx::query(
            "INSERT INTO cash_count_totals (count_id, organization_id, currency, counted, expected, difference) \
             VALUES ($1::uuid, $2::uuid, 'USD', 500, 500 - $3::numeric, $3::numeric)",
        )
        .bind(&count)
        .bind(org_of(branch))
        .bind(difference)
        .execute(&self.pool)
        .await
        .expect("its total");
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
                user: Some(user.clone()),
                staff_user: Some(user),
            }
        })
    }
}

/// The `countDifference` alerts A01 answers the organisation-wide EXECUTIVE of ORG, as (branch, alert).
async fn count_difference_alerts(room: &Room) -> Vec<(String, Value)> {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(Executive);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri("/api/executive/overview?from=2026-10-05&to=2026-10-05")
        .insert_header((header::HOST, HOST))
        .insert_header((AS, "executive"))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert_eq!(status, StatusCode::OK, "A01 answers: {text}");
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    body["data"]["alerts"]
        .as_array()
        .unwrap_or_else(|| panic!("data.alerts: {text}"))
        .iter()
        .filter(|a| a["kind"] == KIND)
        .map(|a| (a["branchId"].as_str().unwrap_or("").to_owned(), a.clone()))
        .collect()
}

fn branches(alerts: &[(String, Value)]) -> Vec<&str> {
    alerts.iter().map(|(b, _)| b.as_str()).collect()
}

/// A PENDING `COUNT_DIFFERENCE` request at B1: one `countDifference` alert for B1.
#[actix_web::test]
async fn a_pending_count_difference_request_is_one_alert_for_its_branch() {
    let room = Room::start().await;
    room.count_difference_request(B1, "PENDING").await;
    let alerts = count_difference_alerts(&room).await;
    assert_eq!(branches(&alerts), vec![B1], "{alerts:?}");
    room.finish().await;
}

/// The same request APPROVED or REJECTED: no alert.
#[actix_web::test]
async fn a_decided_count_difference_request_is_no_alert() {
    let room = Room::start().await;
    room.count_difference_request(B1, "APPROVED").await;
    room.count_difference_request(B2, "REJECTED").await;
    let alerts = count_difference_alerts(&room).await;
    assert!(alerts.is_empty(), "{alerts:?}");
    room.finish().await;
}

/// An open cash exception at B2: one alert for B2.
#[actix_web::test]
async fn an_open_cash_exception_is_one_alert_for_its_branch() {
    let room = Room::start().await;
    room.exception(B2, "OPEN").await;
    let alerts = count_difference_alerts(&room).await;
    assert_eq!(branches(&alerts), vec![B2], "{alerts:?}");
    room.finish().await;
}

/// The same exception reviewed: no alert.
#[actix_web::test]
async fn a_reviewed_cash_exception_is_no_alert() {
    let room = Room::start().await;
    room.exception(B2, "REVIEWED").await;
    let alerts = count_difference_alerts(&room).await;
    assert!(alerts.is_empty(), "{alerts:?}");
    room.finish().await;
}

/// A verified count with difference 0: no alert.
#[actix_web::test]
async fn a_verified_count_with_no_difference_is_no_alert() {
    let room = Room::start().await;
    room.verified_count(B1, "0").await;
    let alerts = count_difference_alerts(&room).await;
    assert!(alerts.is_empty(), "{alerts:?}");
    room.finish().await;
}

/// A branch outside the caller's `BranchScope` (another organisation's) is not listed, whatever it holds; the
/// caller's own branch still is.
#[actix_web::test]
async fn a_branch_outside_the_callers_scope_is_not_listed() {
    let room = Room::start().await;
    room.count_difference_request(B9, "PENDING").await;
    room.exception(B9, "OPEN").await;
    room.count_difference_request(B1, "PENDING").await;
    let alerts = count_difference_alerts(&room).await;
    assert_eq!(branches(&alerts), vec![B1], "{alerts:?}");
    room.finish().await;
}
