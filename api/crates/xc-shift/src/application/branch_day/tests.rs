use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Side};

use crate::application::branch_day::service::{
    BeginDraft, BranchDayService, ClockDirection, StartService,
};
use crate::application::shift::service::ShiftService;
use crate::domain::custody::repositories::custody_ports::{CustodyPorts, OutsideFacts};
use crate::domain::custody::repositories::custody_repository::CustodyRepository;
use crate::domain::custody::value_objects::custody_error::CustodyError;
use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::custody::value_objects::phase::Phase;
use crate::domain::custody::value_objects::service_window::ServiceWindow;
use crate::domain::shift::value_objects::balances::Balances;
use crate::infrastructure::custody::custody_repository::PgCustodyRepository;
use crate::infrastructure::shift::shift_repository::PgShiftRepository;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

struct TestPorts {
    allows_begin: bool,
    fail_outside_facts: bool,
}

impl Default for TestPorts {
    fn default() -> Self {
        Self {
            allows_begin: true,
            fail_outside_facts: false,
        }
    }
}

impl CustodyPorts for TestPorts {
    fn outside_facts<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _open_shift_id: Option<&'a str>,
    ) -> BoxFuture<'a, AppResult<OutsideFacts>> {
        let fail = self.fail_outside_facts;
        Box::pin(async move {
            if fail {
                return Err(pos_error::AppError::Domain(
                    "outside_facts read failed".into(),
                ));
            }
            Ok(OutsideFacts {
                has_live_rate: true,
                rates_updated: "2026-10-11T08:00:00Z".to_string(),
                ..Default::default()
            })
        })
    }

    fn customer_known<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async { Ok(true) })
    }

    fn evidence_owned<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _owner_id: &'a str,
        _keys: &'a [String],
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async { Ok(true) })
    }

    fn service_window<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _now: CustodyTime,
    ) -> BoxFuture<'a, AppResult<ServiceWindow>> {
        let allows = self.allows_begin;
        Box::pin(async move {
            if allows {
                Ok(ServiceWindow::Open {
                    open: CustodyTime::from_epoch_millis(0),
                    close: CustodyTime::from_epoch_millis(i64::MAX / 2),
                })
            } else {
                Ok(ServiceWindow::Closed)
            }
        })
    }

    fn utc_offset_minutes<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _at: CustodyTime,
    ) -> BoxFuture<'a, AppResult<i32>> {
        Box::pin(async { Ok(420) })
    }
}

struct TestDb {
    pool: PgPool,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set.");
        });
        let options = PgConnectOptions::from_str(&url).expect("valid db url");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(database.ends_with("_test"));

        let schema = unique("bd_events");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("admin connect");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(16)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect schema pool");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read migrations")
            .run(&pool)
            .await
            .expect("apply migrations");

        Self { pool }
    }

    async fn branch(&self, label: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $2)")
            .bind(&id)
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert branch");
        BranchId::new(id)
    }

    fn service(
        &self,
        ports: Arc<dyn CustodyPorts>,
    ) -> BranchDayService<PgCustodyRepository, PgShiftRepository> {
        let repo = Arc::new(PgCustodyRepository::new(self.pool.clone()));
        let branch_tx = BranchTx::new(Transactor::new(self.pool.clone()));
        let shifts = ShiftService::new(
            Arc::new(PgShiftRepository::new(self.pool.clone())),
            branch_tx,
        );
        BranchDayService::new(repo, shifts, ports)
    }
}

const MANAGER_ID: &str = "00000000-0000-4000-8000-000000000001";
const TELLER_ID: &str = "00000000-0000-4000-8000-000000000002";
const FINANCE_ID: &str = "00000000-0000-4000-8000-000000000004";
const REVIEWER_ID: &str = "00000000-0000-4000-8000-000000000005";
const ORG_ID: &str = "00000000-0000-0000-0000-000000000001";

async fn seed_test_user(pool: &PgPool, id: &str, name: &str) {
    let org_id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM organizations WHERE code = 'XC'")
        .fetch_one(pool)
        .await
        .expect("XC org");
    let uid = uuid::Uuid::parse_str(id).unwrap();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2, $3, 'ACTIVE', 'test') \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(uid)
    .bind(org_id)
    .bind(name)
    .execute(pool)
    .await
    .expect("seed user");
}

async fn seed_open_shift_for_teller(pool: &PgPool, branch: &BranchId, teller_id: &str) -> String {
    let shift_id = uuid::Uuid::new_v4().to_string();
    let teller_uuid = uuid::Uuid::parse_str(teller_id).unwrap();
    let count_id = uuid::Uuid::new_v4();
    let org_id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM organizations WHERE code = 'XC'")
        .fetch_one(pool)
        .await
        .expect("XC org");
    sqlx::query(
        "INSERT INTO cash_counts (id, organization_id, branch_id, kind, counted_by, counted_at, idempotency_key) \
         VALUES ($1, $2, $3, 'OPEN', $4, now(), $5)",
    )
    .bind(count_id)
    .bind(org_id)
    .bind(branch.as_str())
    .bind(teller_uuid)
    .bind(format!("idem-{}", count_id))
    .execute(pool)
    .await
    .expect("insert cash count");

    sqlx::query(
        "INSERT INTO shifts (id, branch_id, opened_by, opened_at, status, teller_id, open_request_id) \
         VALUES ($1, $2, 'test', now(), 'OPEN', $3, $4::text)",
    )
    .bind(&shift_id)
    .bind(branch.as_str())
    .bind(teller_uuid)
    .bind(count_id)
    .execute(pool)
    .await
    .expect("insert shift");
    shift_id
}

async fn set_branch_phase(pool: &PgPool, branch: &BranchId, phase: Phase) {
    sqlx::query(
        "INSERT INTO ops_state (branch_id, phase) VALUES ($1, $2) \
         ON CONFLICT (branch_id) DO UPDATE SET phase = EXCLUDED.phase",
    )
    .bind(branch.as_str())
    .bind(phase.as_str())
    .execute(pool)
    .await
    .expect("set branch phase");
}

fn actor_scoped(user_id: &str, role: Role, branch: &BranchId) -> Actor {
    Actor::new(
        user_id,
        ORG_ID,
        vec![ScopedRole::new(
            role,
            Some(branch.clone()),
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            None,
        )],
        None,
    )
    .with_branch(Some(branch.clone()))
}

async fn locked_tx(pool: &PgPool, branch: &BranchId) -> Transaction<'static, Postgres> {
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query(
        "INSERT INTO branch_versions (branch_id, version) VALUES ($1, 1) \
         ON CONFLICT (branch_id) DO NOTHING",
    )
    .bind(branch.as_str())
    .execute(&mut *tx)
    .await
    .expect("seed branch_version");
    sqlx::query("SELECT branch_id FROM branch_versions WHERE branch_id = $1 FOR UPDATE")
        .bind(branch.as_str())
        .execute(&mut *tx)
        .await
        .expect("lock branch");
    tx
}

fn now() -> DateTime<Utc> {
    Utc::now()
}

/// Must-fix 1: write the ops_events rows.
/// Pins one row per step for start-service, target, clock-in, clock-out, hold, release, cut-off;
/// and verifies begin_draft emits 0 rows.
#[tokio::test]
async fn test_ops_events_pinned_one_row_per_step() {
    let db = TestDb::start().await;
    let branch = db.branch("BD_EV_PIN").await;
    let ports = Arc::new(TestPorts::default());
    let service = db.service(ports);

    for (id, name) in [
        (MANAGER_ID, "Manager"),
        (TELLER_ID, "Teller"),
        (FINANCE_ID, "Finance"),
        (REVIEWER_ID, "Reviewer"),
    ] {
        seed_test_user(&db.pool, id, name).await;
    }

    let bm = actor_scoped(MANAGER_ID, Role::BranchManager, &branch);
    let teller = actor_scoped(TELLER_ID, Role::Teller, &branch);
    let fin = actor_scoped(FINANCE_ID, Role::Finance, &branch);
    let reviewer = actor_scoped(REVIEWER_ID, Role::VerificationReviewer, &branch);

    set_branch_phase(&db.pool, &branch, Phase::Ready).await;
    seed_open_shift_for_teller(&db.pool, &branch, TELLER_ID).await;

    let mut tx = locked_tx(&db.pool, &branch).await;

    // 1. start_service -> SERVICE_OPEN
    let start_input = StartService {
        equipment_ready: true,
        rates_verified: true,
        rate_updated: "2026-10-11T08:00:00Z".to_string(),
    };
    service
        .start_service(&mut tx, &bm, &branch, start_input, now())
        .await
        .expect("start_service");

    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ops_events WHERE branch_id = $1 AND kind = $2")
            .bind(branch.as_str())
            .bind("SERVICE_OPEN")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(count, 1, "exactly one SERVICE_OPEN event");

    // 2. set_target -> CASH_TARGET
    let amounts = Balances::of(&[(
        CurrencyCode::parse("USD").unwrap(),
        Money::parse("500.00").unwrap(),
    )]);
    service
        .set_target(&mut tx, &fin, &branch, amounts, now())
        .await
        .expect("set_target");

    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ops_events WHERE branch_id = $1 AND kind = $2")
            .bind(branch.as_str())
            .bind("CASH_TARGET")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(count, 1, "exactly one CASH_TARGET event");

    // 3. clock in -> CLOCK_IN
    service
        .clock(&mut tx, &teller, &branch, ClockDirection::In, now())
        .await
        .expect("clock in");

    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ops_events WHERE branch_id = $1 AND kind = $2")
            .bind(branch.as_str())
            .bind("CLOCK_IN")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(count, 1, "exactly one CLOCK_IN event");

    // repeated clock in does NOT add another CLOCK_IN event
    service
        .clock(&mut tx, &teller, &branch, ClockDirection::In, now())
        .await
        .expect("clock in repeat");
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ops_events WHERE branch_id = $1 AND kind = $2")
            .bind(branch.as_str())
            .bind("CLOCK_IN")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(count, 1, "still exactly one CLOCK_IN event");

    // 4. hold -> HOLD_CUSTOMER
    let hold_id = service
        .hold(
            &mut tx,
            &teller,
            &branch,
            "CUST-99",
            "Suspicious note",
            now(),
        )
        .await
        .expect("hold");

    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ops_events WHERE branch_id = $1 AND kind = $2")
            .bind(branch.as_str())
            .bind("HOLD_CUSTOMER")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(count, 1, "exactly one HOLD_CUSTOMER event");

    // 5. begin_draft -> 0 events
    let draft_input = BeginDraft {
        side: Side::Buy,
        currency: CurrencyCode::parse("USD").unwrap(),
        amount: Money::parse("100.00").unwrap(),
    };
    let total_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ops_events WHERE branch_id = $1")
            .bind(branch.as_str())
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    service
        .begin_draft(&mut tx, &teller, &branch, draft_input, now())
        .await
        .expect("begin_draft");
    let total_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ops_events WHERE branch_id = $1")
            .bind(branch.as_str())
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(total_after, total_before, "begin_draft emits no ops_events");

    // 6. release -> RELEASE_CUSTOMER
    service
        .release(&mut tx, &reviewer, &branch, &hold_id, "All good", now())
        .await
        .expect("release");

    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ops_events WHERE branch_id = $1 AND kind = $2")
            .bind(branch.as_str())
            .bind("RELEASE_CUSTOMER")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(count, 1, "exactly one RELEASE_CUSTOMER event");

    // 7. clock out -> CLOCK_OUT
    service
        .clock(&mut tx, &teller, &branch, ClockDirection::Out, now())
        .await
        .expect("clock out");

    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ops_events WHERE branch_id = $1 AND kind = $2")
            .bind(branch.as_str())
            .bind("CLOCK_OUT")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(count, 1, "exactly one CLOCK_OUT event");

    // 8. cut_off -> CUT_OFF
    service
        .cut_off(&mut tx, &bm, &branch, now())
        .await
        .expect("cut_off");

    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ops_events WHERE branch_id = $1 AND kind = $2")
            .bind(branch.as_str())
            .bind("CUT_OFF")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(count, 1, "exactly one CUT_OFF event");

    // Read all events from repository
    let repo = PgCustodyRepository::new(db.pool.clone());
    let _events = repo.list_events(&branch).await.unwrap();
    // In transaction, verify kinds in order
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT kind FROM ops_events WHERE branch_id = $1 ORDER BY at ASC")
            .bind(branch.as_str())
            .fetch_all(&mut *tx)
            .await
            .unwrap();
    assert_eq!(
        kinds,
        vec![
            "SERVICE_OPEN",
            "CASH_TARGET",
            "CLOCK_IN",
            "HOLD_CUSTOMER",
            "RELEASE_CUSTOMER",
            "CLOCK_OUT",
            "CUT_OFF",
        ]
    );

    tx.commit().await.unwrap();
    let events_committed = repo.list_events(&branch).await.unwrap();
    assert_eq!(events_committed.len(), 7);
}

/// Must-fix 2: begin_draft must refuse unless phase == Open, and outside the service window (OutsideServiceHours).
#[tokio::test]
async fn test_begin_draft_refusals_phase_and_service_window() {
    let db = TestDb::start().await;
    let branch = db.branch("BD_DRAFT_REF").await;
    let ports = Arc::new(TestPorts::default());
    let service = db.service(ports);

    seed_test_user(&db.pool, TELLER_ID, "Teller").await;
    let teller = actor_scoped(TELLER_ID, Role::Teller, &branch);
    seed_open_shift_for_teller(&db.pool, &branch, TELLER_ID).await;

    let draft_input = BeginDraft {
        side: Side::Buy,
        currency: CurrencyCode::parse("USD").unwrap(),
        amount: Money::parse("50.00").unwrap(),
    };

    // 1. Refuse when phase is READY
    set_branch_phase(&db.pool, &branch, Phase::Ready).await;
    let mut tx = locked_tx(&db.pool, &branch).await;
    let err = service
        .begin_draft(&mut tx, &teller, &branch, draft_input.clone(), now())
        .await
        .unwrap_err();
    let pos_error::AppError::Domain(code) = err else {
        panic!("expected Domain error");
    };
    assert_eq!(
        code,
        CustodyError::NotCustodianInOpenService.to_string(),
        "refuse begin_draft when phase is READY"
    );
    tx.rollback().await.unwrap();

    // 2. Refuse when phase is CUT_OFF
    set_branch_phase(&db.pool, &branch, Phase::CutOff).await;
    let mut tx = locked_tx(&db.pool, &branch).await;
    let err = service
        .begin_draft(&mut tx, &teller, &branch, draft_input.clone(), now())
        .await
        .unwrap_err();
    let pos_error::AppError::Domain(code) = err else {
        panic!("expected Domain error");
    };
    assert_eq!(
        code,
        CustodyError::NotCustodianInOpenService.to_string(),
        "refuse begin_draft when phase is CUT_OFF"
    );
    tx.rollback().await.unwrap();

    // 3. Refuse when outside service window
    set_branch_phase(&db.pool, &branch, Phase::Open).await;
    let closed_ports = Arc::new(TestPorts {
        allows_begin: false,
        ..Default::default()
    });
    let closed_service = db.service(closed_ports);
    let mut tx = locked_tx(&db.pool, &branch).await;
    let err = closed_service
        .begin_draft(&mut tx, &teller, &branch, draft_input.clone(), now())
        .await
        .unwrap_err();
    let pos_error::AppError::Domain(code) = err else {
        panic!("expected Domain error");
    };
    assert_eq!(
        code,
        CustodyError::OutsideServiceHours.to_string(),
        "refuse begin_draft when outside service window"
    );
    tx.rollback().await.unwrap();

    // 4. Allowed when phase is Open and inside service window
    let mut tx = locked_tx(&db.pool, &branch).await;
    let ok = service
        .begin_draft(&mut tx, &teller, &branch, draft_input.clone(), now())
        .await;
    assert!(ok.is_ok(), "allowed when Open and inside service window");
    tx.rollback().await.unwrap();
}

/// Must-fix 3: cut the hold reason and the release note to 500 characters with cut_to(…, 500).
#[tokio::test]
async fn test_hold_reason_and_release_note_cut_to_500() {
    let db = TestDb::start().await;
    let branch = db.branch("BD_CUT_500").await;
    let ports = Arc::new(TestPorts::default());
    let service = db.service(ports);

    seed_test_user(&db.pool, TELLER_ID, "Teller").await;
    seed_test_user(&db.pool, REVIEWER_ID, "Reviewer").await;
    let teller = actor_scoped(TELLER_ID, Role::Teller, &branch);
    let reviewer = actor_scoped(REVIEWER_ID, Role::VerificationReviewer, &branch);

    set_branch_phase(&db.pool, &branch, Phase::Open).await;

    let long_reason = "A".repeat(750);
    let long_note = "B".repeat(600);

    let mut tx = locked_tx(&db.pool, &branch).await;
    let hold_id = service
        .hold(&mut tx, &teller, &branch, "CUST-LONG", &long_reason, now())
        .await
        .expect("hold");

    let repo = PgCustodyRepository::new(db.pool.clone());
    let state = repo.load_state_in(&mut tx, &branch).await.unwrap();
    let hold = state.holds.iter().find(|h| h.id == hold_id).expect("hold");
    assert_eq!(
        hold.reason.chars().count(),
        500,
        "hold reason must be truncated to 500 chars"
    );
    assert_eq!(hold.reason, "A".repeat(500));

    let stored_reason: String =
        sqlx::query_scalar("SELECT data->>'reason' FROM customer_holds WHERE id = $1")
            .bind(&hold_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(stored_reason.chars().count(), 500);
    assert_eq!(stored_reason, "A".repeat(500));

    service
        .release(&mut tx, &reviewer, &branch, &hold_id, &long_note, now())
        .await
        .expect("release");

    let state = repo.load_state_in(&mut tx, &branch).await.unwrap();
    let hold = state.holds.iter().find(|h| h.id == hold_id).expect("hold");
    assert_eq!(
        hold.note.as_deref().unwrap().chars().count(),
        500,
        "release note must be truncated to 500 chars"
    );
    assert_eq!(hold.note.as_deref().unwrap(), "B".repeat(500));

    let stored_note: String =
        sqlx::query_scalar("SELECT data->>'note' FROM customer_holds WHERE id = $1")
            .bind(&hold_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(stored_note.chars().count(), 500);
    assert_eq!(stored_note, "B".repeat(500));

    tx.commit().await.unwrap();
}

/// Must-fix (round 2): cut_off must propagate errors from outside_facts and active_in rather than swallowing them.
/// Make the read fail, assert that cut_off returns the error, and that no cut-off row is written.
#[tokio::test]
async fn test_cut_off_propagates_read_errors() {
    let db = TestDb::start().await;
    let branch = db.branch("BD_CUTOFF_ERR").await;
    let ports = Arc::new(TestPorts {
        fail_outside_facts: true,
        ..Default::default()
    });
    let service = db.service(ports);

    seed_test_user(&db.pool, MANAGER_ID, "Manager").await;
    let bm = actor_scoped(MANAGER_ID, Role::BranchManager, &branch);

    set_branch_phase(&db.pool, &branch, Phase::Open).await;

    let mut tx = locked_tx(&db.pool, &branch).await;

    let res = service.cut_off(&mut tx, &bm, &branch, now()).await;
    assert!(
        res.is_err(),
        "cut_off must return error when outside_facts fails"
    );
    let pos_error::AppError::Domain(msg) = res.unwrap_err() else {
        panic!("expected Domain error");
    };
    assert_eq!(msg, "outside_facts read failed");

    // Assert that no cut-off event or state was written
    let event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ops_events WHERE branch_id = $1 AND kind = 'CUT_OFF'",
    )
    .bind(branch.as_str())
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(
        event_count, 0,
        "no CUT_OFF event must be written on failure"
    );

    let cutoff_state: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT cutoff FROM ops_state WHERE branch_id = $1")
            .bind(branch.as_str())
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert!(
        cutoff_state.is_none(),
        "ops_state.cutoff must not be written on failure"
    );

    let phase: String = sqlx::query_scalar("SELECT phase FROM ops_state WHERE branch_id = $1")
        .bind(branch.as_str())
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(phase, "OPEN", "phase must remain OPEN on failure");

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn test_teller_of_actor_in_resolves_users_id() {
    let db = TestDb::start().await;
    let branch = db.branch("teller_uid").await;
    let shifts = ShiftService::new(
        Arc::new(PgShiftRepository::new(db.pool.clone())),
        BranchTx::new(Transactor::new(db.pool.clone())),
    );
    seed_test_user(&db.pool, TELLER_ID, "Teller").await;

    let mut tx = locked_tx(&db.pool, &branch).await;
    let resolved = shifts
        .teller_of_actor_in(&mut tx, &branch, TELLER_ID)
        .await
        .expect("teller_of_actor_in succeeds");
    assert_eq!(resolved.as_deref(), Some(TELLER_ID));

    let unknown = shifts
        .teller_of_actor_in(&mut tx, &branch, "no-such-actor")
        .await
        .expect("unknown actor returns Ok(None)");
    assert_eq!(unknown, None);
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn test_teller_of_actor_in_does_not_resolve_a_non_user_id() {
    let db = TestDb::start().await;
    let branch = db.branch("teller_non_uid").await;
    let shifts = ShiftService::new(
        Arc::new(PgShiftRepository::new(db.pool.clone())),
        BranchTx::new(Transactor::new(db.pool.clone())),
    );
    seed_test_user(&db.pool, TELLER_ID, "front").await;

    let mut tx = locked_tx(&db.pool, &branch).await;
    let resolved = shifts
        .teller_of_actor_in(&mut tx, &branch, "front")
        .await
        .expect("non-user id returns Ok(None)");
    assert_eq!(resolved, None);
    tx.rollback().await.unwrap();
}

/// #3306 (BE controller ruling, 2026-10-20): clock-in records `resolve_role(who)`, the same rule custody
/// events use, `Other` included, so a person's attendance row and their CLOCK_IN event never disagree.
/// Provided test (up front); the implementer must not change it.
#[tokio::test]
async fn test_clock_in_records_the_role_its_event_records() {
    const RATES_ID: &str = "00000000-0000-4000-8000-000000000006";
    let db = TestDb::start().await;
    let branch = db.branch("BD_CLOCK_ROLE").await;
    let service = db.service(Arc::new(TestPorts::default()));

    let rows = [
        (MANAGER_ID, "Manager", Role::BranchManager, "MANAGER"),
        (TELLER_ID, "Teller", Role::Teller, "FRONT"),
        (RATES_ID, "Rates", Role::RateManager, "OTHER"),
    ];
    for (id, name, _, _) in rows {
        seed_test_user(&db.pool, id, name).await;
    }

    let mut tx = locked_tx(&db.pool, &branch).await;
    for (id, _, role, want) in rows {
        let who = actor_scoped(id, role, &branch);
        service
            .clock(&mut tx, &who, &branch, ClockDirection::In, now())
            .await
            .unwrap_or_else(|e| panic!("{role:?} clocks in: {e:?}"));

        let attendance: String = sqlx::query_scalar(
            "SELECT data->>'role' FROM attendance WHERE branch_id = $1 AND employee_id = $2",
        )
        .bind(branch.as_str())
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .expect("the attendance row");
        let event: String = sqlx::query_scalar(
            "SELECT role FROM ops_events WHERE branch_id = $1 AND actor = $2 AND kind = 'CLOCK_IN'",
        )
        .bind(branch.as_str())
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .expect("the CLOCK_IN event");

        assert_eq!(
            event, want,
            "{role:?}: the CLOCK_IN event's role (resolve_role)"
        );
        assert_eq!(attendance, want, "{role:?}: the attendance row's role");
        assert_eq!(
            attendance, event,
            "{role:?}: attendance and its event agree"
        );
    }
    tx.rollback().await.unwrap();
}
