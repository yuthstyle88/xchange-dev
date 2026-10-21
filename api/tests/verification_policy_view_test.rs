//! #1878 — the `verification_policy` view (design `docs/design/blueprint/verification.md` §4.5).
//!
//! F02's at-branch form is caseless, and `branch_intake` checks the organisation's **latest** published policy
//! (`api/crates/xc-verification/src/application/branch_intake.rs:181-190`). This view answers that same policy to a
//! reviewer, so the screen can show the form exactly when the server would accept it.
//!
//! Wire shape (`GET /api/v3/views/verification_policy`, the shared envelope `{version, changed, data}`):
//! `data = { policyVersion, branchReviewAllowed, acceptedDocumentTypes, documentsRequired }`, and the envelope
//! `version` is the latest policy version, so an unchanged `since` answers `{changed: false}` until a new version
//! is published.
//!
//! 1. `a_reviewer_reads_the_latest_policy_of_their_organisation`
//! 2. `a_newer_published_version_wins_over_an_older_one`
//! 3. `branch_review_allowed_false_is_answered_false`
//! 4. `an_unchanged_since_answers_changed_false_and_a_new_version_answers_changed_true`
//! 5. `each_reviewer_reads_their_own_organisations_policy`
//! 6. `without_verification_review_the_view_is_refused`
//! 7. `an_organisation_with_no_policy_is_refused_with_no_verification_policy`

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
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const VIEW: &str = "/api/v3/views/verification_policy";

/// Seeded by `20261011100000_organizations.sql`; `20261205121000_verification.sql` backfills its policy version 1
/// (`PASSPORT`, `NATIONAL_ID`; 1 document; `branch_review_allowed = true`).
const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
/// Created here with its own policy version 1.
const ORG_2: &str = "00000000-0000-0000-0000-000000000002";
/// Created here with no policy at all (an organisation created after the backfill, before any publish).
const ORG_3: &str = "00000000-0000-0000-0000-000000000003";
/// Users that publish versions above 1 in ORG_1 and ORG_2.
const PUBLISHER_1: &str = "99999999-0000-0000-0000-000000000001";
const PUBLISHER_2: &str = "99999999-0000-0000-0000-000000000002";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

#[derive(Clone, Debug)]
struct Who {
    user_id: String,
    role: Role,
    branch: Option<String>,
    customer_id: Option<String>,
    org_id: String,
}

impl Who {
    fn reviewer(user_id: &str, org_id: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            role: Role::VerificationReviewer,
            branch: None,
            customer_id: None,
            org_id: org_id.to_owned(),
        }
    }

    fn teller(user_id: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            role: Role::Teller,
            branch: Some("MAIN".to_owned()),
            customer_id: None,
            org_id: ORG_1.to_owned(),
        }
    }

    fn customer(user_id: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            role: Role::Customer,
            branch: None,
            customer_id: Some("CX_1".to_owned()),
            org_id: ORG_1.to_owned(),
        }
    }

    fn spec(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.user_id,
            self.role.as_str(),
            self.branch.as_deref().unwrap_or(""),
            self.customer_id.as_deref().unwrap_or(""),
            self.org_id,
        )
    }
}

/// Builds the caller from the `x-test-as` header, the same way `verification_f02_test.rs` does.
struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();

        Box::pin(async move {
            let user = if spec.is_empty() {
                None
            } else {
                let parts: Vec<&str> = spec.split('|').collect();
                let branch = if parts[2].is_empty() {
                    None
                } else {
                    Some(BranchId::new(parts[2]))
                };
                let customer_id = if parts[3].is_empty() {
                    None
                } else {
                    Some(parts[3])
                };
                Role::parse(parts[1]).map(|r| {
                    let from = Utc::now() - ChronoDuration::days(30);
                    let scoped = ScopedRole::new(r, branch.clone(), from, None);
                    let mut actor = User::new(parts[0], parts[4], vec![scoped], customer_id);
                    actor.branch = branch;
                    actor
                })
            };

            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> String {
        self.body["error"]["code"]
            .as_str()
            .or_else(|| self.body["code"].as_str())
            .unwrap_or("")
            .to_owned()
    }

    fn ok(&self, what: &str) -> &Value {
        assert_eq!(self.status, StatusCode::OK, "{what}: {}", self.text);
        &self.body
    }
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

        let schema = unique("vpol");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to test db");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema pool");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");

        let room = Self { pool, schema };
        room.seed().await;
        room
    }

    async fn seed(&self) {
        sqlx::query(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) VALUES \
             ($1, 'ORG2', 'Second Org', 'Asia/Bangkok', now()), \
             ($2, 'ORG3', 'Third Org', 'Asia/Bangkok', now()) \
             ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::from_str(ORG_2).unwrap())
        .bind(Uuid::from_str(ORG_3).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed organisations 2 and 3");
        // ORG_3 is created after the migration's backfill, so it has no policy; make sure of it.
        sqlx::query("DELETE FROM verification_policies WHERE organization_id = $1")
            .bind(Uuid::from_str(ORG_3).unwrap())
            .execute(&self.pool)
            .await
            .expect("ORG_3 has no policy");
        // A version above 1 must name its publisher (`CHECK (version = 1 OR published_by IS NOT NULL)`).
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) VALUES \
             ($1::uuid, $3, 'Org1 Publisher', 'ACTIVE', 'seed'), \
             ($2::uuid, $4, 'Org2 Publisher', 'ACTIVE', 'seed') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(PUBLISHER_1)
        .bind(PUBLISHER_2)
        .bind(Uuid::from_str(ORG_1).unwrap())
        .bind(Uuid::from_str(ORG_2).unwrap())
        .execute(&self.pool)
        .await
        .expect("seed publishers");
        self.publish(ORG_2, 1, &["PASSPORT", "NATIONAL_ID"], 1, true)
            .await;
    }

    /// Publishes a policy version (V11: a published version is never edited; a change is a new version).
    async fn publish(
        &self,
        org: &str,
        version: i32,
        accepted: &[&str],
        documents_required: i32,
        branch_review_allowed: bool,
    ) {
        let accepted: Vec<String> = accepted.iter().map(|s| (*s).to_owned()).collect();
        let publisher: Option<Uuid> = match (version, org) {
            (1, _) => None,
            (_, ORG_1) => Some(Uuid::from_str(PUBLISHER_1).unwrap()),
            (_, ORG_2) => Some(Uuid::from_str(PUBLISHER_2).unwrap()),
            _ => panic!("no publisher seeded for {org}"),
        };
        sqlx::query(
            "INSERT INTO verification_policies \
             (organization_id, version, accepted_document_types, documents_required, branch_review_allowed, \
              published_by, published_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, now(), now()) ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::from_str(org).unwrap())
        .bind(version)
        .bind(&accepted)
        .bind(documents_required)
        .bind(branch_review_allowed)
        .bind(publisher)
        .execute(&self.pool)
        .await
        .expect("publish a policy version");
    }

    async fn finish(self) {
        let Room { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
    }
}

async fn get(room: &Room, who: &Who, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
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
        .insert_header((AS, who.spec()))
        .insert_header((
            header::COOKIE,
            format!("__Host-xc_session=session-{}", who.user_id),
        ))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    Answer {
        status,
        text: String::from_utf8_lossy(&bytes).into_owned(),
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

fn reviewer_1() -> Who {
    Who::reviewer("11111111-1111-1111-1111-111111111111", ORG_1)
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap_or_else(|| panic!("expected an array, got {v}"))
        .iter()
        .map(|s| s.as_str().expect("a string").to_owned())
        .collect()
}

// 1.
#[actix_web::test]
async fn a_reviewer_reads_the_latest_policy_of_their_organisation() {
    let room = Room::start().await;

    let ans = get(&room, &reviewer_1(), VIEW).await;
    let body = ans.ok("verification_policy for ORG_1");
    assert_eq!(body["changed"], Value::Bool(true), "{}", ans.text);
    assert_eq!(
        body["version"].as_i64(),
        Some(1),
        "cursor = latest policy version: {}",
        ans.text
    );
    let data = &body["data"];
    assert_eq!(data["policyVersion"].as_i64(), Some(1), "{}", ans.text);
    assert_eq!(
        data["branchReviewAllowed"],
        Value::Bool(true),
        "{}",
        ans.text
    );
    assert_eq!(data["documentsRequired"].as_i64(), Some(1), "{}", ans.text);
    assert_eq!(
        strings(&data["acceptedDocumentTypes"]),
        vec!["PASSPORT".to_owned(), "NATIONAL_ID".to_owned()],
        "{}",
        ans.text
    );

    room.finish().await;
}

// 2.
#[actix_web::test]
async fn a_newer_published_version_wins_over_an_older_one() {
    let room = Room::start().await;
    room.publish(ORG_1, 2, &["PASSPORT"], 2, true).await;

    let ans = get(&room, &reviewer_1(), VIEW).await;
    let body = ans.ok("verification_policy after version 2");
    assert_eq!(body["version"].as_i64(), Some(2), "{}", ans.text);
    let data = &body["data"];
    assert_eq!(data["policyVersion"].as_i64(), Some(2), "{}", ans.text);
    assert_eq!(data["documentsRequired"].as_i64(), Some(2), "{}", ans.text);
    assert_eq!(
        strings(&data["acceptedDocumentTypes"]),
        vec!["PASSPORT".to_owned()],
        "{}",
        ans.text
    );

    room.finish().await;
}

// 3.
#[actix_web::test]
async fn branch_review_allowed_false_is_answered_false() {
    let room = Room::start().await;
    room.publish(ORG_1, 2, &["PASSPORT", "NATIONAL_ID"], 1, false)
        .await;

    let ans = get(&room, &reviewer_1(), VIEW).await;
    let body = ans.ok("verification_policy with branch review off");
    assert_eq!(
        body["data"]["branchReviewAllowed"],
        Value::Bool(false),
        "the latest version forbids at-branch review, the same policy branch_intake refuses on: {}",
        ans.text
    );

    room.finish().await;
}

// 4.
#[actix_web::test]
async fn an_unchanged_since_answers_changed_false_and_a_new_version_answers_changed_true() {
    let room = Room::start().await;
    let reviewer = reviewer_1();

    let first = get(&room, &reviewer, VIEW).await;
    let version = first.ok("first read")["version"]
        .as_i64()
        .expect("the answer carries a version");

    let unchanged = get(&room, &reviewer, &format!("{VIEW}?since={version}")).await;
    assert_eq!(
        unchanged.ok("unchanged poll")["changed"],
        Value::Bool(false),
        "since = current version must answer changed: false: {}",
        unchanged.text
    );

    room.publish(ORG_1, 2, &["PASSPORT", "NATIONAL_ID"], 1, false)
        .await;

    let moved = get(&room, &reviewer, &format!("{VIEW}?since={version}")).await;
    let body = moved.ok("poll after a new version");
    assert_eq!(body["changed"], Value::Bool(true), "{}", moved.text);
    assert_ne!(body["version"].as_i64(), Some(version), "{}", moved.text);
    assert_eq!(
        body["data"]["branchReviewAllowed"],
        Value::Bool(false),
        "the poll carries the new policy: {}",
        moved.text
    );

    room.finish().await;
}

// 5.
#[actix_web::test]
async fn each_reviewer_reads_their_own_organisations_policy() {
    let room = Room::start().await;
    room.publish(ORG_2, 2, &["PASSPORT"], 1, false).await;

    let org1 = get(&room, &reviewer_1(), VIEW).await;
    assert_eq!(
        org1.ok("ORG_1 reviewer")["data"]["branchReviewAllowed"],
        Value::Bool(true),
        "ORG_2's policy must not leak into ORG_1: {}",
        org1.text
    );

    let reviewer_2 = Who::reviewer("44444444-4444-4444-4444-444444444444", ORG_2);
    let org2 = get(&room, &reviewer_2, VIEW).await;
    let body = org2.ok("ORG_2 reviewer");
    assert_eq!(
        body["data"]["branchReviewAllowed"],
        Value::Bool(false),
        "{}",
        org2.text
    );
    assert_eq!(
        body["data"]["policyVersion"].as_i64(),
        Some(2),
        "{}",
        org2.text
    );

    room.finish().await;
}

// 6.
#[actix_web::test]
async fn without_verification_review_the_view_is_refused() {
    let room = Room::start().await;

    for (label, who) in [
        (
            "teller",
            Who::teller("33333333-3333-3333-3333-333333333333"),
        ),
        (
            "customer",
            Who::customer("77777777-7777-7777-7777-777777777777"),
        ),
    ] {
        let ans = get(&room, &who, VIEW).await;
        assert!(
            ans.status == StatusCode::FORBIDDEN || ans.status == StatusCode::BAD_REQUEST,
            "{label}: expected a refusal, got {} {}",
            ans.status,
            ans.text
        );
        let code = ans.code();
        assert!(
            code == "outOfScope" || code == "roleNotAllowedHere",
            "{label}: code was '{code}': {}",
            ans.text
        );
        assert!(
            !ans.text.contains("branchReviewAllowed"),
            "{label}: a refusal must not carry the policy: {}",
            ans.text
        );
    }

    room.finish().await;
}

// 7.
#[actix_web::test]
async fn an_organisation_with_no_policy_is_refused_with_no_verification_policy() {
    let room = Room::start().await;
    let reviewer_3 = Who::reviewer("55555555-5555-5555-5555-555555555555", ORG_3);

    let ans = get(&room, &reviewer_3, VIEW).await;
    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "no policy is refused like branch_intake refuses it (400): {}",
        ans.text
    );
    assert_eq!(ans.code(), "noVerificationPolicy", "{}", ans.text);

    room.finish().await;
}
