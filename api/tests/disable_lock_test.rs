//! Ticket #157 — an employee can be disabled between the custody check and the closing of the
//! account. Written before the fix. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `POST /api/employees` `disable` asks custody whether the employee holds cash or has work
//! pending, and then closes the account — two steps, neither under the branch's lock
//! (`BranchTx`). A custody command of the same branch that is under way when the question is asked
//! can make the employee the custodian or a party of a request a moment later, and the account is
//! closed all the same. v2 took one lock for both.
//!
//! What these tests hold: while a custody command of the employee's branch is under way (a
//! `BranchTx::run` of the test that has stored its state and has not committed), a `disable`
//! of an employee of that branch **waits** for it; when the command has committed, the question
//! is asked again and answered by what the command left — the employee stays an employee, or the
//! account is closed once; never both. A `disable` of an employee of another branch does not
//! wait.
//!
//! No sleep orders anything: the test reads from the database who waits for whom
//! (`pg_blocking_pids`) and goes on only when the `disable` is seen waiting for the command's
//! backend, as `create_branch_race_test.rs` does (#117).
//!
//! They need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in `_test`);
//! without it they FAIL, none skips. They stand on the harness of `auth_http_test.rs` and on
//! ticket T11a-E (PR 152): the route table registers `PgEmployeeCustody`.
//!
//! Red before the fix: the `disable` is answered while the command is still under way
//! (tests 1–3 fail with "was answered while a custody command … was still under way"). The
//! fourth test holds before the fix and after it: it keeps the lock the employee's own branch's,
//! not everybody's.
//!
//! Re-expected for CA-10 #626 by its ticket author (claude/c4d72c), on the lane controller's ruling
//! Q3 (2026-10-15; `docs/design/blueprint/cash-shift-transfer.md` §6, "What CA-10 keeps"): the
//! custodian and the custody requests are gone, and `disable` is refused while the employee holds
//! an OPEN shift of their own (`shifts.teller_id`) or is the receiver of a PENDING hand-over
//! (`handovers.receiver_id`). The command under way now leaves one of those rows instead of a
//! custody state; the locking it pins, the sentence, the audit line and the version moves are
//! unchanged.

mod auth_http_common;
mod common;

use std::time::{Duration, Instant};

use auth_http_common::*;
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use serde_json::json;
use tokio::sync::oneshot;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

/// Upper bound for anything that must finish.
const MUST_FINISH: Duration = Duration::from_secs(30);
/// How often the database is asked who waits for whom.
const ASK_AGAIN: Duration = Duration::from_millis(10);

/// v2, `auth/v2_auth_http_cases.json` story "employees" steps 32, 34, 35 (as in `employee_custody_test.rs`).
const HOLDS: &str = "ส่งมอบเงินและงานค้างก่อนปิดบัญชี หากเข้าระบบไม่ได้ให้รีเซ็ตรหัสผ่าน";
const MAIN: &str = "MAIN";
const OTHER: &str = "B2";

// ── the app, the employees, the rows ─────────────────────────────────────

fn branch_tx(desk: &Desk) -> BranchTx {
    BranchTx::new(Transactor::new(desk.pool.clone()))
}

/// A second branch, and the version rows of both branches, committed — as on a server that has
/// lived. A lock that was only the creation of such a row would otherwise look like a lock.
async fn two_branches(desk: &Desk) {
    desk.branch(OTHER).await;
    sqlx::query(
        "INSERT INTO branch_versions (branch_id, version) VALUES ($1, 0), ($2, 0) \
         ON CONFLICT (branch_id) DO NOTHING",
    )
    .bind(MAIN)
    .bind(OTHER)
    .execute(&desk.pool)
    .await
    .expect("insert the version rows");
}

async fn create(
    app: &TestApp,
    desk: &Desk,
    owner: &str,
    username: &str,
    role: &str,
    branch: &str,
) -> String {
    let made = send(
        app,
        Call::post(
            "/api/employees",
            json!({ "action": "create", "payload": {
                "username": username, "name": username, "role": role, "branchId": branch,
                "password": "first-password-1" } }),
        )
        .owner(owner),
    )
    .await;
    assert_eq!(made.status, 200, "create {username}: {}", made.text);
    desk.employee_id(username).await
}

async fn disable(app: &TestApp, owner: &str, id: &str) -> Answer {
    send(
        app,
        Call::post(
            "/api/employees",
            json!({ "action": "disable", "payload": { "id": id } }),
        )
        .owner(owner),
    )
    .await
}

async fn is_active(desk: &Desk, id: &str) -> bool {
    sqlx::query_scalar("SELECT active FROM employees WHERE id = $1")
        .bind(id)
        .fetch_one(&desk.pool)
        .await
        .expect("the employee row")
}

/// How many `DISABLE` lines the audit trail holds for `id`.
async fn disable_lines(desk: &Desk, id: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM employee_audit WHERE subject = $1 AND action = 'DISABLE'",
    )
    .bind(id)
    .fetch_one(&desk.pool)
    .await
    .expect("read employee_audit")
}

fn refused(answer: &Answer, sentence: &str, why: &str) {
    assert_eq!(
        (answer.status, answer.error().as_str()),
        (400, sentence),
        "{why}: {}",
        answer.text
    );
    assert_eq!(answer.cache_control.as_deref(), Some("no-store"), "{why}");
}

fn saved(answer: &Answer, why: &str) {
    assert_eq!(answer.status, 200, "{why}: {}", answer.text);
    assert_eq!(answer.json(), json!({ "saved": true }), "{why}");
}

// ── what the commands under way leave behind ──────────────────────────────

/// What a command under way writes before it commits (CA-10, Q3).
#[derive(Clone)]
enum Leaves {
    /// Nothing that concerns anybody.
    Nothing,
    /// An OPEN shift whose teller is this `users.id`.
    OpenShiftOf(String),
    /// A PENDING hand-over to this `users.id`, out of an OPEN shift of `from` (a `users.id`).
    PendingHandoverTo { receiver: String, from: String },
}

/// The `users.id` the employee `employee_id` is mirrored to.
async fn user_of(desk: &Desk, employee_id: &str) -> String {
    sqlx::query_scalar("SELECT id::text FROM users WHERE legacy_employee_id = $1")
        .bind(employee_id)
        .fetch_one(&desk.pool)
        .await
        .unwrap_or_else(|e| panic!("the user of employee {employee_id}: {e}"))
}

async fn open_shift_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &str,
    teller: &str,
) -> Result<String, AppError> {
    let id = format!("SH-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, teller_id) \
         VALUES ($1, $2, 'OPEN', now(), $3, '', $1, $3::uuid)",
    )
    .bind(&id)
    .bind(branch)
    .bind(teller)
    .execute(&mut **tx)
    .await
    .map_err(AppError::Database)?;
    Ok(id)
}

async fn leave_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &str,
    leaves: Leaves,
) -> Result<(), AppError> {
    match leaves {
        Leaves::Nothing => Ok(()),
        Leaves::OpenShiftOf(teller) => open_shift_in(tx, branch, &teller).await.map(|_| ()),
        Leaves::PendingHandoverTo { receiver, from } => {
            let shift = open_shift_in(tx, branch, &from).await?;
            let org: uuid::Uuid =
                sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                    .bind(branch)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;
            let count: uuid::Uuid = sqlx::query_scalar(
                "INSERT INTO cash_counts (organization_id, branch_id, shift_id, kind, counted_by, counted_at, idempotency_key) \
                 VALUES ($1, $2, $3, 'HANDOVER_OUT', $4::uuid, now(), gen_random_uuid()::text) RETURNING id",
            )
            .bind(org)
            .bind(branch)
            .bind(&shift)
            .bind(&from)
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)?;
            sqlx::query(
                "INSERT INTO handovers (organization_id, branch_id, from_shift_id, count_id, receiver_id, status) \
                 VALUES ($1, $2, $3, $4, $5::uuid, 'PENDING')",
            )
            .bind(org)
            .bind(branch)
            .bind(&shift)
            .bind(count)
            .bind(&receiver)
            .execute(&mut **tx)
            .await
            .map_err(AppError::Database)?;
            Ok(())
        }
    }
}

// ── a custody command under way, and who waits for it ────────────────────

/// A command of `branch` that is under way: it has taken the branch's lock, written `leaves`,
/// told the test which database backend it is, and goes on to commit only when `release` fires.
async fn command_under_way(
    desk: &Desk,
    branch: &str,
    leaves: Leaves,
    ready: oneshot::Sender<i32>,
    release: oneshot::Receiver<()>,
) -> AppResult<()> {
    let id = BranchId::new(branch);
    let inner = branch.to_owned();
    branch_tx(desk)
        .run(&id, move |tx| {
            Box::pin(async move {
                leave_in(tx, &inner, leaves).await?;
                let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;
                let _ = ready.send(pid);
                let _ = release.await;
                Ok::<(), AppError>(())
            })
        })
        .await
}

/// The backends that wait for a lock `holder` holds, as the database sees it now.
async fn blocked_by(desk: &Desk, holder: i32) -> Vec<i32> {
    sqlx::query_scalar(
        "SELECT pid FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) ORDER BY pid",
    )
    .bind(holder)
    .fetch_all(&desk.pool)
    .await
    .expect("read pg_stat_activity")
}

/// Sends `disable` of `employee` while a custody command of `branch` is under way and has left
/// `leaves`; sees the `disable` come to wait for the command; lets the command commit; gives
/// back what the `disable` answered. Fails when the `disable` is answered before the command
/// has committed.
async fn disable_while_a_command_is_under_way(
    desk: &Desk,
    app: &TestApp,
    owner: &str,
    employee: &str,
    branch: &str,
    leaves: Leaves,
) -> Answer {
    let (ready_tx, ready_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let command = command_under_way(desk, branch, leaves, ready_tx, release_rx);
    let driver = async {
        let holder = ready_rx
            .await
            .expect("the command under way tells the test which backend it is");
        let mut call = Box::pin(disable(app, owner, employee));
        let started = Instant::now();
        loop {
            tokio::select! {
                answer = &mut call => panic!(
                    "the disable of {employee} was answered while a custody command of {branch} \
                     was still under way — it did not wait for the branch's lock: {} {}",
                    answer.status, answer.text
                ),
                () = tokio::time::sleep(ASK_AGAIN) => {}
            }
            if !blocked_by(desk, holder).await.is_empty() {
                break;
            }
            assert!(
                started.elapsed() < MUST_FINISH,
                "nobody came to wait for backend {holder} within {MUST_FINISH:?}"
            );
        }
        release_tx
            .send(())
            .expect("the command under way is still waiting");
        tokio::time::timeout(MUST_FINISH, call)
            .await
            .expect("the disable did not finish after the command had committed")
    };
    let (committed, answer) = tokio::join!(command, driver);
    committed.expect("the command under way commits");
    answer
}

// ── the tests ────────────────────────────────────────────────────────────

/// 1. The command under way opens a shift the employee holds. The `disable` waits, is
///    then refused with v2's sentence, and leaves the employee active and no line in the audit
///    trail.
#[actix_web::test]
async fn a_disable_waits_for_the_command_that_opens_a_shift_the_employee_holds() {
    let desk = Desk::start().await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    two_branches(&desk).await;
    let ann = create(&app, &desk, &owner, "ann", "FRONT", MAIN).await;

    let leaves = Leaves::OpenShiftOf(user_of(&desk, &ann).await);
    let answer =
        disable_while_a_command_is_under_way(&desk, &app, &owner, &ann, MAIN, leaves).await;

    refused(
        &answer,
        HOLDS,
        "the employee came to hold an OPEN shift first",
    );
    assert!(
        is_active(&desk, &ann).await,
        "a refused disable closes nothing"
    );
    assert_eq!(
        disable_lines(&desk, &ann).await,
        0,
        "a refused disable writes no audit line"
    );
    desk.drop_schema().await;
}

/// 2. The command under way hands cash over to the employee, still PENDING. Same answer.
#[actix_web::test]
async fn a_disable_waits_for_the_command_that_hands_cash_over_to_the_employee() {
    let desk = Desk::start().await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    two_branches(&desk).await;
    let bea = create(&app, &desk, &owner, "bea", "BACKEND", MAIN).await;

    let sam = create(&app, &desk, &owner, "sam", "FRONT", MAIN).await;
    let leaves = Leaves::PendingHandoverTo {
        receiver: user_of(&desk, &bea).await,
        from: user_of(&desk, &sam).await,
    };
    let answer =
        disable_while_a_command_is_under_way(&desk, &app, &owner, &bea, MAIN, leaves).await;

    refused(
        &answer,
        HOLDS,
        "the employee became the receiver of a hand-over first",
    );
    assert!(
        is_active(&desk, &bea).await,
        "a refused disable closes nothing"
    );
    assert_eq!(disable_lines(&desk, &bea).await, 0);
    desk.drop_schema().await;
}

/// 3. The command under way has nothing to do with the employee. The `disable` still waits for
///    it — it is a write of the branch — and then closes the account, once, with one audit line.
#[actix_web::test]
async fn a_disable_waits_for_any_command_of_its_branch_and_then_closes_the_account_once() {
    let desk = Desk::start().await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    two_branches(&desk).await;
    let cy = create(&app, &desk, &owner, "cyd", "MANAGER", MAIN).await;

    let before = branch_tx(&desk)
        .version(&BranchId::new(MAIN))
        .await
        .expect("read the version");
    let answer =
        disable_while_a_command_is_under_way(&desk, &app, &owner, &cy, MAIN, Leaves::Nothing).await;

    saved(&answer, "nobody holds anything");
    assert!(!is_active(&desk, &cy).await, "the account is closed");
    assert_eq!(
        disable_lines(&desk, &cy).await,
        1,
        "exactly one DISABLE line"
    );
    let after = branch_tx(&desk)
        .version(&BranchId::new(MAIN))
        .await
        .expect("read the version");
    assert_eq!(
        after,
        before + 2,
        "the command under way and the disable are two writes of the branch"
    );
    desk.drop_schema().await;
}

/// 4. Holds before the fix and after it: a `disable` of an employee of another branch does not
///    wait for the command under way in `MAIN`.
#[actix_web::test]
async fn a_disable_in_another_branch_does_not_wait() {
    let desk = Desk::start().await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    two_branches(&desk).await;
    let dee = create(&app, &desk, &owner, "dee", "FRONT", OTHER).await;

    let (ready_tx, ready_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let command = command_under_way(&desk, MAIN, Leaves::Nothing, ready_tx, release_rx);
    let driver = async {
        ready_rx
            .await
            .expect("the command under way tells the test which backend it is");
        let answer = tokio::time::timeout(MUST_FINISH, disable(&app, &owner, &dee))
            .await
            .expect("a disable in B2 waited for a command of MAIN");
        release_tx
            .send(())
            .expect("the command under way is still waiting");
        answer
    };
    let (committed, answer) = tokio::join!(command, driver);
    committed.expect("the command under way commits");

    saved(&answer, "another branch's lock is not this one's");
    assert!(!is_active(&desk, &dee).await);
    desk.drop_schema().await;
}
