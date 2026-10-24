//! The five v3 verification review commands must not decide a case at a branch where the caller holds
//! no `VerificationReview` role, even when the caller's session branch is that branch.
//!
//! Written before the fix by the ticket author's helper, on the xchange BE lane controller claude/7ca80c's
//! order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The defect: `reviewer_actor` in `verification_case_{approve,reject,request_update,revoke,start_review}.rs`
//! takes `who.branch` (the session branch) as the reviewer's branch when `BranchScope::of(who,
//! VerificationReview, now)` is `Branches(branches)`, without checking `branches.contains(branch)`. A user who
//! reviews at branch A and is only a TELLER at branch B can switch the session to B and decide B's cases,
//! because `ReviewService::check_reviewer_scope` only compares the case's branch with the scope it is given.
//! The branch intake route (`branch_verification_intake.rs`) and the case view (`v3/views/verification_case.rs`)
//! already refuse that caller with 403 `outOfScope`.
//!
//! Behaviour locked here:
//!
//! | #  | Caller roles                     | Session branch | Case branch / status | Command        | Expected                          |
//! |----|----------------------------------|----------------|----------------------|----------------|-----------------------------------|
//! | R1 | REVIEWER@A + TELLER@B            | B              | B / UNDER_REVIEW     | approve        | 403 `outOfScope`, case unchanged  |
//! | R2 | REVIEWER@A + TELLER@B            | B              | B / UNDER_REVIEW     | reject         | 403 `outOfScope`, case unchanged  |
//! | R3 | REVIEWER@A + TELLER@B            | B              | B / UNDER_REVIEW     | request-update | 403 `outOfScope`, case unchanged  |
//! | R4 | REVIEWER@A + TELLER@B            | B              | B / VERIFIED         | revoke         | 403 `outOfScope`, case unchanged  |
//! | R5 | REVIEWER@A + TELLER@B            | B              | B / SUBMITTED        | start-review   | 403 `outOfScope`, case unchanged  |
//! | R6 | REVIEWER@A + TELLER@B (guard)    | A              | A / each of the above| all five       | 200, the command's next status    |
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.
//! Each test works in a schema of its own.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{Duration as ChronoDuration, NaiveDate, TimeZone, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::actor::Actor as AccessActor;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const BRANCH_A: &str = "RSCOPE_A";
const BRANCH_B: &str = "RSCOPE_B";
const USER: &str = "7ca80c00-0000-4000-8000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

/// The caller under test: VERIFICATION_REVIEWER at branch A, TELLER at branch B, signed in on `session_branch`.
fn reviewer_at_a_teller_at_b(session_branch: &str) -> AccessActor {
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let roles = vec![
        ScopedRole::new(
            Role::VerificationReviewer,
            Some(BranchId::new(BRANCH_A)),
            from,
            None,
        ),
        ScopedRole::new(Role::Teller, Some(BranchId::new(BRANCH_B)), from, None),
    ];
    AccessActor::new(USER, ORG_1, roles, None).with_branch(Some(BranchId::new(session_branch)))
}

struct FixedActorReader(AccessActor);

impl SessionReader for FixedActorReader {
    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let user = Some(self.0.clone());
        Box::pin(async move {
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
}

/// One of the five review commands: its path segment, the case status it starts from, the status it
/// leaves on success, and its body (without `expectedVersion`).
#[derive(Clone, Copy)]
enum Cmd {
    Approve,
    Reject,
    RequestUpdate,
    Revoke,
    StartReview,
}

impl Cmd {
    const ALL: [Cmd; 5] = [
        Cmd::Approve,
        Cmd::Reject,
        Cmd::RequestUpdate,
        Cmd::Revoke,
        Cmd::StartReview,
    ];

    fn segment(self) -> &'static str {
        match self {
            Cmd::Approve => "approve",
            Cmd::Reject => "reject",
            Cmd::RequestUpdate => "request-update",
            Cmd::Revoke => "revoke",
            Cmd::StartReview => "start-review",
        }
    }

    fn starts_from(self) -> &'static str {
        match self {
            Cmd::Approve | Cmd::Reject | Cmd::RequestUpdate => "UNDER_REVIEW",
            Cmd::Revoke => "VERIFIED",
            Cmd::StartReview => "SUBMITTED",
        }
    }

    fn leaves(self) -> &'static str {
        match self {
            Cmd::Approve => "VERIFIED",
            Cmd::Reject => "REJECTED",
            Cmd::RequestUpdate => "NEEDS_UPDATE",
            Cmd::Revoke => "REVOKED",
            Cmd::StartReview => "UNDER_REVIEW",
        }
    }

    fn body(self, expires_at: NaiveDate) -> Value {
        match self {
            Cmd::Approve => json!({"expectedVersion": 1, "expiresAt": expires_at.to_string()}),
            Cmd::Reject => json!({"expectedVersion": 1, "reasonCode": "DOCUMENT_EXPIRED"}),
            Cmd::RequestUpdate => {
                json!({"expectedVersion": 1, "reasonCode": "DOCUMENT_UNREADABLE"})
            }
            Cmd::Revoke => json!({"expectedVersion": 1, "reasonCode": "SUSPECTED_FRAUD"}),
            Cmd::StartReview => json!({"expectedVersion": 1}),
        }
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

        let schema = unique("rscope");
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
        let org = Uuid::from_str(ORG_1).unwrap();
        sqlx::query(
            "INSERT INTO branches (id, organization_id, name) VALUES \
             ($1, $3, 'Reviewer Branch A'), ($2, $3, 'Teller Branch B') \
             ON CONFLICT (id) DO UPDATE SET organization_id = EXCLUDED.organization_id",
        )
        .bind(BRANCH_A)
        .bind(BRANCH_B)
        .bind(org)
        .execute(&self.pool)
        .await
        .expect("seed branches A and B");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'Reviewer A / Teller B', 'ACTIVE', 'seed') ON CONFLICT (id) DO NOTHING",
        )
        .bind(USER)
        .bind(org)
        .execute(&self.pool)
        .await
        .expect("seed the caller");

        let hash: String = Sha256::digest(format!("session-{USER}").as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        sqlx::query(
            "INSERT INTO user_sessions (hash, user_id, version, expires, organization_id) \
             VALUES ($1, $2::uuid, 1, now() + interval '8 hours', $3) ON CONFLICT (hash) DO NOTHING",
        )
        .bind(hash)
        .bind(USER)
        .bind(org)
        .execute(&self.pool)
        .await
        .expect("seed a live session");
    }

    async fn today_bangkok(&self) -> NaiveDate {
        sqlx::query_scalar::<_, NaiveDate>("SELECT (now() AT TIME ZONE 'Asia/Bangkok')::date")
            .fetch_one(&self.pool)
            .await
            .expect("today in Bangkok")
    }

    /// A case at `branch` in the status `cmd` starts from, version 1, for a customer of its own.
    async fn case_for(&self, cmd: Cmd, branch: &str) -> Uuid {
        let org = Uuid::from_str(ORG_1).unwrap();
        let customer = format!("CX_{}", Uuid::new_v4().simple());
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($1, $2, $1, 'Scope Customer', 'LIVE', now())",
        )
        .bind(&customer)
        .bind(org)
        .execute(&self.pool)
        .await
        .expect("seed customer");

        let status = cmd.starts_from();
        let verified = status == "VERIFIED";
        let now = Utc::now();
        let case_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO verification_cases \
             (id, organization_id, customer_id, provider, external_case_ref, status, policy_version, branch_id, \
              submitted_at, decided_at, expires_at, reviewed_by, decision_reason_code, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'STAFF', NULL, $4, 1, $5, $6, $7, $8, $9, NULL, 1, now(), now())",
        )
        .bind(case_id)
        .bind(org)
        .bind(&customer)
        .bind(status)
        .bind(branch)
        .bind(Some(now - ChronoDuration::days(if verified { 10 } else { 0 })))
        .bind(verified.then(|| now - ChronoDuration::days(10)))
        .bind(verified.then(|| now + ChronoDuration::days(300)))
        .bind(verified.then(|| Uuid::parse_str(USER).unwrap()))
        .execute(&self.pool)
        .await
        .expect("insert verification case");
        case_id
    }

    async fn status_and_version(&self, case_id: Uuid) -> (String, i32) {
        sqlx::query_as::<_, (String, i32)>(
            "SELECT status, version FROM verification_cases WHERE id = $1",
        )
        .bind(case_id)
        .fetch_one(&self.pool)
        .await
        .expect("read the case back")
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

async fn decide(room: &Room, actor: AccessActor, cmd: Cmd, case_id: Uuid) -> Answer {
    let expires_at = room.today_bangkok().await + ChronoDuration::days(365);
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(FixedActorReader(actor));
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let uri = format!(
        "/api/operations/verification/cases/{case_id}/{}",
        cmd.segment()
    );
    let req = atest::TestRequest::post()
        .uri(&uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::COOKIE, format!("__Host-xc_session=session-{USER}")))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("rscope-key").as_str()))
        .set_payload(cmd.body(expires_at).to_string());

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// R1–R5: session branch B, case at B, the caller reviews only at A ⇒ 403 `outOfScope`, case unchanged.
async fn refused_at_the_teller_branch(cmd: Cmd) {
    let room = Room::start().await;
    let case_id = room.case_for(cmd, BRANCH_B).await;

    let answer = decide(&room, reviewer_at_a_teller_at_b(BRANCH_B), cmd, case_id).await;

    assert_eq!(
        answer.status,
        StatusCode::FORBIDDEN,
        "{}: a reviewer at {BRANCH_A} who is only a teller at {BRANCH_B} must not decide a {BRANCH_B} case \
         by switching the session to {BRANCH_B}; body: {}",
        cmd.segment(),
        answer.text
    );
    assert_eq!(
        answer.code(),
        "outOfScope",
        "{}: body: {}",
        cmd.segment(),
        answer.text
    );
    assert_eq!(
        room.status_and_version(case_id).await,
        (cmd.starts_from().to_owned(), 1),
        "{}: the refused case must be left as it was",
        cmd.segment()
    );

    room.finish().await;
}

#[actix_web::test]
async fn r1_approve_refuses_a_case_at_a_branch_where_the_caller_is_only_a_teller() {
    refused_at_the_teller_branch(Cmd::Approve).await;
}

#[actix_web::test]
async fn r2_reject_refuses_a_case_at_a_branch_where_the_caller_is_only_a_teller() {
    refused_at_the_teller_branch(Cmd::Reject).await;
}

#[actix_web::test]
async fn r3_request_update_refuses_a_case_at_a_branch_where_the_caller_is_only_a_teller() {
    refused_at_the_teller_branch(Cmd::RequestUpdate).await;
}

#[actix_web::test]
async fn r4_revoke_refuses_a_case_at_a_branch_where_the_caller_is_only_a_teller() {
    refused_at_the_teller_branch(Cmd::Revoke).await;
}

#[actix_web::test]
async fn r5_start_review_refuses_a_case_at_a_branch_where_the_caller_is_only_a_teller() {
    refused_at_the_teller_branch(Cmd::StartReview).await;
}

/// R6 (guard, passes today): the same caller on session branch A decides A's cases with every command.
#[actix_web::test]
async fn r6_the_same_caller_on_the_reviewer_branch_decides_that_branchs_cases() {
    let room = Room::start().await;
    for cmd in Cmd::ALL {
        let case_id = room.case_for(cmd, BRANCH_A).await;
        let answer = decide(&room, reviewer_at_a_teller_at_b(BRANCH_A), cmd, case_id).await;
        assert_eq!(
            answer.status,
            StatusCode::OK,
            "{}: a reviewer at {BRANCH_A} on session branch {BRANCH_A} decides a {BRANCH_A} case; body: {}",
            cmd.segment(),
            answer.text
        );
        assert_eq!(
            answer.body["status"].as_str(),
            Some(cmd.leaves()),
            "{}: body: {}",
            cmd.segment(),
            answer.text
        );
        assert_eq!(
            room.status_and_version(case_id).await,
            (cmd.leaves().to_owned(), 2),
            "{}: the decided case is stored",
            cmd.segment()
        );
    }
    room.finish().await;
}
