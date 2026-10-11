//! Specification tests for V0-B-session-end (#492; docs/design/xchange-v3-web.md
//! §12.19.4, D-S3): the `me` view has a version of its own, made from who is
//! signed in, so an account switch in another window answers `changed: true`.
//!
//! Written before the implementation by the lane controller. Do not edit,
//! delete or weaken.
//!
//! - `BranchView::own_version(&self, branch, who)`; `MeBranchView::own_version`
//!   answers `Some(identity_version(who))`.
//! - `identity_version(who)` is pure over `who.id()` and `who.role()`, and its
//!   value is in `0 ..= 2^53 - 1`: `since` must be `>= 0` (`view.rs:151`) and the
//!   browser reads `version` as a JavaScript number (`client.ts:96`).
//! - Consequences pinned here: the same caller gets the same version, before and
//!   after branch writes; another id or another role gets another version; with
//!   the old `since` that caller gets `changed: true` and its own `Me`.
//!
//! The view is reached through the whole application (`bootstrap::configure`),
//! as the browser reaches it. Who is signed in comes from the request header
//! `x-test-as`: `owner:<id>`, `staff:<id>:<ROLE>:<branch>`, or absent.
//!
//! These tests need a real PostgreSQL: `TEST_DATABASE_URL` must name an
//! existing database whose name ends in `_test`. They fail without it; they do
//! not skip. Each test works in a schema of its own.

mod common;

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
/// The largest integer a JavaScript number holds exactly: 2^53 - 1.
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

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
                 The me version tests need a real PostgreSQL and do not skip. Point it at an \
                 existing database whose name ends in `_test`, e.g.\n\
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
        let schema = unique("v3me");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B02', 'B02')")
            .execute(&pool)
            .await
            .expect("insert the branches this file names into branches");
        Self { pool, schema }
    }

    /// One committed `BranchTx` run on `branch` that writes nothing: the
    /// branch version still rises (#231).
    async fn write(&self, branch: &str) {
        BranchTx::new(Transactor::new(self.pool.clone()))
            .run(&BranchId::new(branch), |_tx| {
                Box::pin(async move { AppResult::Ok(()) })
            })
            .await
            .expect("a BranchTx run commits");
    }

    async fn branch_version(&self, branch: &str) -> i64 {
        BranchVersions::new(self.pool.clone())
            .current(&BranchId::new(branch))
            .await
            .expect("read branch_versions")
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

struct HeaderSessions;

fn sessions_of(spec: &str) -> Sessions {
    if spec.is_empty() {
        return Sessions {
            owner: None,
            staff: None,
        };
    }
    if let Some(id) = spec.strip_prefix("owner:") {
        return Sessions {
            owner: Some(Actor::owner(id)),
            staff: None,
        };
    }
    let mut parts = spec.splitn(4, ':');
    assert_eq!(parts.next(), Some("staff"), "caller spec {spec:?}");
    let id = parts.next().expect("an id");
    let role = Role::parse(parts.next().expect("a role")).expect("a known role");
    let branch = parts.next().expect("a branch");
    Sessions {
        owner: None,
        staff: Actor::staff(id, role, BranchId::new(branch)),
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

const STAFF_ROLES: [&str; 9] = [
    "FRONT",
    "BACKEND",
    "MANAGER",
    "RATES",
    "MONITOR",
    "FINANCE",
    "SYSTEM",
    "DOCUMENTS",
    "DOCUMENT_REVIEW",
];

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    body: Value,
}

/// `GET uri` as `who`, on the whole application.
async fn get(db: &TestDb, who: &str, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::get()
        .uri(uri)
        .insert_header((header::HOST, "xc.test"));
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "GET {uri} as {who:?}: a JSON body ({e}): {:?}",
            String::from_utf8_lossy(&bytes)
        )
    });
    Answer { status, body }
}

/// `GET /api/v3/views/me` as `who`: a changed answer with data. Returns its
/// version, checked to be an integer a JavaScript number holds exactly.
async fn me_version(db: &TestDb, who: &str) -> i64 {
    let a = get(db, who, "/api/v3/views/me").await;
    assert_eq!(a.status, StatusCode::OK, "me as {who}: body {}", a.body);
    assert_eq!(a.body["changed"], true, "me as {who}: {}", a.body);
    assert!(
        a.body.get("data").is_some_and(|d| !d.is_null()),
        "me as {who}: data in {}",
        a.body
    );
    let version = a.body["version"]
        .as_i64()
        .unwrap_or_else(|| panic!("me as {who}: an integer version in {}", a.body));
    assert!(
        (0..=MAX_SAFE_INTEGER).contains(&version),
        "me as {who}: version {version} is outside 0 ..= 2^53 - 1"
    );
    version
}

// ------------------------------------------------------------ tests

#[actix_web::test]
async fn the_same_caller_keeps_its_me_version_across_branch_writes() {
    let db = TestDb::start().await;
    for (who, branch) in [
        ("owner:owner-1", "MAIN"),
        ("staff:employee-7:RATES:XC-B01", "XC-B01"),
    ] {
        let first = me_version(&db, who).await;
        db.write(branch).await;
        db.write(branch).await;
        let branch_version = db.branch_version(branch).await;
        assert!(branch_version >= 2, "the writes moved {branch}");

        let second = me_version(&db, who).await;
        assert_eq!(
            second, first,
            "{who}: a write to {branch} (now version {branch_version}) does not move the me version"
        );

        let unchanged = get(&db, who, &format!("/api/v3/views/me?since={first}")).await;
        assert_eq!(
            unchanged.status,
            StatusCode::OK,
            "{who}: {}",
            unchanged.body
        );
        assert_eq!(
            unchanged.body,
            serde_json::json!({ "version": first, "changed": false }),
            "{who}: since = its own me version answers unchanged, with no data, after the writes"
        );

        let stale = get(
            &db,
            who,
            &format!("/api/v3/views/me?since={branch_version}"),
        )
        .await;
        assert_eq!(stale.status, StatusCode::OK, "{who}: {}", stale.body);
        assert_eq!(
            stale.body["version"], first,
            "{who}: the branch version is not the me version"
        );
        assert_eq!(stale.body["changed"], true, "{who}: {}", stale.body);
    }
    db.finish().await;
}

#[actix_web::test]
async fn another_account_gets_another_me_version_and_its_own_me_on_the_old_since() {
    let db = TestDb::start().await;
    let rates = "staff:employee-7:RATES:XC-B01";
    let other_id = "staff:employee-8:RATES:XC-B01";
    let other_role = "staff:employee-7:FRONT:XC-B01";
    let owner = "owner:owner-1";

    let v_rates = me_version(&db, rates).await;
    let v_other_id = me_version(&db, other_id).await;
    let v_other_role = me_version(&db, other_role).await;
    let v_owner = me_version(&db, owner).await;
    assert_ne!(v_rates, v_other_id, "another id, same role");
    assert_ne!(v_rates, v_other_role, "same id, another role");
    assert_ne!(v_rates, v_owner, "RATES staff and the owner");
    assert_ne!(
        v_other_id, v_other_role,
        "employee-8 RATES and employee-7 FRONT"
    );

    // OR-7.7 #1030: `me` answers `roles` as design §3.3 maps the old role (RATES → RATE_MANAGER and
    // FRONT → TELLER at the employee's branch; OWNER → ADMIN and EXECUTIVE, organisation-wide).
    for (who, version, id, roles) in [
        (
            other_id,
            v_other_id,
            "employee-8",
            serde_json::json!([{ "role": "RATE_MANAGER", "branchId": "XC-B01" }]),
        ),
        (
            other_role,
            v_other_role,
            "employee-7",
            serde_json::json!([{ "role": "TELLER", "branchId": "XC-B01" }]),
        ),
        (
            owner,
            v_owner,
            "owner-1",
            serde_json::json!([
                { "role": "ADMIN", "branchId": null },
                { "role": "EXECUTIVE", "branchId": null },
            ]),
        ),
    ] {
        let a = get(&db, who, &format!("/api/v3/views/me?since={v_rates}")).await;
        assert_eq!(a.status, StatusCode::OK, "{who}: {}", a.body);
        assert_eq!(
            a.body["changed"], true,
            "{who} with RATES staff's since: {}",
            a.body
        );
        assert_eq!(a.body["version"], version, "{who}: {}", a.body);
        assert_eq!(a.body["data"]["id"], id, "{who}: {}", a.body);
        assert_eq!(a.body["data"]["roles"], roles, "{who}: {}", a.body);
    }
    db.finish().await;
}

/// The Blueprint role design §3.3 maps each v3 staff role to (OR-7.7 #1030).
fn blueprint_role(v3: &str) -> &'static str {
    match v3 {
        "FRONT" => "TELLER",
        "BACKEND" | "MANAGER" => "BRANCH_MANAGER",
        "RATES" => "RATE_MANAGER",
        "FINANCE" => "FINANCE",
        "SYSTEM" => "ADMIN",
        "MONITOR" | "DOCUMENTS" | "DOCUMENT_REVIEW" => "VERIFICATION_REVIEWER",
        other => panic!("no §3.3 mapping written for {other}"),
    }
}

/// OR-7.7 #1030: the `me` version follows the identity `me` answers, `{id, roles}`. Two v3 roles that
/// §3.3 maps to the same Blueprint role at the same branch (BACKEND and MANAGER → BRANCH_MANAGER;
/// MONITOR, DOCUMENTS and DOCUMENT_REVIEW → VERIFICATION_REVIEWER) are one identity and share one
/// version; every other pair of callers has its own.
#[actix_web::test]
async fn every_account_and_mapped_role_has_its_own_me_version_within_a_javascript_safe_integer() {
    let db = TestDb::start().await;
    let mut callers = vec![
        ("owner:owner-1".to_string(), "owner-1|OWNER".to_string()),
        ("owner:owner-2".to_string(), "owner-2|OWNER".to_string()),
    ];
    for id in [
        "e-1",
        "e-10",
        "employee-7",
        "0f8c6c1e-6a0b-4c55-9d1e-6f1f1c2b3a4d",
    ] {
        for role in STAFF_ROLES {
            callers.push((
                format!("staff:{id}:{role}:XC-B01"),
                format!("{id}|{}@XC-B01", blueprint_role(role)),
            ));
        }
    }
    let mut by_identity: HashMap<String, (i64, String)> = HashMap::new();
    let mut by_version: HashMap<i64, String> = HashMap::new();
    for (who, identity) in &callers {
        let version = me_version(&db, who).await;
        match by_identity.get(identity) {
            Some((same, earlier)) => assert_eq!(
                version, *same,
                "{who} and {earlier} are the same identity {identity}, so one me version"
            ),
            None => {
                if let Some(other) = by_version.insert(version, identity.clone()) {
                    panic!("{who} ({identity}) and {other} share the me version {version}");
                }
                by_identity.insert(identity.clone(), (version, who.clone()));
            }
        }
    }
    // 2 owners + 4 ids × 6 Blueprint roles (9 v3 roles collapse into 6).
    assert_eq!(by_identity.len(), 2 + 4 * 6);
    assert_eq!(by_version.len(), by_identity.len());
    db.finish().await;
}

/// OR-7.7 #1030: a role row is a grant at a branch (`role@branch`, design §3.3), so the same id and
/// role at another branch is another identity with another version (the web reads it as "switched").
/// The owner's rows are organisation-wide, so naming a branch keeps the owner's version.
#[actix_web::test]
async fn the_me_version_follows_id_and_role_grants_at_their_branch() {
    let db = TestDb::start().await;
    db.write("XC-B02").await;
    let at_b01 = me_version(&db, "staff:employee-7:RATES:XC-B01").await;
    let at_b02 = me_version(&db, "staff:employee-7:RATES:XC-B02").await;
    assert_ne!(
        at_b01, at_b02,
        "the same id and role at another branch is another grant, so another version"
    );
    for _ in 0..3 {
        assert_eq!(
            me_version(&db, "staff:employee-7:RATES:XC-B01").await,
            at_b01,
            "stable at XC-B01"
        );
    }

    let owner = me_version(&db, "owner:owner-1").await;
    let named = get(&db, "owner:owner-1", "/api/v3/views/me?branch=XC-B02").await;
    assert_eq!(named.status, StatusCode::OK, "{}", named.body);
    assert_eq!(named.body["data"]["branch"], "XC-B02", "{}", named.body);
    assert_eq!(
        named.body["version"], owner,
        "the owner naming a branch keeps the version"
    );

    for _ in 0..3 {
        assert_eq!(me_version(&db, "owner:owner-1").await, owner, "stable");
    }
    db.finish().await;
}
