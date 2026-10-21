//! Ticket #2813: `/api/v3/views/executive_overview` (`Mount::Views` in `org_view.rs`) accepts `branches=B1,B2`
//! with the same parsing and refusals as `Mount::Org` (`/api/executive/overview`), keeps `branch=B1` and
//! the neither-key behaviour unchanged, and refuses sending both `branch` and `branches` with
//! `400 validationFailed` on `branches`.
//!
//! Needs `TEST_DATABASE_URL` naming a `_test` database; nothing skips. Each test works in a schema of its own.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const HOST: &str = "xc.test";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const ORG2: &str = "00000000-0000-0000-0000-0000000000f2";
const B1: &str = "XC-B01";
const B2: &str = "XC-B02";
const OTHER_ORG_B: &str = "XC-B90";
const EXECUTIVE: &str = "00000000-0000-0000-0000-0000000000e1";
const PERIOD: &str = "from=2026-10-05&to=2026-10-05";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
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
        let schema = unique("org_view_branches");
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
        for (branch, org) in [(B1, ORG), (B2, ORG), (OTHER_ORG_B, ORG2)] {
            Self::insert_branch(&pool, branch, org).await;
        }
        Room { pool, schema }
    }

    async fn insert_branch(pool: &PgPool, branch: &str, org: &str) {
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id, timezone) \
             VALUES ($1, $1, $2::uuid, 'Asia/Bangkok')",
        )
        .bind(branch)
        .bind(org)
        .execute(pool)
        .await
        .expect("a branch");
        sqlx::query(
            "INSERT INTO branch_versions (branch_id, version, organization_id, changed_at) \
             VALUES ($1, 1, $2::uuid, now())",
        )
        .bind(branch)
        .bind(org)
        .execute(pool)
        .await
        .expect("a branch version");
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

/// The organisation-wide EXECUTIVE of ORG.
struct Executive;

impl SessionReader for Executive {
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

fn refused(a: &Answer, status: StatusCode, code: &str, field: Option<&str>, what: &str) {
    assert_eq!(
        (a.status, a.body["error"]["code"].as_str().unwrap_or("")),
        (status, code),
        "{what}: {}",
        a.text
    );
    if let Some(expected_field) = field {
        assert_eq!(
            a.body["error"]["field"].as_str(),
            Some(expected_field),
            "{what}: {}",
            a.text
        );
    }
    assert!(
        a.body.get("data").is_none_or(Value::is_null),
        "{what}: {}",
        a.text
    );
}

fn row_ids(a: &Answer) -> Vec<String> {
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    a.body["data"]["branches"]
        .as_array()
        .unwrap_or_else(|| panic!("data.branches: {}", a.text))
        .iter()
        .map(|r| r["branchId"].as_str().unwrap_or("").to_owned())
        .collect()
}

fn version_keys(a: &Answer) -> Vec<String> {
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    a.body["versions"]
        .as_object()
        .unwrap_or_else(|| panic!("versions: {}", a.text))
        .keys()
        .cloned()
        .collect()
}

/// Row 1: `branches=B1` -> 200, data for B1 only.
#[actix_web::test]
async fn row_1_branches_b1_returns_200_with_data_for_b1_only() {
    let room = Room::start().await;
    let a = get(
        &room,
        &format!("/api/v3/views/executive_overview?branches={B1}&{PERIOD}"),
    )
    .await;
    assert_eq!(row_ids(&a), vec![B1]);
    assert_eq!(version_keys(&a), vec![B1]);
    room.finish().await;
}

/// Row 2: `branches=B1,B2` -> 200, data for B1 and B2 (deduplicated, sorted).
/// Note that ORG also has the seeded `"MAIN"` branch in scope, so ignoring `branches` would return `["MAIN", B1, B2]`.
#[actix_web::test]
async fn row_2_branches_b1_b2_returns_200_deduplicated_and_sorted() {
    let room = Room::start().await;

    let a = get(
        &room,
        &format!("/api/v3/views/executive_overview?branches={B2},{B1},{B2}&{PERIOD}"),
    )
    .await;
    assert_eq!(row_ids(&a), vec![B1, B2]);
    assert_eq!(version_keys(&a), vec![B1, B2]);

    let a2 = get(
        &room,
        &format!("/api/v3/views/executive_overview?branches={B1},{B2}&{PERIOD}"),
    )
    .await;
    assert_eq!(row_ids(&a2), vec![B1, B2]);
    assert_eq!(version_keys(&a2), vec![B1, B2]);
    room.finish().await;
}

/// Row 3: `branches=B1,` (empty item) -> 400 `validationFailed`, field `branches`.
#[actix_web::test]
async fn row_3_branches_with_empty_item_returns_400_validation_failed_on_branches() {
    let room = Room::start().await;
    for list in [
        format!("{B1},"),
        format!(",{B1}"),
        format!("{B1},,{B2}"),
        String::new(),
    ] {
        let a = get(
            &room,
            &format!("/api/v3/views/executive_overview?branches={list}&{PERIOD}"),
        )
        .await;
        refused(
            &a,
            StatusCode::BAD_REQUEST,
            "validationFailed",
            Some("branches"),
            &format!("branches={list}"),
        );
    }
    room.finish().await;
}

/// Row 4: `branches=OTHER-ORG-B` (a branch of another organisation) -> 403 `outOfScope` (same as `Mount::Org`).
#[actix_web::test]
async fn row_4_branches_with_other_org_branch_returns_403_out_of_scope() {
    let room = Room::start().await;
    for list in [
        OTHER_ORG_B.to_owned(),
        format!("{B1},{OTHER_ORG_B}"),
        "XC-NOPE".to_owned(),
    ] {
        let a = get(
            &room,
            &format!("/api/v3/views/executive_overview?branches={list}&{PERIOD}"),
        )
        .await;
        refused(
            &a,
            StatusCode::FORBIDDEN,
            "outOfScope",
            None,
            &format!("branches={list}"),
        );
    }
    room.finish().await;
}

/// Row 5: `branch=B1` (kept) -> 200, B1 only, unchanged.
#[actix_web::test]
async fn row_5_branch_b1_is_kept_and_returns_200_for_b1_only() {
    let room = Room::start().await;
    let a = get(
        &room,
        &format!("/api/v3/views/executive_overview?branch={B1}&{PERIOD}"),
    )
    .await;
    assert_eq!(row_ids(&a), vec![B1]);
    assert_eq!(version_keys(&a), vec![B1]);
    room.finish().await;
}

/// Row 6: neither key (kept) -> 200, every branch in scope, unchanged.
#[actix_web::test]
async fn row_6_neither_key_is_kept_and_returns_200_for_every_branch_in_scope() {
    let room = Room::start().await;
    let a = get(&room, &format!("/api/v3/views/executive_overview?{PERIOD}")).await;
    assert_eq!(row_ids(&a), vec!["MAIN", B1, B2]);
    assert_eq!(version_keys(&a), vec![B1, B2]);
    room.finish().await;
}

/// Row 7: both `branch=B1` and `branches=B2` -> 400 `validationFailed`, field `branches` (one filter at a time).
#[actix_web::test]
async fn row_7_both_branch_and_branches_returns_400_validation_failed_on_branches() {
    let room = Room::start().await;
    let a = get(
        &room,
        &format!("/api/v3/views/executive_overview?branch={B1}&branches={B2}&{PERIOD}"),
    )
    .await;
    refused(
        &a,
        StatusCode::BAD_REQUEST,
        "validationFailed",
        Some("branches"),
        "both branch and branches",
    );
    room.finish().await;
}
