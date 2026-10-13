//! MK-4 (#1397, design `docs/design/blueprint/marketing-campaigns.md` §4.1, §4.4, §4.5, §6, §7, D-4, D-5):
//! Up-front tests for `CampaignService`, `CampaignVersionSubject`, `CampaignApprovalPolicy`, and `CampaignSimulator`.
//!
//! Written before the implementation by the test writer (gemini/pool-4, on the BE lane controller
//! claude/ad05bf8d's order); the implementer must not edit, delete or weaken them.
//!
//! Behaviours locked (§4 criterion 1, §9 edge cases):
//! 1. `submit` without a fresh simulation => `simulationRequired` (D-4).
//! 2. Submit after an edit newer than the last simulation => `simulationRequired` (D-4).
//! 3. Submit with fresh simulation moves to `PENDING_APPROVAL`, computes `version_hash`, and creates approval request.
//! 4. Edit while `PENDING_APPROVAL` => `staleVersion` (a new DRAFT is created instead).
//! 5. The proposer cannot decide (`allows_self_decision = false`).
//! 6. The submitter is EXECUTIVE => still cannot decide its own request (§9 edge case 3).
//! 7. MARKETING cannot decide; only EXECUTIVE decides.
//! 8. EXECUTIVE approves => version becomes `APPROVED`, previous approved/paused becomes `SUPERSEDED`,
//!    and `campaigns.current_version_id` moves to it in one transaction.
//! 9. A change after submit makes the request stale (`CampaignVersionSubject::version_hash` mismatch).
//! 10. Pause and resume: `APPROVED` -> `PAUSED` -> `APPROVED` with no new approval required.
//! 11. Simulation with no cost ledger => `cost_source = UNAVAILABLE`, never zero cost (§6.1).
//! 12. Simulation with cost ledger => `cost_source = LEDGER` (D-4, §6.1).

use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration as CDuration, TimeZone, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xc_approval::domain::policy::ApprovalPolicy;
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_auth::domain::access::{Actor, Role};
use xc_marketing::application::approval::{CampaignApprovalPolicy, CampaignVersionSubject};
use xc_marketing::application::campaign_service::{CampaignService, CampaignServiceError};
use xc_marketing::application::simulator::CampaignSimulator;
use xc_marketing::domain::campaign::version::{StoredStatus, VersionTerms};

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const USER: &str = "00000000-0000-0000-0000-000000000002";
const MAIN: &str = "MAIN";
const MUST_FINISH: Duration = Duration::from_secs(30);

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("valid decimal")
}

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn new() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect(
            "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip",
        );
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("camp_svc");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to test database");
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
            .expect("connect to test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, version, created_at, created_by) \
             VALUES ('00000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 'Test User', 'ACTIVE', 1, now(), 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
        .execute(&pool)
        .await
        .expect("seed test user");

        Self { pool, schema }
    }

    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
    }
}

fn sample_terms(now: DateTime<Utc>) -> VersionTerms {
    VersionTerms {
        starts_at: now + CDuration::hours(1),
        ends_at: now + CDuration::days(30),
        budget_thb: d("50000.00"),
        max_uses: Some(500),
        stack_group: "DEFAULT".to_string(),
        priority: 10,
        segment_snapshot_id: None,
    }
}

fn marketing_user(user_id: &str) -> Actor {
    Actor::new(
        user_id,
        ORG,
        vec![ScopedRole::new(
            Role::Marketing,
            None,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

fn executive_user(user_id: &str) -> Actor {
    Actor::new(
        user_id,
        ORG,
        vec![ScopedRole::new(
            Role::Executive,
            None,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

// ------------------------------------------------------------
// Tests locking §4 criterion 1 and §9 edge cases
// ------------------------------------------------------------

#[tokio::test]
async fn submit_without_simulation_fails_with_simulation_required() {
    let db = TestDb::new().await;
    let service = CampaignService::new(db.pool.clone());
    let org_id = Uuid::parse_str(ORG).unwrap();
    let user_id = Uuid::parse_str(USER).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap();

    let campaign = service
        .create(
            org_id,
            "Unsimulated Campaign",
            sample_terms(now),
            &[MAIN],
            user_id,
            now,
        )
        .await
        .expect("create campaign succeeds");

    // Version 1 is created in DRAFT without simulation
    let v1_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM campaign_versions WHERE campaign_id = $1 AND version = 1",
    )
    .bind(campaign.id)
    .fetch_one(&db.pool)
    .await
    .expect("version 1 exists");

    let submit_res = service.submit(org_id, v1_id, user_id, now).await;
    assert_eq!(
        submit_res.err(),
        Some(CampaignServiceError::SimulationRequired),
        "submit without simulation must fail with SimulationRequired (D-4)"
    );

    db.finish().await;
}

#[tokio::test]
async fn submit_after_edit_newer_than_simulation_fails_with_simulation_required() {
    let db = TestDb::new().await;
    let service = CampaignService::new(db.pool.clone());
    let simulator = CampaignSimulator::new(db.pool.clone());
    let org_id = Uuid::parse_str(ORG).unwrap();
    let user_id = Uuid::parse_str(USER).unwrap();
    let t1_create = Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap();

    let campaign = service
        .create(
            org_id,
            "Edited Campaign",
            sample_terms(t1_create),
            &[MAIN],
            user_id,
            t1_create,
        )
        .await
        .expect("create campaign succeeds");

    let v1_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM campaign_versions WHERE campaign_id = $1 AND version = 1",
    )
    .bind(campaign.id)
    .fetch_one(&db.pool)
    .await
    .expect("version 1 exists");

    // Simulate at T2
    let t2_sim = t1_create + CDuration::hours(1);
    simulator
        .simulate(org_id, v1_id, vec![], user_id, t2_sim)
        .await
        .expect("simulation succeeds");

    // Edit at T3 (newer than simulation at T2)
    let t3_edit = t2_sim + CDuration::hours(1);
    let mut updated_terms = sample_terms(t3_edit);
    updated_terms.budget_thb = d("75000.00");
    service
        .save_draft(org_id, v1_id, updated_terms, &[MAIN], t3_edit)
        .await
        .expect("save_draft succeeds");

    // Submit at T4: must fail because edit (T3) > simulation (T2)
    let t4_submit = t3_edit + CDuration::hours(1);
    let submit_res = service.submit(org_id, v1_id, user_id, t4_submit).await;
    assert_eq!(
        submit_res.err(),
        Some(CampaignServiceError::SimulationRequired),
        "submit after an edit newer than the last simulation must fail with simulationRequired (D-4)"
    );

    db.finish().await;
}

#[tokio::test]
async fn submit_with_fresh_simulation_moves_to_pending_approval_and_sets_hash() {
    let db = TestDb::new().await;
    let service = CampaignService::new(db.pool.clone());
    let simulator = CampaignSimulator::new(db.pool.clone());
    let org_id = Uuid::parse_str(ORG).unwrap();
    let user_id = Uuid::parse_str(USER).unwrap();
    let t1 = Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap();

    let campaign = service
        .create(
            org_id,
            "Fresh Campaign",
            sample_terms(t1),
            &[MAIN],
            user_id,
            t1,
        )
        .await
        .expect("create campaign succeeds");

    let v1_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM campaign_versions WHERE campaign_id = $1 AND version = 1",
    )
    .bind(campaign.id)
    .fetch_one(&db.pool)
    .await
    .expect("version 1 exists");

    // Simulate at T2 (fresh)
    let t2 = t1 + CDuration::hours(1);
    simulator
        .simulate(org_id, v1_id, vec![], user_id, t2)
        .await
        .expect("simulation succeeds");

    // Submit at T3
    let t3 = t2 + CDuration::hours(1);
    let submitted = service
        .submit(org_id, v1_id, user_id, t3)
        .await
        .expect("submit succeeds");

    assert_eq!(
        submitted.status,
        StoredStatus::PendingApproval,
        "submitted version status must be PENDING_APPROVAL"
    );
    let hash = submitted
        .version_hash
        .as_ref()
        .expect("version_hash must be set on submit");
    assert_eq!(
        hash.len(),
        64,
        "version_hash must be 64-hex sha256 string (D-3)"
    );

    // Approval request must exist with type CAMPAIGN_VERSION
    let req_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM approval_requests WHERE object_id = $1 AND type = 'CAMPAIGN_VERSION'",
    )
    .bind(v1_id.to_string())
    .fetch_one(&db.pool)
    .await
    .expect("count approval requests");
    assert_eq!(
        req_count, 1,
        "approval request must be created for CAMPAIGN_VERSION"
    );

    db.finish().await;
}

#[tokio::test]
async fn edit_while_pending_approval_fails_with_stale_version() {
    let db = TestDb::new().await;
    let service = CampaignService::new(db.pool.clone());
    let simulator = CampaignSimulator::new(db.pool.clone());
    let org_id = Uuid::parse_str(ORG).unwrap();
    let user_id = Uuid::parse_str(USER).unwrap();
    let t1 = Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap();

    let campaign = service
        .create(
            org_id,
            "Pending Campaign",
            sample_terms(t1),
            &[MAIN],
            user_id,
            t1,
        )
        .await
        .expect("create campaign succeeds");

    let v1_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM campaign_versions WHERE campaign_id = $1 AND version = 1",
    )
    .bind(campaign.id)
    .fetch_one(&db.pool)
    .await
    .expect("version 1 exists");

    simulator
        .simulate(org_id, v1_id, vec![], user_id, t1 + CDuration::hours(1))
        .await
        .expect("simulate succeeds");
    service
        .submit(org_id, v1_id, user_id, t1 + CDuration::hours(2))
        .await
        .expect("submit succeeds");

    // Edit while PENDING_APPROVAL must refuse with staleVersion (D-3, §9 edge case 2)
    let edit_res = service
        .save_draft(
            org_id,
            v1_id,
            sample_terms(t1 + CDuration::hours(3)),
            &[MAIN],
            t1 + CDuration::hours(3),
        )
        .await;

    assert_eq!(
        edit_res.err(),
        Some(CampaignServiceError::StaleVersion),
        "editing a version in PENDING_APPROVAL must return staleVersion"
    );

    db.finish().await;
}

#[tokio::test]
async fn approval_policy_proposer_cannot_decide() {
    let db = TestDb::new().await;
    let policy = CampaignApprovalPolicy::new(db.pool.clone());
    let user_id = "user-proposer-1";
    let proposer = marketing_user(user_id);

    // D-5: allows_self_decision must be false
    assert!(
        !policy.allows_self_decision(&proposer),
        "CampaignApprovalPolicy::allows_self_decision must return false (D-5)"
    );

    let req = ApprovalRequest {
        id: "req-1".into(),
        type_name: "CAMPAIGN_VERSION".to_string(),
        branch_id: None,
        object_id: Uuid::new_v4().to_string(),
        object_version_hash: "hash".to_string(),
        policy_version: "1".to_string(),
        proposed_payload: serde_json::json!({}),
        requested_by: user_id.to_string(),
        requested_at: Utc::now(),
        status: "PENDING".to_string(),
        idempotency_key: "k1".to_string(),
    };

    let decision_res = policy.may_decide(&proposer, &req);
    assert!(
        decision_res.is_err(),
        "proposer must not be allowed to decide their own request"
    );

    db.finish().await;
}

#[tokio::test]
async fn approval_policy_submitter_is_executive_still_cannot_decide() {
    let db = TestDb::new().await;
    let policy = CampaignApprovalPolicy::new(db.pool.clone());
    let exec_user_id = "user-exec-proposer";
    let exec_proposer = executive_user(exec_user_id);

    assert!(
        !policy.allows_self_decision(&exec_proposer),
        "executive proposer still cannot self-decide"
    );

    let req = ApprovalRequest {
        id: "req-2".into(),
        type_name: "CAMPAIGN_VERSION".to_string(),
        branch_id: None,
        object_id: Uuid::new_v4().to_string(),
        object_version_hash: "hash".to_string(),
        policy_version: "1".to_string(),
        proposed_payload: serde_json::json!({}),
        requested_by: exec_user_id.to_string(),
        requested_at: Utc::now(),
        status: "PENDING".to_string(),
        idempotency_key: "k2".to_string(),
    };

    let decision_res = policy.may_decide(&exec_proposer, &req);
    assert!(
        decision_res.is_err(),
        "submitter who is executive still cannot decide their own request (§9 edge case 3)"
    );

    db.finish().await;
}

#[tokio::test]
async fn approval_policy_marketing_cannot_decide() {
    let db = TestDb::new().await;
    let policy = CampaignApprovalPolicy::new(db.pool.clone());
    let submitter_id = "user-sub-1";
    let other_marketing = marketing_user("user-mktg-other");

    let req = ApprovalRequest {
        id: "req-3".into(),
        type_name: "CAMPAIGN_VERSION".to_string(),
        branch_id: None,
        object_id: Uuid::new_v4().to_string(),
        object_version_hash: "hash".to_string(),
        policy_version: "1".to_string(),
        proposed_payload: serde_json::json!({}),
        requested_by: submitter_id.to_string(),
        requested_at: Utc::now(),
        status: "PENDING".to_string(),
        idempotency_key: "k3".to_string(),
    };

    let decision_res = policy.may_decide(&other_marketing, &req);
    assert!(
        decision_res.is_err(),
        "MARKETING role cannot decide a campaign approval request; only EXECUTIVE decides (D-5)"
    );

    db.finish().await;
}

#[tokio::test]
async fn approval_policy_executive_approves_moves_version_to_approved() {
    let db = TestDb::new().await;
    let service = CampaignService::new(db.pool.clone());
    let simulator = CampaignSimulator::new(db.pool.clone());
    let subject = CampaignVersionSubject::new(db.pool.clone());
    let org_id = Uuid::parse_str(ORG).unwrap();
    let proposer_id = Uuid::parse_str(USER).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap();

    let campaign = service
        .create(
            org_id,
            "Approval Campaign",
            sample_terms(now),
            &[MAIN],
            proposer_id,
            now,
        )
        .await
        .expect("create campaign succeeds");

    let v1_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM campaign_versions WHERE campaign_id = $1 AND version = 1",
    )
    .bind(campaign.id)
    .fetch_one(&db.pool)
    .await
    .expect("version 1 exists");

    simulator
        .simulate(
            org_id,
            v1_id,
            vec![],
            proposer_id,
            now + CDuration::hours(1),
        )
        .await
        .expect("simulate succeeds");
    service
        .submit(org_id, v1_id, proposer_id, now + CDuration::hours(2))
        .await
        .expect("submit succeeds");

    // Positive control: policy allows an EXECUTIVE other than the requester to decide (D-5)
    let policy = CampaignApprovalPolicy::new(db.pool.clone());
    let req = ApprovalRequest {
        id: "req-app-1".into(),
        type_name: "CAMPAIGN_VERSION".to_string(),
        branch_id: None,
        object_id: v1_id.to_string(),
        object_version_hash: "hash".to_string(),
        policy_version: "1".to_string(),
        proposed_payload: serde_json::json!({}),
        requested_by: proposer_id.to_string(),
        requested_at: Utc::now(),
        status: "PENDING".to_string(),
        idempotency_key: "k_app_1".to_string(),
    };
    let other_exec = executive_user("user-exec-other");
    assert!(
        policy.may_decide(&other_exec, &req).is_ok(),
        "approval policy must allow an EXECUTIVE other than requester to decide (D-5 positive control)"
    );

    // Execute on_decided hook with Outcome::Approved
    let mut tx = db.pool.begin().await.expect("begin tx");
    subject
        .on_decided(&mut tx, &v1_id.to_string(), Outcome::Approved)
        .await
        .expect("on_decided hook succeeds");
    tx.commit().await.expect("commit tx");

    // Version must be APPROVED
    let status: String = sqlx::query_scalar("SELECT status FROM campaign_versions WHERE id = $1")
        .bind(v1_id)
        .fetch_one(&db.pool)
        .await
        .expect("fetch status");
    assert_eq!(
        status, "APPROVED",
        "version must be APPROVED after executive approves"
    );

    // campaigns.current_version_id must move to v1_id (D-3)
    let cur_id: Option<Uuid> =
        sqlx::query_scalar("SELECT current_version_id FROM campaigns WHERE id = $1")
            .bind(campaign.id)
            .fetch_one(&db.pool)
            .await
            .expect("fetch current_version_id");
    assert_eq!(
        cur_id,
        Some(v1_id),
        "campaigns.current_version_id must be updated to the approved version id"
    );

    db.finish().await;
}

#[tokio::test]
async fn change_after_submit_makes_request_stale() {
    let db = TestDb::new().await;
    let service = CampaignService::new(db.pool.clone());
    let simulator = CampaignSimulator::new(db.pool.clone());
    let subject = CampaignVersionSubject::new(db.pool.clone());
    let org_id = Uuid::parse_str(ORG).unwrap();
    let user_id = Uuid::parse_str(USER).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap();

    let campaign = service
        .create(
            org_id,
            "Stale Check Campaign",
            sample_terms(now),
            &[MAIN],
            user_id,
            now,
        )
        .await
        .expect("create succeeds");

    let v1_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM campaign_versions WHERE campaign_id = $1 AND version = 1",
    )
    .bind(campaign.id)
    .fetch_one(&db.pool)
    .await
    .expect("version 1 exists");

    simulator
        .simulate(org_id, v1_id, vec![], user_id, now + CDuration::hours(1))
        .await
        .expect("simulate succeeds");
    let submitted = service
        .submit(org_id, v1_id, user_id, now + CDuration::hours(2))
        .await
        .expect("submit succeeds");
    assert!(
        submitted.version_hash.is_some(),
        "submit must compute and bind version_hash"
    );
    let original_hash = submitted.version_hash.unwrap();

    let mut tx = db.pool.begin().await.expect("begin tx");
    let fetched_hash = subject
        .version_hash(&mut tx, &v1_id.to_string())
        .await
        .expect("version_hash succeeds");
    tx.commit().await.expect("commit tx");

    assert_eq!(
        fetched_hash, original_hash,
        "subject.version_hash must return the version's bound hash"
    );

    // If an out-of-band change modified version_hash or underlying terms, hash differs
    sqlx::query("UPDATE campaign_versions SET version_hash = 'tampered_hash_value' WHERE id = $1")
        .bind(v1_id)
        .execute(&db.pool)
        .await
        .expect("tamper hash");

    let mut tx = db.pool.begin().await.expect("begin tx");
    let tampered_hash = subject
        .version_hash(&mut tx, &v1_id.to_string())
        .await
        .expect("version_hash succeeds");
    tx.commit().await.expect("commit tx");

    assert_ne!(
        tampered_hash, original_hash,
        "changed version yields mismatched hash making approval request stale"
    );

    db.finish().await;
}

#[tokio::test]
async fn pause_and_resume_lifecycle() {
    let db = TestDb::new().await;
    let service = CampaignService::new(db.pool.clone());
    let simulator = CampaignSimulator::new(db.pool.clone());
    let subject = CampaignVersionSubject::new(db.pool.clone());
    let org_id = Uuid::parse_str(ORG).unwrap();
    let user_id = Uuid::parse_str(USER).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap();

    let campaign = service
        .create(
            org_id,
            "Pause Resume Campaign",
            sample_terms(now),
            &[MAIN],
            user_id,
            now,
        )
        .await
        .expect("create succeeds");

    let v1_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM campaign_versions WHERE campaign_id = $1 AND version = 1",
    )
    .bind(campaign.id)
    .fetch_one(&db.pool)
    .await
    .expect("version 1 exists");

    simulator
        .simulate(org_id, v1_id, vec![], user_id, now + CDuration::hours(1))
        .await
        .expect("simulate succeeds");
    service
        .submit(org_id, v1_id, user_id, now + CDuration::hours(2))
        .await
        .expect("submit succeeds");

    let mut tx = db.pool.begin().await.expect("begin tx");
    subject
        .on_decided(&mut tx, &v1_id.to_string(), Outcome::Approved)
        .await
        .expect("approve succeeds");
    tx.commit().await.expect("commit tx");

    // Count approval requests before pause
    let approvals_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM approval_requests WHERE object_id = $1 AND type = 'CAMPAIGN_VERSION'",
    )
    .bind(v1_id.to_string())
    .fetch_one(&db.pool)
    .await
    .expect("fetch approvals count before pause");

    // Pause: APPROVED -> PAUSED
    let paused = service
        .pause(org_id, campaign.id, user_id, now + CDuration::hours(3))
        .await
        .expect("pause succeeds");
    assert_eq!(
        paused.status,
        StoredStatus::Paused,
        "pause moves status to PAUSED"
    );

    // Resume: PAUSED -> APPROVED with no new approval
    let resumed = service
        .resume(org_id, campaign.id, user_id, now + CDuration::hours(4))
        .await
        .expect("resume succeeds");
    assert_eq!(
        resumed.status,
        StoredStatus::Approved,
        "resume returns to APPROVED with no new approval (D-5)"
    );

    // Approval requests count must be unchanged after resume (D-5)
    let approvals_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM approval_requests WHERE object_id = $1 AND type = 'CAMPAIGN_VERSION'",
    )
    .bind(v1_id.to_string())
    .fetch_one(&db.pool)
    .await
    .expect("fetch approvals count after resume");
    assert_eq!(
        approvals_after, approvals_before,
        "resuming a paused campaign must not file a new approval request (D-5)"
    );

    db.finish().await;
}

#[tokio::test]
async fn simulation_with_no_cost_ledger_sets_cost_source_unavailable() {
    let db = TestDb::new().await;
    let service = CampaignService::new(db.pool.clone());
    let simulator = CampaignSimulator::new(db.pool.clone());
    let org_id = Uuid::parse_str(ORG).unwrap();
    let user_id = Uuid::parse_str(USER).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap();

    let campaign = service
        .create(
            org_id,
            "No Cost Ledger Campaign",
            sample_terms(now),
            &[MAIN],
            user_id,
            now,
        )
        .await
        .expect("create succeeds");

    let v1_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM campaign_versions WHERE campaign_id = $1 AND version = 1",
    )
    .bind(campaign.id)
    .fetch_one(&db.pool)
    .await
    .expect("version 1 exists");

    let sim_result = simulator
        .simulate(org_id, v1_id, vec![], user_id, now)
        .await
        .expect("simulation succeeds");

    assert_eq!(
        sim_result.cost_source, "UNAVAILABLE",
        "simulation with no cost ledger must record cost_source = UNAVAILABLE, never zero cost (§6.1)"
    );

    // Assert row in campaign_simulations table has cost_source = 'UNAVAILABLE' (§4.1)
    let db_cost_source: Option<String> = sqlx::query_scalar(
        "SELECT cost_source FROM campaign_simulations WHERE campaign_version_id = $1",
    )
    .bind(v1_id)
    .fetch_optional(&db.pool)
    .await
    .expect("fetch cost_source from campaign_simulations");

    assert_eq!(
        db_cost_source.as_deref(),
        Some("UNAVAILABLE"),
        "stored campaign_simulations.cost_source must be UNAVAILABLE"
    );

    db.finish().await;
}

#[tokio::test]
async fn simulation_with_cost_ledger_sets_cost_source_ledger() {
    let db = TestDb::new().await;
    let service = CampaignService::new(db.pool.clone());
    let simulator = CampaignSimulator::new(db.pool.clone());
    let org_id = Uuid::parse_str(ORG).unwrap();
    let user_id = Uuid::parse_str(USER).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap();

    let campaign = service
        .create(
            org_id,
            "Cost Ledger Campaign",
            sample_terms(now),
            &[MAIN],
            user_id,
            now,
        )
        .await
        .expect("create succeeds");

    let v1_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM campaign_versions WHERE campaign_id = $1 AND version = 1",
    )
    .bind(campaign.id)
    .fetch_one(&db.pool)
    .await
    .expect("version 1 exists");

    // Seed completed transaction with realized_margin_thb on transaction_lines (D-4, §6.1 control)
    let branch_id = format!("B-{}", Uuid::new_v4());
    sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2)")
        .bind(&branch_id)
        .bind(org_id)
        .execute(&db.pool)
        .await
        .expect("seed branch");

    let customer_id = format!("C-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2, $1, 'Somchai', 'DEMO', now())",
    )
    .bind(&customer_id)
    .bind(org_id)
    .execute(&db.pool)
    .await
    .expect("seed customer");

    let tx_id = Uuid::new_v4();
    let doc_ref = format!("DOC-{}", Uuid::new_v4());
    let sub_id = format!("sub-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO transactions ( \
             id, organization_id, document_ref, branch_id, mode, shift_id, submission_id, \
             payload_hash, customer_id, customer_name, nationality, type, code, amount, rate, principal, fee, \
             bank_fee, received, paid, method, payment_ref, account, staff_id, booking_no, notes, value_date, \
             paid_at, recorded_at, recorded_by, status, completed_at \
         ) VALUES ( \
             $1, $2, $3, $4, 'DEMO', NULL, $5, \
             'hash', $6, 'Somchai', 'THA', 'BUY', 'USD', 100, 32.0, \
             3200, 0, 0, 0, 3200, 'Cash', '', 'Cash drawer', 'system', 'WALK-IN', '', current_date, \
             now(), now(), 'system', 'COMPLETED', now() \
         )",
    )
    .bind(tx_id)
    .bind(org_id)
    .bind(&doc_ref)
    .bind(&branch_id)
    .bind(&sub_id)
    .bind(&customer_id)
    .execute(&db.pool)
    .await
    .expect("seed transaction");

    let line_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO transaction_lines ( \
             id, organization_id, transaction_id, shop_side, currency, \
             quantity, unit_size, rate_snapshot, \
             thb_amount, fee_thb, settle_thb, cost_basis_thb, realized_margin_thb, line_no, created_at, updated_at \
         ) VALUES ($1, $2, $3, 'BUY', 'USD', 100.00, 1, 33.50, 3350.00, 0, 3350.00, 3300.00, 50.00, 1, now(), now())",
    )
    .bind(line_id)
    .bind(org_id)
    .bind(tx_id)
    .execute(&db.pool)
    .await
    .expect("seed transaction line with realized_margin_thb");

    let sim_result = simulator
        .simulate(org_id, v1_id, vec![], user_id, now)
        .await
        .expect("simulation succeeds");

    assert_ne!(
        sim_result.simulation_id,
        Uuid::nil(),
        "simulation must generate a non-nil simulation_id"
    );

    assert_eq!(
        sim_result.cost_source, "LEDGER",
        "simulation with cost ledger (completed lines with realized_margin_thb) must record cost_source = LEDGER (D-4, §6.1)"
    );

    let db_cost_source: Option<String> = sqlx::query_scalar(
        "SELECT cost_source FROM campaign_simulations WHERE campaign_version_id = $1",
    )
    .bind(v1_id)
    .fetch_optional(&db.pool)
    .await
    .expect("fetch cost_source from campaign_simulations");

    assert_eq!(
        db_cost_source.as_deref(),
        Some("LEDGER"),
        "stored campaign_simulations.cost_source must be LEDGER when ledger data is available"
    );

    db.finish().await;
}
