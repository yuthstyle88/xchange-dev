//! Specification tests for #1463 (Old-cookie (v2) staff actors name their organisation
//! by code "XC" instead of its id, so v3 organisation checks refuse them).
//!
//! Written before implementation by test-writer (gemini/warut-6, ordered by
//! claude/5edf7bb5 and claude/ad05bf8d, Owner's Rule A). Do not edit, delete or weaken.
//!
//! Cases covered:
//! (a) An old-cookie teller (a staff session built through `old_cookie_callers` with resolved `org_id`)
//!     has an actor whose `organization_id` equals the organisation's real id from the `organizations`
//!     table, not "XC".
//! (b) That teller passes `QueueDesk::authorize_in` at their own branch (no OutOfScope / Forbidden).
//! (c) A teller is still refused (OutOfScope / Forbidden) at another organisation's branch.
//! (d) The startup lookup: if the organizations row for code XC is missing, resolving the organisation id
//!     at startup fails with a clear error.
//! (e) Trait default `SessionReader::callers` carries the seeded XC organisation ID
//!     ("00000000-0000-0000-0000-000000000001") for test readers.
//!
//! Needs TEST_DATABASE_URL; nothing skips.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::HttpRequest;
use actix_web::test as atest;
use chrono::Utc;
use pos_error::AppError;
use pos_infra::transactions::Transactor;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::actor::Actor as AccessActor;
use xc_auth::domain::access::authorizer::{Authorizer, Refusal, Target};
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::policy::value_objects::actor::Actor as LegacyActor;
use xc_auth::domain::policy::value_objects::role::Role as LegacyRole;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_queue::application::desk::queue_desk::QueueDesk;
use xc_queue::application::status::status_service::StatusService;
use xc_queue::domain::ticket::queue_ticket::TicketStatus;
use xc_queue::infrastructure::repository::PgQueueRepository;
use xchange::presentation::pages::{
    SessionReader, Sessions, old_cookie_callers, resolve_old_cookie_org_id,
};

const MUST_FINISH: Duration = Duration::from_secs(30);

const B01: &str = "XC-B01";
const OTHER_B01: &str = "OTHER-B01";
const ORG_XC: &str = "00000000-0000-0000-0000-000000000001";
const ORG_OTHER: &str = "00000000-0000-0000-0000-000000000002";
const TELLER_ID: &str = "00000000-0000-0000-0000-000000000010";

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
        let schema = unique("old_cookie_org");
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

        // The default organization 'XC' (00000000-0000-0000-0000-000000000001) is seeded by migrations.
        // Seed branch XC-B01 for organization 'XC'.
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id, code) VALUES ($1, 'Branch 1', $2::uuid, $1)",
        )
        .bind(B01)
        .bind(ORG_XC)
        .execute(&pool)
        .await
        .expect("insert own branch");

        // Seed a second organization 'OTHER' with another branch.
        sqlx::query(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ($1::uuid, 'OTHER', 'Other Org', 'Asia/Bangkok', now())",
        )
        .bind(ORG_OTHER)
        .execute(&pool)
        .await
        .expect("insert other organization");

        sqlx::query(
            "INSERT INTO branches (id, name, organization_id, code) VALUES ($1, 'Other Branch', $2::uuid, $1)",
        )
        .bind(OTHER_B01)
        .bind(ORG_OTHER)
        .execute(&pool)
        .await
        .expect("insert other branch");

        // Seed user row for teller under organization 'XC'.
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2::uuid, 'Teller 1', 'ACTIVE', 'test')",
        )
        .bind(TELLER_ID)
        .bind(ORG_XC)
        .execute(&pool)
        .await
        .expect("insert teller user");

        // Seed open shift for teller on XC-B01.
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('shift-1', $1, 'OPEN', now(), 'test', 'req-open', $2::uuid)",
        )
        .bind(B01)
        .bind(TELLER_ID)
        .execute(&pool)
        .await
        .expect("insert shift");

        // Seed active counter for teller on XC-B01.
        sqlx::query(
            "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
             VALUES ($1::uuid, $2, $3::uuid, 'ACTIVE') \
             ON CONFLICT (branch_id, teller_id) DO NOTHING",
        )
        .bind(ORG_XC)
        .bind(B01)
        .bind(TELLER_ID)
        .execute(&pool)
        .await
        .expect("insert queue_counter");

        // Ensure branch_service_status exists and is open for XC-B01.
        sqlx::query(
            "INSERT INTO branch_service_status (branch_id, organization_id, queue_open, active_counters, queue_version) \
             VALUES ($1, $2::uuid, true, 1, 1) \
             ON CONFLICT (branch_id) DO UPDATE SET queue_open = true, active_counters = 1, queue_version = 1",
        )
        .bind(B01)
        .bind(ORG_XC)
        .execute(&pool)
        .await
        .expect("insert branch_service_status for XC-B01");

        // Ensure branch_service_status exists for OTHER-B01.
        sqlx::query(
            "INSERT INTO branch_service_status (branch_id, organization_id, queue_open, active_counters, queue_version) \
             VALUES ($1, $2::uuid, true, 0, 1) \
             ON CONFLICT (branch_id) DO UPDATE SET queue_open = true, active_counters = 0, queue_version = 1",
        )
        .bind(OTHER_B01)
        .bind(ORG_OTHER)
        .execute(&pool)
        .await
        .expect("insert branch_service_status for OTHER-B01");

        Self { pool, schema }
    }

    async fn seed_waiting_ticket(&self, branch: &str, org: &str) -> Uuid {
        let ticket_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, \
             sequence, display_number, status, issued_at, idempotency_key) \
             VALUES ($1, $2::uuid, $3, current_date, 'WALK_IN', 1, 'W001', 'WAITING', now(), $4)",
        )
        .bind(ticket_id)
        .bind(org)
        .bind(branch)
        .bind(unique("idem"))
        .execute(&self.pool)
        .await
        .expect("insert waiting ticket");
        ticket_id
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

// ------------------------------------------------------------ tests

/// Case (a): An old-cookie teller (a staff session built through `old_cookie_callers` with resolved `org_id`)
/// has an actor whose `organization_id` equals the organisation's real id from the `organizations` table, not "XC".
///
/// Ruling (#1463 issuecomment-6126805322):
/// Callers are built the way production does: `resolve_old_cookie_org_id(&pool)` resolves the real id once,
/// and `old_cookie_callers(sessions, &org_id)` builds the callers carrying that id.
#[actix_web::test]
async fn old_cookie_teller_session_reader_assigns_organization_id_from_database_not_code() {
    let db = TestDb::start().await;
    let expected_org_id: String =
        sqlx::query_scalar("SELECT id::text FROM organizations WHERE code = 'XC'")
            .fetch_one(&db.pool)
            .await
            .expect("query org id for code XC");

    let org_id = resolve_old_cookie_org_id(&db.pool).await.expect("org XC");
    let callers = old_cookie_callers(
        Sessions {
            owner: None,
            staff: LegacyActor::staff(TELLER_ID.to_string(), LegacyRole::Front, BranchId::new(B01)),
        },
        &org_id,
    );
    let teller_actor: AccessActor = callers
        .staff_user
        .expect("staff_user must be present in Callers");

    assert_eq!(
        teller_actor.organization_id, expected_org_id,
        "SessionReader::callers must set the organization UUID from the database, not literal 'XC'"
    );
    assert_ne!(
        teller_actor.organization_id, "XC",
        "teller actor organization_id must not be literal code 'XC'"
    );

    db.finish().await;
}

/// Case (b): That teller passes `QueueDesk::authorize_in` at their own branch (no OutOfScope / Forbidden).
///
/// On base (main 202e5935):
/// `pages.rs:46` sets `let org = "XC"`, so the actor carries `organization_id = "XC"`.
/// `Authorizer::check` compares `"XC"` against the branch's organization id
/// (`00000000-0000-0000-0000-000000000001`), refuses `OutOfScope`, and `QueueDesk::authorize_in`
/// maps it to `AppError::Forbidden`.
#[actix_web::test]
async fn old_cookie_teller_at_own_branch_passes_queue_desk_authorize_in() {
    let db = TestDb::start().await;
    let waiting_ticket_id = db.seed_waiting_ticket(B01, ORG_XC).await;

    let org_id = resolve_old_cookie_org_id(&db.pool).await.expect("org XC");
    let callers = old_cookie_callers(
        Sessions {
            owner: None,
            staff: LegacyActor::staff(TELLER_ID.to_string(), LegacyRole::Front, BranchId::new(B01)),
        },
        &org_id,
    );
    let teller_actor: AccessActor = callers
        .staff_user
        .expect("staff_user must be built by SessionReader::callers");

    let branch_tx = BranchTx::new(Transactor::new(db.pool.clone()));
    let repo = Arc::new(PgQueueRepository::new(db.pool.clone()));
    let status = StatusService::new(branch_tx.clone(), repo.clone());
    let desk = QueueDesk::new(branch_tx, repo, status);
    let now = Utc::now();

    // At own branch: passes authorize_in and successfully calls the waiting ticket.
    let own_branch = BranchId::new(B01);
    let result = desk.call_next(&teller_actor, &own_branch, now).await;
    assert!(
        !matches!(result, Err(AppError::Forbidden)),
        "an old-cookie teller at their own branch must not fail authorize_in with Forbidden (outOfScope); got {result:?}"
    );
    let ticket = result
        .expect("an old-cookie teller at their own branch should successfully call next ticket");
    assert_eq!(ticket.id, waiting_ticket_id.to_string());
    assert_eq!(ticket.status, TicketStatus::Called);

    db.finish().await;
}

/// Case (c): A teller is still refused (OutOfScope / Forbidden) at another organisation's branch.
///
/// Asserts that the teller actor carries the real organization id from the database, and when
/// attempting to operate at a branch of another organisation, `QueueDesk::authorize_in` and
/// `Authorizer::check` strictly refuse with OutOfScope / Forbidden.
#[actix_web::test]
async fn old_cookie_teller_at_another_organization_branch_is_refused() {
    let db = TestDb::start().await;
    let expected_org_id: String =
        sqlx::query_scalar("SELECT id::text FROM organizations WHERE code = 'XC'")
            .fetch_one(&db.pool)
            .await
            .expect("query org id for code XC");

    let org_id = resolve_old_cookie_org_id(&db.pool).await.expect("org XC");
    let callers = old_cookie_callers(
        Sessions {
            owner: None,
            staff: LegacyActor::staff(TELLER_ID.to_string(), LegacyRole::Front, BranchId::new(B01)),
        },
        &org_id,
    );
    let teller_actor: AccessActor = callers
        .staff_user
        .expect("staff_user must be built by SessionReader::callers");

    // The teller actor must be bound to its real organisation id, not literal "XC"
    assert_eq!(
        teller_actor.organization_id, expected_org_id,
        "teller actor organization_id must equal the real organisation id from the database"
    );

    let branch_tx = BranchTx::new(Transactor::new(db.pool.clone()));
    let repo = Arc::new(PgQueueRepository::new(db.pool.clone()));
    let status = StatusService::new(branch_tx.clone(), repo.clone());
    let desk = QueueDesk::new(branch_tx, repo, status);
    let now = Utc::now();

    // At another organisation's branch: refused with Forbidden (mapped from OutOfScope)
    let other_branch = BranchId::new(OTHER_B01);
    let other_result = desk.call_next(&teller_actor, &other_branch, now).await;
    assert!(
        matches!(other_result, Err(AppError::Forbidden)),
        "an old-cookie teller at another organisation's branch must be refused with Forbidden (outOfScope); got {other_result:?}"
    );

    // Direct check via Authorizer: returns Refusal::OutOfScope
    let other_target = Target::new(ORG_OTHER, Some(other_branch), None);
    let refusal = Authorizer::check(
        Some(&teller_actor),
        Permission::QueueOperate,
        &other_target,
        now,
    );
    assert_eq!(
        refusal,
        Err(Refusal::OutOfScope),
        "Authorizer::check must refuse teller at another organisation's branch with OutOfScope"
    );

    db.finish().await;
}

/// Case (d): Startup lookup: if the organisations row for code XC is missing, resolving the
/// organisation id at startup fails with a clear error.
///
/// Lookup (lane controller ruling, 2026-10-12): an organisation's code-to-id mapping never
/// changes, so the id of the old cookie's organisation (code XC) is resolved once at startup
/// from `organizations` and held by `SessionReader`. If the row is missing at startup, boot
/// fails with a clear message rather than serving refused sessions.
#[actix_web::test]
async fn startup_lookup_fails_when_organization_xc_row_is_missing() {
    let db = TestDb::start().await;

    // Rename the code of the seeded organisation so there is no row with code 'XC'
    sqlx::query("UPDATE organizations SET code = 'MISSING' WHERE code = 'XC'")
        .execute(&db.pool)
        .await
        .expect("rename organization code");

    // With code 'XC' missing from organizations, resolve_old_cookie_org_id must return Err with a clear message
    let result = resolve_old_cookie_org_id(&db.pool).await;
    assert!(
        result.is_err(),
        "startup lookup must fail with an error when organization row for code 'XC' is missing"
    );
    let err_msg = result.unwrap_err();
    assert!(
        err_msg.contains("XC") || err_msg.contains("organization"),
        "error message must clearly mention missing organisation XC, got: {err_msg}"
    );

    db.finish().await;
}

// ------------------------------------------------------------ test reader for trait default

/// A test reader that implements only `sessions` and relies on the default
/// trait implementation of `SessionReader::callers` from `presentation/pages.rs`.
struct TestOnlySessionReader {
    staff: Option<LegacyActor>,
}

impl SessionReader for TestOnlySessionReader {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let staff = self.staff.clone();
        Box::pin(async move { Sessions { owner: None, staff } })
    }
}

/// Case (e): Trait default `SessionReader::callers` carries the seeded XC organisation ID
/// ("00000000-0000-0000-0000-000000000001") for test readers.
///
/// Ruling (#1463 issuecomment-6132082032):
/// The migration seeds the old cookie's organisation with a fixed id (api/migrations/20261011100000_organizations.sql:9-10).
/// The trait-default `callers` passes the seeded XC organisation id to `old_cookie_callers`, so test readers that
/// implement only `sessions` receive the seeded id without performing runtime database access.
#[actix_web::test]
async fn old_cookie_trait_default_callers_carry_the_seeded_xc_org_id() {
    let reader = TestOnlySessionReader {
        staff: LegacyActor::staff(TELLER_ID.to_string(), LegacyRole::Front, BranchId::new(B01)),
    };
    let req = atest::TestRequest::default().to_http_request();
    let callers = reader.callers(&req).await;
    let staff_user: AccessActor = callers
        .staff_user
        .expect("staff_user must be present in Callers");

    // Seeded organization id for code 'XC' from api/migrations/20261011100000_organizations.sql:9-10.
    assert_eq!(
        staff_user.organization_id, "00000000-0000-0000-0000-000000000001",
        "trait default callers must carry the seeded XC organization id for test readers"
    );
}
