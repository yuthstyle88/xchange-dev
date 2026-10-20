//! CA-7b (#1273, design `cash-shift-transfer.md` §4.8, §9.1 CA-7b row):
//! Up-front tests for `BranchDayService` on `access::Actor`.
//!
//! Written before the implementation by the test-writer (gemini/studio-13, Rule A); moved to the
//! caller's-transaction signatures of §4.9 by claude/709ec749 on the BE lane controller
//! claude/ad05bf8d's order (#1273). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! §4.9: every method takes the caller's `&mut Transaction` first, does all its work on it, opens
//! no transaction and takes no lock of its own, and never commits. Each call below runs the way
//! the CM-1 seam runs a command (`in_tx!`): a transaction that already holds the branch lock
//! (`branch_versions … FOR UPDATE`), committed on `Ok`, rolled back on `Err`.
//! Tests:
//! 1. each method refuses an actor without its §3.2 permission, with no role literal in the code under test:
//!    the refusal carries `Authorizer::check`'s code (`Refusal::RoleNotAllowed`, `roleNotAllowed`), and
//!    each refused call is otherwise valid (phase, an OPEN hold, an OPEN shift of the actor's own);
//! 2. start-service only from READY, and cut-off only from OPEN, through BranchDay;
//! 3. the target, attendance, hold and draft rows are written on the branch transaction;
//! 4. a release needs an open hold and a note;
//! 5. clock-out needs a clock-in;
//! 6. begin_draft needs ShiftOperate and only works on the teller's own OPEN shift;
//! 7. every method finishes on the caller's locked transaction (no self-wait), its rows visible there;
//! 8. rolling the caller's transaction back leaves no rows.

mod custody_common;
mod custody_service_common;
mod shift_service_common;

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{PgPool, Postgres, Transaction};
use xc_auth::domain::access::{Actor, Refusal, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Side};
use xc_shift::application::branch_day::service::{
    BeginDraft, BranchDayService, ClockDirection, StartService,
};
use xc_shift::domain::custody::entities::customer_hold::HoldStatus;
use xc_shift::domain::custody::entities::tx_draft::DraftStatus;
use xc_shift::domain::custody::repositories::custody_ports::{CustodyPorts, OutsideFacts};
use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use shift_service_common::TestDb;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const MANAGER_ID: &str = "00000000-0000-4000-8000-000000000001";
const TELLER_ID: &str = "00000000-0000-4000-8000-000000000002";
const BOB_TELLER_ID: &str = "00000000-0000-4000-8000-000000000003";
const FINANCE_ID: &str = "00000000-0000-4000-8000-000000000004";
const REVIEWER_ID: &str = "00000000-0000-4000-8000-000000000005";
const CUSTOMER_ID: &str = "00000000-0000-4000-8000-000000000006";

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 11, 8, 0, 0).unwrap()
}

fn actor_scoped(user_id: &str, role: Role, branch: &BranchId) -> Actor {
    Actor::new(
        user_id,
        ORG,
        vec![ScopedRole::new(
            role,
            Some(branch.clone()),
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            None,
        )],
        None,
    )
}

struct TestPorts;

impl CustodyPorts for TestPorts {
    fn outside_facts<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _open_shift_id: Option<&'a str>,
    ) -> BoxFuture<'a, AppResult<OutsideFacts>> {
        Box::pin(async {
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

    fn employee_role<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _employee_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<CustodyRole>>> {
        Box::pin(async { Ok(Some(CustodyRole::Front)) })
    }

    fn service_window<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _now: CustodyTime,
    ) -> BoxFuture<'a, AppResult<ServiceWindow>> {
        Box::pin(async {
            Ok(ServiceWindow::Open {
                open: CustodyTime::from_epoch_millis(0),
                close: CustodyTime::from_epoch_millis(i64::MAX / 2),
            })
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

fn make_service(db: &TestDb) -> BranchDayService<PgCustodyRepository, PgShiftRepository> {
    let repo = Arc::new(PgCustodyRepository::new(db.pool.clone()));
    let shifts = db.service();
    let ports = Arc::new(TestPorts);
    BranchDayService::new(repo, shifts, ports)
}

/// Opens a transaction and locks the branch exactly as the CM-1 seam does before a command runs
/// (`BranchTx::run`, `xc-kernel/src/branch_tx.rs`): the `branch_versions` row, then `FOR UPDATE`.
async fn locked_tx(pool: &PgPool, branch: &BranchId) -> Transaction<'static, Postgres> {
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query(
        "INSERT INTO branch_versions (branch_id, version) VALUES ($1, 0) ON CONFLICT (branch_id) DO NOTHING",
    )
    .bind(branch.as_str())
    .execute(&mut *tx)
    .await
    .expect("branch_versions row");
    sqlx::query("SELECT version FROM branch_versions WHERE branch_id = $1 FOR UPDATE")
        .bind(branch.as_str())
        .execute(&mut *tx)
        .await
        .expect("lock the branch");
    tx
}

/// How long a call may take on the caller's locked transaction before the test calls it a
/// self-wait. A method that opened its own `BranchTx` (or any second connection) and asked for the
/// branch lock the caller already holds would wait for the caller forever.
const NO_SELF_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Runs one service call the way the seam runs a command (design §4.9): on a transaction that
/// already holds the branch lock, then commits on `Ok` and rolls back on `Err`. The method itself
/// opens no transaction, takes no lock, and never commits.
macro_rules! in_tx {
    ($db:expr, $branch:expr, |$tx:ident| $call:expr) => {{
        let mut seam = locked_tx(&$db.pool, &$branch).await;
        let result = {
            let $tx = &mut seam;
            tokio::time::timeout(NO_SELF_WAIT, $call)
                .await
                .expect("the call waited on a lock its caller already holds (§4.9)")
        };
        if result.is_ok() {
            seam.commit().await.expect("commit");
        } else {
            seam.rollback().await.expect("rollback");
        }
        result
    }};
}

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
         VALUES ($1, $2, 'test', now(), 'OPEN', $3, $1)",
    )
    .bind(&shift_id)
    .bind(branch.as_str())
    .bind(teller_uuid)
    .execute(pool)
    .await
    .expect("insert open shift");
    shift_id
}

async fn set_branch_phase(pool: &PgPool, branch: &BranchId, phase: Phase) {
    let repo = PgCustodyRepository::new(pool.clone());
    let mut tx = pool.begin().await.unwrap();
    let before = repo.load_state_in(&mut tx, branch).await.unwrap();
    let mut after = before.clone();
    // CA-10 (#626, #2955): no `enabled` switch any more; the checks always apply (Q2).
    after.phase = phase;
    repo.store_state_in(&mut tx, branch, &before, &after)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

/// The code `Authorizer::check` refuses a missing permission with: `Refusal::RoleNotAllowed`
/// (`roleNotAllowed`, `xc-auth/src/domain/access/authorizer.rs`). The refusal must carry it, so a
/// call refused for another reason (an unknown hold, no open shift of one's own, the wrong phase)
/// cannot pass for a permission refusal.
fn assert_refused_for_permission<T: std::fmt::Debug>(result: AppResult<T>, what: &str) {
    let code = Refusal::RoleNotAllowed.code();
    match result {
        Ok(v) => panic!("{what}: expected a `{code}` refusal, got Ok({v:?})"),
        Err(e) => assert!(
            e.to_string().contains(code),
            "{what}: refused with `{e}`, not with `{code}`"
        ),
    }
}

/// 1. Each method refuses an actor without its §3.2 permission (no role literal in code under test).
#[tokio::test]
async fn each_method_refuses_an_actor_without_its_permission() {
    let db = TestDb::start().await;
    let branch = db.branch("BD_PERM").await;
    let service = make_service(&db);

    seed_test_user(&db.pool, MANAGER_ID, "BranchManager").await;
    seed_test_user(&db.pool, TELLER_ID, "TellerAlice").await;
    seed_test_user(&db.pool, FINANCE_ID, "FinanceBob").await;
    seed_test_user(&db.pool, REVIEWER_ID, "ReviewerCarol").await;
    seed_test_user(&db.pool, CUSTOMER_ID, "CustomerDave").await;

    let bm = actor_scoped(MANAGER_ID, Role::BranchManager, &branch);
    let teller = actor_scoped(TELLER_ID, Role::Teller, &branch);
    let fin = actor_scoped(FINANCE_ID, Role::Finance, &branch);
    let reviewer = actor_scoped(REVIEWER_ID, Role::VerificationReviewer, &branch);
    let customer = actor_scoped(CUSTOMER_ID, Role::Customer, &branch);

    let start_input = StartService {
        equipment_ready: true,
        rates_verified: true,
        rate_updated: "2026-10-11T08:00:00Z".to_string(),
    };
    let draft_input = BeginDraft {
        side: Side::Buy,
        currency: CurrencyCode::parse("USD").unwrap(),
        amount: Money::parse("100.00").unwrap(),
    };
    let target_bal = Balances::of(&[(
        CurrencyCode::parse("USD").unwrap(),
        Money::parse("1000.00").unwrap(),
    )]);

    let repo = PgCustodyRepository::new(db.pool.clone());

    // start_service needs BranchDayOperate (held by BranchManager; Teller and Customer lack it).
    // The branch is READY, so nothing but the permission can refuse them, and it stays READY.
    set_branch_phase(&db.pool, &branch, Phase::Ready).await;
    assert_refused_for_permission(
        in_tx!(db, branch, |tx| service.start_service(
            tx,
            &teller,
            &branch,
            start_input.clone(),
            now()
        )),
        "start_service as TELLER",
    );
    assert_refused_for_permission(
        in_tx!(db, branch, |tx| service.start_service(
            tx,
            &customer,
            &branch,
            start_input.clone(),
            now()
        )),
        "start_service as CUSTOMER",
    );
    assert_eq!(
        repo.load_state(&branch).await.unwrap().phase,
        Phase::Ready,
        "a refused start-service leaves the branch READY"
    );

    // cut_off needs BranchDayOperate (held by BranchManager; Teller and Customer lack it).
    // The branch is OPEN, so nothing but the permission can refuse them, and it stays OPEN.
    set_branch_phase(&db.pool, &branch, Phase::Open).await;
    assert_refused_for_permission(
        in_tx!(db, branch, |tx| service.cut_off(
            tx,
            &teller,
            &branch,
            now()
        )),
        "cut_off as TELLER",
    );
    assert_refused_for_permission(
        in_tx!(db, branch, |tx| service.cut_off(
            tx,
            &customer,
            &branch,
            now()
        )),
        "cut_off as CUSTOMER",
    );
    assert_eq!(
        repo.load_state(&branch).await.unwrap().phase,
        Phase::Open,
        "a refused cut-off leaves the branch OPEN"
    );

    // set_target needs CashTargetSet (held by Finance; Teller and BranchManager lack it)
    assert_refused_for_permission(
        in_tx!(db, branch, |tx| service.set_target(
            tx,
            &teller,
            &branch,
            target_bal.clone(),
            now()
        )),
        "set_target as TELLER",
    );
    assert_refused_for_permission(
        in_tx!(db, branch, |tx| service.set_target(
            tx,
            &bm,
            &branch,
            target_bal.clone(),
            now()
        )),
        "set_target as BRANCH_MANAGER",
    );

    // clock needs AttendanceClock (held by staff roles; Customer lacks it)
    assert_refused_for_permission(
        in_tx!(db, branch, |tx| service.clock(
            tx,
            &customer,
            &branch,
            ClockDirection::In,
            now()
        )),
        "clock as CUSTOMER",
    );

    // hold needs CustomerHoldPlace (held by Teller/BranchManager; Customer and Finance lack it)
    assert_refused_for_permission(
        in_tx!(db, branch, |tx| service.hold(
            tx,
            &customer,
            &branch,
            "CUST-1",
            "AML check",
            now()
        )),
        "hold as CUSTOMER",
    );
    assert_refused_for_permission(
        in_tx!(db, branch, |tx| service.hold(
            tx,
            &fin,
            &branch,
            "CUST-1",
            "AML check",
            now()
        )),
        "hold as FINANCE",
    );

    // release needs VerificationReview (held by VerificationReviewer and BranchManager under §11.2; Finance and Teller lack it).
    // They name a real OPEN hold with a note, so nothing but the permission can refuse them, and
    // the hold stays OPEN.
    let hold_id = in_tx!(db, branch, |tx| service.hold(
        tx,
        &teller,
        &branch,
        "CUST-1",
        "AML check",
        now()
    ))
    .expect("a TELLER places the hold the refused releases name");
    assert_refused_for_permission(
        in_tx!(db, branch, |tx| service.release(
            tx,
            &fin,
            &branch,
            &hold_id,
            "ok note",
            now()
        )),
        "release as FINANCE",
    );
    assert_refused_for_permission(
        in_tx!(db, branch, |tx| service.release(
            tx,
            &teller,
            &branch,
            &hold_id,
            "ok note",
            now()
        )),
        "release as TELLER",
    );
    assert!(
        repo.load_state(&branch)
            .await
            .unwrap()
            .holds
            .iter()
            .any(|h| h.id == hold_id && h.status == HoldStatus::Open),
        "a refused release leaves the hold OPEN"
    );

    // begin_draft needs ShiftOperate (held by Teller/BranchManager; Customer, Finance, and Reviewer lack it).
    // Each refused actor has an OPEN shift of its own at this branch, so the own-shift rule cannot
    // refuse them first.
    for id in [FINANCE_ID, REVIEWER_ID, CUSTOMER_ID] {
        seed_open_shift_for_teller(&db.pool, &branch, id).await;
    }
    assert_refused_for_permission(
        in_tx!(db, branch, |tx| service.begin_draft(
            tx,
            &fin,
            &branch,
            draft_input.clone(),
            now()
        )),
        "begin_draft as FINANCE",
    );
    assert_refused_for_permission(
        in_tx!(db, branch, |tx| service.begin_draft(
            tx,
            &reviewer,
            &branch,
            draft_input.clone(),
            now()
        )),
        "begin_draft as VERIFICATION_REVIEWER",
    );
    assert_refused_for_permission(
        in_tx!(db, branch, |tx| service.begin_draft(
            tx,
            &customer,
            &branch,
            draft_input.clone(),
            now()
        )),
        "begin_draft as CUSTOMER",
    );
}

/// 2. start-service only from READY, and cut-off only from OPEN, through BranchDay.
#[tokio::test]
async fn start_service_only_from_ready_and_cut_off_only_from_open_through_branch_day() {
    let db = TestDb::start().await;
    let branch = db.branch("BD_PHASE").await;
    let service = make_service(&db);

    seed_test_user(&db.pool, MANAGER_ID, "BranchManager").await;
    let bm = actor_scoped(MANAGER_ID, Role::BranchManager, &branch);

    let start_input = StartService {
        equipment_ready: true,
        rates_verified: true,
        rate_updated: "2026-10-11T08:00:00Z".to_string(),
    };

    // Phase: Closed
    set_branch_phase(&db.pool, &branch, Phase::Closed).await;
    assert!(
        in_tx!(db, branch, |tx| service.start_service(
            tx,
            &bm,
            &branch,
            start_input.clone(),
            now()
        ))
        .is_err()
    );
    assert!(in_tx!(db, branch, |tx| service.cut_off(tx, &bm, &branch, now())).is_err());

    // Phase: Ready -> cut-off must be refused, start_service must succeed and transition to Open
    set_branch_phase(&db.pool, &branch, Phase::Ready).await;
    assert!(in_tx!(db, branch, |tx| service.cut_off(tx, &bm, &branch, now())).is_err());

    let phase = in_tx!(db, branch, |tx| service.start_service(
        tx,
        &bm,
        &branch,
        start_input.clone(),
        now()
    ))
    .expect("start_service from Ready must succeed");
    assert_eq!(phase, Phase::Open);

    // Verify persisted in ops_state
    let repo = PgCustodyRepository::new(db.pool.clone());
    let state = repo.load_state(&branch).await.unwrap();
    assert_eq!(state.phase, Phase::Open);

    // Phase: Open -> start_service must be refused, cut_off must succeed and transition to CutOff
    assert!(
        in_tx!(db, branch, |tx| service.start_service(
            tx,
            &bm,
            &branch,
            start_input.clone(),
            now()
        ))
        .is_err()
    );

    let phase = in_tx!(db, branch, |tx| service.cut_off(tx, &bm, &branch, now()))
        .expect("cut_off from Open must succeed");
    assert_eq!(phase, Phase::CutOff);

    let state = repo.load_state(&branch).await.unwrap();
    assert_eq!(state.phase, Phase::CutOff);

    // Phase: CutOff -> cut_off must be refused
    assert!(in_tx!(db, branch, |tx| service.cut_off(tx, &bm, &branch, now())).is_err());
}

/// 3. The target, attendance, hold and draft rows are written on the branch transaction.
#[tokio::test]
async fn target_attendance_hold_and_draft_rows_are_written_on_the_branch_transaction() {
    let db = TestDb::start().await;
    let branch = db.branch("BD_ROWS").await;
    let service = make_service(&db);

    seed_test_user(&db.pool, FINANCE_ID, "FinanceBob").await;
    seed_test_user(&db.pool, TELLER_ID, "TellerAlice").await;
    let fin = actor_scoped(FINANCE_ID, Role::Finance, &branch);
    let teller = actor_scoped(TELLER_ID, Role::Teller, &branch);

    set_branch_phase(&db.pool, &branch, Phase::Open).await;
    let shift_id = seed_open_shift_for_teller(&db.pool, &branch, TELLER_ID).await;

    // 1. set_target writes to ops_state
    let amounts = Balances::of(&[(
        CurrencyCode::parse("USD").unwrap(),
        Money::parse("7500.00").unwrap(),
    )]);
    in_tx!(db, branch, |tx| service.set_target(
        tx,
        &fin,
        &branch,
        amounts.clone(),
        now()
    ))
    .expect("set_target");

    let repo = PgCustodyRepository::new(db.pool.clone());
    let state = repo.load_state(&branch).await.unwrap();
    assert!(state.target.is_some());
    assert_eq!(state.target.as_ref().unwrap().amounts, amounts);

    let target_json: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT target FROM ops_state WHERE branch_id = $1")
            .bind(branch.as_str())
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(
        target_json.is_some(),
        "target column in ops_state must be written"
    );

    // 2. clock writes to attendance table
    in_tx!(db, branch, |tx| service.clock(
        tx,
        &teller,
        &branch,
        ClockDirection::In,
        now()
    ))
    .expect("clock in");

    let att_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM attendance WHERE branch_id = $1 AND employee_id = $2",
    )
    .bind(branch.as_str())
    .bind(TELLER_ID)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(att_count, 1, "attendance row must be written");

    // 3. hold writes to customer_holds table
    let hold_id = in_tx!(db, branch, |tx| service.hold(
        tx,
        &teller,
        &branch,
        "CUST-77",
        "High value verification",
        now(),
    ))
    .expect("hold");

    let hold_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM customer_holds WHERE id = $1 AND branch_id = $2")
            .bind(&hold_id)
            .bind(branch.as_str())
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(hold_count, 1, "customer_holds row must be written");

    // 4. begin_draft writes to tx_drafts table
    let draft_input = BeginDraft {
        side: Side::Buy,
        currency: CurrencyCode::parse("EUR").unwrap(),
        amount: Money::parse("250.00").unwrap(),
    };
    let draft_id = in_tx!(db, branch, |tx| service.begin_draft(
        tx,
        &teller,
        &branch,
        draft_input.clone(),
        now()
    ))
    .expect("begin_draft");

    let draft_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tx_drafts WHERE id = $1 AND branch_id = $2")
            .bind(&draft_id)
            .bind(branch.as_str())
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(draft_count, 1, "tx_drafts row must be written");

    // Verify through repository state as well
    let state = repo.load_state(&branch).await.unwrap();
    assert!(
        state
            .attendance
            .iter()
            .any(|a| a.employee_id == TELLER_ID && a.clock_out.is_none())
    );
    assert!(
        state
            .holds
            .iter()
            .any(|h| h.id == hold_id && h.status == HoldStatus::Open)
    );
    assert!(
        state
            .drafts
            .iter()
            .any(|d| d.id == draft_id && d.shift_id == shift_id && d.status == DraftStatus::Open)
    );
}

/// 4. A release needs an open hold and a note.
#[tokio::test]
async fn release_needs_an_open_hold_and_a_note() {
    let db = TestDb::start().await;
    let branch = db.branch("BD_REL").await;
    let service = make_service(&db);

    seed_test_user(&db.pool, TELLER_ID, "TellerAlice").await;
    seed_test_user(&db.pool, REVIEWER_ID, "ReviewerCarol").await;
    let teller = actor_scoped(TELLER_ID, Role::Teller, &branch);
    let reviewer = actor_scoped(REVIEWER_ID, Role::VerificationReviewer, &branch);

    set_branch_phase(&db.pool, &branch, Phase::Open).await;

    // Create an open hold
    let hold_id = in_tx!(db, branch, |tx| service.hold(
        tx,
        &teller,
        &branch,
        "CUST-99",
        "Passport mismatch",
        now()
    ))
    .expect("hold created");

    // Release with empty note must be refused
    assert!(
        in_tx!(db, branch, |tx| service.release(
            tx,
            &reviewer,
            &branch,
            &hold_id,
            "",
            now()
        ))
        .is_err(),
        "release with empty note must be refused"
    );

    // Release with whitespace-only note must be refused
    assert!(
        in_tx!(db, branch, |tx| service.release(
            tx,
            &reviewer,
            &branch,
            &hold_id,
            "   \t\n  ",
            now()
        ))
        .is_err(),
        "release with whitespace-only note must be refused"
    );

    // Release of non-existent hold must be refused
    assert!(
        in_tx!(db, branch, |tx| service.release(
            tx,
            &reviewer,
            &branch,
            "no-such-hold-id",
            "Valid review note",
            now()
        ))
        .is_err(),
        "release with non-existent hold id must be refused"
    );

    // Release with a valid note must succeed
    let release_time = now() + chrono::Duration::minutes(5);
    in_tx!(db, branch, |tx| service.release(
        tx,
        &reviewer,
        &branch,
        &hold_id,
        "Customer confirmed identity with passport",
        release_time,
    ))
    .expect("release with valid note must succeed");

    // Verify hold is now Released
    let repo = PgCustodyRepository::new(db.pool.clone());
    let state = repo.load_state(&branch).await.unwrap();
    let released_hold = state
        .holds
        .iter()
        .find(|h| h.id == hold_id)
        .expect("hold exists");
    assert_eq!(released_hold.status, HoldStatus::Released);
    assert_eq!(released_hold.reviewed_by.as_deref(), Some(REVIEWER_ID));
    assert_eq!(
        released_hold.note.as_deref(),
        Some("Customer confirmed identity with passport")
    );

    // Release on an already released hold must be refused
    assert!(
        in_tx!(db, branch, |tx| service.release(
            tx,
            &reviewer,
            &branch,
            &hold_id,
            "Another note",
            release_time
        ))
        .is_err(),
        "release on already released hold must be refused"
    );
}

/// 5. Clock-out needs a clock-in.
#[tokio::test]
async fn clock_out_needs_a_clock_in() {
    let db = TestDb::start().await;
    let branch = db.branch("BD_CLOCK").await;
    let service = make_service(&db);

    seed_test_user(&db.pool, TELLER_ID, "TellerAlice").await;
    let teller = actor_scoped(TELLER_ID, Role::Teller, &branch);

    // Clock out before any clock in must be refused
    assert!(
        in_tx!(db, branch, |tx| service.clock(
            tx,
            &teller,
            &branch,
            ClockDirection::Out,
            now()
        ))
        .is_err(),
        "clock-out without clock-in must be refused"
    );

    // Clock in succeeds
    in_tx!(db, branch, |tx| service.clock(
        tx,
        &teller,
        &branch,
        ClockDirection::In,
        now()
    ))
    .expect("clock-in must succeed");

    let repo = PgCustodyRepository::new(db.pool.clone());
    let state = repo.load_state(&branch).await.unwrap();
    let active_att = state
        .attendance
        .iter()
        .find(|a| a.employee_id == TELLER_ID && a.clock_out.is_none());
    assert!(active_att.is_some(), "must have an open attendance record");

    // Clock out succeeds
    let out_time = now() + chrono::Duration::hours(8);
    in_tx!(db, branch, |tx| service.clock(
        tx,
        &teller,
        &branch,
        ClockDirection::Out,
        out_time
    ))
    .expect("clock-out must succeed after clock-in");

    let state = repo.load_state(&branch).await.unwrap();
    let active_att = state
        .attendance
        .iter()
        .find(|a| a.employee_id == TELLER_ID && a.clock_out.is_none());
    assert!(
        active_att.is_none(),
        "must have no open attendance record after clock-out"
    );

    // Clock out again when not clocked in must be refused
    let out_time2 = out_time + chrono::Duration::minutes(1);
    assert!(
        in_tx!(db, branch, |tx| service.clock(
            tx,
            &teller,
            &branch,
            ClockDirection::Out,
            out_time2
        ))
        .is_err(),
        "second clock-out without a new clock-in must be refused"
    );
}

/// 6. begin_draft needs ShiftOperate and only works on the teller's own OPEN shift.
#[tokio::test]
async fn begin_draft_needs_shift_operate_and_only_works_on_tellers_own_open_shift() {
    let db = TestDb::start().await;
    let branch = db.branch("BD_DRAFT").await;
    let service = make_service(&db);

    seed_test_user(&db.pool, TELLER_ID, "TellerAlice").await;
    seed_test_user(&db.pool, BOB_TELLER_ID, "TellerBob").await;
    seed_test_user(&db.pool, FINANCE_ID, "FinanceCharlie").await;

    let alice = actor_scoped(TELLER_ID, Role::Teller, &branch);
    let bob = actor_scoped(BOB_TELLER_ID, Role::Teller, &branch);
    let fin = actor_scoped(FINANCE_ID, Role::Finance, &branch);

    set_branch_phase(&db.pool, &branch, Phase::Open).await;

    let draft_input = BeginDraft {
        side: Side::Buy,
        currency: CurrencyCode::parse("USD").unwrap(),
        amount: Money::parse("150.00").unwrap(),
    };

    // Role without ShiftOperate is refused. Finance has an OPEN shift of its own here, so the
    // own-shift rule cannot refuse it first.
    seed_open_shift_for_teller(&db.pool, &branch, FINANCE_ID).await;
    assert_refused_for_permission(
        in_tx!(db, branch, |tx| service.begin_draft(
            tx,
            &fin,
            &branch,
            draft_input.clone(),
            now()
        )),
        "finance actor lacks ShiftOperate and must be refused",
    );

    // Alice has ShiftOperate but NO open shift at this branch -> must be refused
    assert!(
        in_tx!(db, branch, |tx| service.begin_draft(
            tx,
            &alice,
            &branch,
            draft_input.clone(),
            now()
        ))
        .is_err(),
        "actor without open shift must be refused"
    );

    // Bob opens a shift at this branch. Alice STILL has no open shift of her own.
    let bob_shift_id = seed_open_shift_for_teller(&db.pool, &branch, BOB_TELLER_ID).await;

    // Bob can begin draft on his own shift
    let bob_draft_id = in_tx!(db, branch, |tx| service.begin_draft(
        tx,
        &bob,
        &branch,
        draft_input.clone(),
        now()
    ))
    .expect("bob can begin draft on his own shift");
    assert!(!bob_draft_id.is_empty());

    let repo = PgCustodyRepository::new(db.pool.clone());
    let state = repo.load_state(&branch).await.unwrap();
    let bob_draft = state
        .drafts
        .iter()
        .find(|d| d.id == bob_draft_id)
        .expect("bob draft found");
    assert_eq!(bob_draft.shift_id, bob_shift_id);
    assert_eq!(bob_draft.employee_id, BOB_TELLER_ID);

    // Alice tries to begin draft on Bob's shift -> must be refused (teller's OWN open shift only!)
    assert!(
        in_tx!(db, branch, |tx| service.begin_draft(
            tx,
            &alice,
            &branch,
            draft_input.clone(),
            now()
        ))
        .is_err(),
        "actor cannot begin draft on another teller's shift"
    );

    // Alice opens her own shift at this branch
    let alice_shift_id = seed_open_shift_for_teller(&db.pool, &branch, TELLER_ID).await;

    // Now Alice can begin draft on her own shift
    let draft_id = in_tx!(db, branch, |tx| service.begin_draft(
        tx,
        &alice,
        &branch,
        draft_input.clone(),
        now()
    ))
    .expect("alice can begin draft on her own open shift");

    // Verify draft is recorded on Alice's shift and employee id
    let repo = PgCustodyRepository::new(db.pool.clone());
    let state = repo.load_state(&branch).await.unwrap();
    let draft = state
        .drafts
        .iter()
        .find(|d| d.id == draft_id)
        .expect("draft found");
    assert_eq!(draft.shift_id, alice_shift_id);
    assert_eq!(draft.employee_id, TELLER_ID);
    assert_eq!(draft.side, Side::Buy);
    assert_eq!(draft.currency, CurrencyCode::parse("USD").unwrap());
    assert_eq!(draft.amount, Money::parse("150.00").unwrap());
    assert_eq!(draft.status, DraftStatus::Open);
}

/// One call on `tx`, failing the test if it does not finish within `NO_SELF_WAIT`.
macro_rules! on_tx {
    ($what:literal, $call:expr) => {
        tokio::time::timeout(NO_SELF_WAIT, $call)
            .await
            .expect(concat!(
                $what,
                " waited on a lock its caller already holds (§4.9)"
            ))
    };
}

/// 7. §4.9: every method works on the caller's transaction, which already holds the branch lock.
/// With `statement_timeout` 2 s on that transaction, each of the seven finishes, and each one's
/// rows read back on that same transaction (a method that wrote through its own connection
/// would not be visible here before its own commit, and could not commit under the held lock).
#[tokio::test]
async fn every_method_finishes_on_the_callers_locked_transaction_with_no_self_wait() {
    let db = TestDb::start().await;
    let branch = db.branch("BD_SEAM").await;
    let service = make_service(&db);

    for (id, name) in [
        (MANAGER_ID, "BranchManager"),
        (TELLER_ID, "TellerAlice"),
        (FINANCE_ID, "FinanceBob"),
        (REVIEWER_ID, "ReviewerCarol"),
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
    sqlx::query("SET LOCAL statement_timeout = '2s'")
        .execute(&mut *tx)
        .await
        .expect("statement_timeout");

    let start = StartService {
        equipment_ready: true,
        rates_verified: true,
        rate_updated: "2026-10-11T08:00:00Z".to_string(),
    };
    let phase = on_tx!(
        "start_service",
        service.start_service(&mut tx, &bm, &branch, start, now())
    )
    .expect("start_service");
    assert_eq!(phase, Phase::Open);

    let amounts = Balances::of(&[(
        CurrencyCode::parse("USD").unwrap(),
        Money::parse("500.00").unwrap(),
    )]);
    on_tx!(
        "set_target",
        service.set_target(&mut tx, &fin, &branch, amounts, now())
    )
    .expect("set_target");
    on_tx!(
        "clock",
        service.clock(&mut tx, &teller, &branch, ClockDirection::In, now())
    )
    .expect("clock in");
    let hold_id = on_tx!(
        "hold",
        service.hold(&mut tx, &teller, &branch, "CUST-5", "check", now())
    )
    .expect("hold");
    let draft = BeginDraft {
        side: Side::Sell,
        currency: CurrencyCode::parse("USD").unwrap(),
        amount: Money::parse("50.00").unwrap(),
    };
    let draft_id = on_tx!(
        "begin_draft",
        service.begin_draft(&mut tx, &teller, &branch, draft, now())
    )
    .expect("begin_draft");
    on_tx!(
        "release",
        service.release(
            &mut tx,
            &reviewer,
            &branch,
            &hold_id,
            "Identity confirmed",
            now()
        )
    )
    .expect("release: the hold written on this transaction is found on it");
    let phase = on_tx!("cut_off", service.cut_off(&mut tx, &bm, &branch, now())).expect("cut_off");
    assert_eq!(phase, Phase::CutOff);

    // every row is on the caller's transaction, before its commit
    let count = |n: i64, what: &str| assert_eq!(n, 1, "{what} on the caller's transaction");
    let target: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT target FROM ops_state WHERE branch_id = $1")
            .bind(branch.as_str())
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert!(target.is_some(), "the target on the caller's transaction");
    for (sql, what) in [
        (
            "SELECT count(*) FROM attendance WHERE branch_id = $1",
            "attendance",
        ),
        (
            "SELECT count(*) FROM customer_holds WHERE branch_id = $1",
            "hold",
        ),
        (
            "SELECT count(*) FROM tx_drafts WHERE branch_id = $1",
            "draft",
        ),
    ] {
        let n: i64 = sqlx::query_scalar(sql)
            .bind(branch.as_str())
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        count(n, what);
    }
    let phase_now: String = sqlx::query_scalar("SELECT phase FROM ops_state WHERE branch_id = $1")
        .bind(branch.as_str())
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        phase_now, "CUT_OFF",
        "the phase on the caller's transaction"
    );
    assert!(!draft_id.is_empty());
    tx.commit().await.expect("the caller commits");
}

/// 8. §4.9: a method never commits on its own. Rolling the caller's transaction back leaves no
/// row of target, attendance, hold or draft.
#[tokio::test]
async fn rolling_back_the_callers_transaction_leaves_no_rows() {
    let db = TestDb::start().await;
    let branch = db.branch("BD_ROLLBACK").await;
    let service = make_service(&db);

    seed_test_user(&db.pool, FINANCE_ID, "FinanceBob").await;
    seed_test_user(&db.pool, TELLER_ID, "TellerAlice").await;
    let fin = actor_scoped(FINANCE_ID, Role::Finance, &branch);
    let teller = actor_scoped(TELLER_ID, Role::Teller, &branch);

    set_branch_phase(&db.pool, &branch, Phase::Open).await;
    seed_open_shift_for_teller(&db.pool, &branch, TELLER_ID).await;

    let mut tx = locked_tx(&db.pool, &branch).await;
    let amounts = Balances::of(&[(
        CurrencyCode::parse("USD").unwrap(),
        Money::parse("500.00").unwrap(),
    )]);
    on_tx!(
        "set_target",
        service.set_target(&mut tx, &fin, &branch, amounts, now())
    )
    .expect("set_target");
    on_tx!(
        "clock",
        service.clock(&mut tx, &teller, &branch, ClockDirection::In, now())
    )
    .expect("clock in");
    on_tx!(
        "hold",
        service.hold(&mut tx, &teller, &branch, "CUST-6", "check", now())
    )
    .expect("hold");
    let draft = BeginDraft {
        side: Side::Buy,
        currency: CurrencyCode::parse("USD").unwrap(),
        amount: Money::parse("20.00").unwrap(),
    };
    on_tx!(
        "begin_draft",
        service.begin_draft(&mut tx, &teller, &branch, draft, now())
    )
    .expect("begin_draft");
    tx.rollback().await.expect("the caller rolls back");

    let target: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT target FROM ops_state WHERE branch_id = $1")
            .bind(branch.as_str())
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(target.is_none(), "no target survives the caller's rollback");
    for (sql, what) in [
        (
            "SELECT count(*) FROM attendance WHERE branch_id = $1",
            "attendance",
        ),
        (
            "SELECT count(*) FROM customer_holds WHERE branch_id = $1",
            "hold",
        ),
        (
            "SELECT count(*) FROM tx_drafts WHERE branch_id = $1",
            "draft",
        ),
    ] {
        let n: i64 = sqlx::query_scalar(sql)
            .bind(branch.as_str())
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(n, 0, "no {what} row survives the caller's rollback");
    }
}
