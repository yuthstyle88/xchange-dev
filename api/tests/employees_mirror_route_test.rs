//! Ticket #616 (OR-2) — the employees mirror through the route and the `DisableEmployee` workflow. Written before the
//! implementation by the ticket author (claude/e5ab6c). Do not edit, delete or weaken; add tests in a file of your
//! own.
//!
//! Design: `docs/design/blueprint/org-roles-branches.md` (#550, merged f0ec3ca) §9 item 2, "Mirroring": the
//! `employees` route (`api/src/presentation/http/employees.rs:43`) is the only writer of accounts until OR-5; a
//! create through it leaves one `users` row (`PENDING`, `legacy_employee_id` = the new id) with its one
//! `user_roles` row, mapped by §3.3; every disable reaches `disable_employee_in`, which sets
//! `users.status = 'DISABLED'` in the caller's transaction — the route's `DisableEmployee`
//! (`api/src/application/workflows/disable_employee.rs:74`) included. A refused disable (the employee holds cash)
//! changes no `users` row, because `DisableEmployee` rolls its transaction back.
//!
//! The repository half (`employees("disable")`, a rolled-back `disable_employee_in`, reset) is held in
//! `api/crates/xc-auth/tests/employees_mirror_test.rs`. This file is in the root crate because `DisableEmployee`
//! and the route table are.
//!
//! Every test drives the server's own route table (`bootstrap::configure`, harness of `auth_http_test.rs`) or the
//! real `DisableEmployee` with the real `PgEmployeeCustody`, as `employee_custody_test.rs` and
//! `disable_failure_under_lock_test.rs` do. They need a real PostgreSQL: `TEST_DATABASE_URL` naming a database
//! whose name ends in `_test`. Without it they FAIL; none skips.
//!
//! Signatures these tests assume: none new. They read the tables of §8 (`users`, `user_roles`) with ids as
//! `::text`.

mod auth_http_common;
mod common;

use std::sync::Arc;
use std::time::SystemTime;

use auth_http_common::*;
use pos_infra::transactions::Transactor;
use serde_json::json;
use xc_auth::application::account::service::AccountService;
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xchange::application::workflows::disable_employee::{DisableEmployee, DisableOutcome};
use xchange::infrastructure::custody::employee_custody::PgEmployeeCustody;
use xchange::presentation::http::employees::EmployeeCustody;

/// v2, `auth/v2_auth_http_cases.json` story "employees" steps 32, 34, 35 (as in `employee_custody_test.rs`).
const HOLDS: &str = "ส่งมอบเงินและงานค้างก่อนปิดบัญชี หากเข้าระบบไม่ได้ให้รีเซ็ตรหัสผ่าน";
const MAIN: &str = "MAIN";
const B2: &str = "B2";

// ── the route and the workflow ───────────────────────────────────────────────

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
                "username": username, "name": format!("คุณ {username}"), "role": role,
                "branchId": branch, "password": "first-password-1" } }),
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

fn branch_tx(desk: &Desk) -> BranchTx {
    BranchTx::new(Transactor::new(desk.pool.clone()))
}

fn accounts(desk: &Desk) -> AccountService<PgAccountRepository> {
    AccountService::new(
        Arc::new(PgAccountRepository::new(desk.pool.clone())),
        PasswordHasher::with_cost(8, 1, 1),
    )
}

/// Makes `holder` the custodian of `branch`, as the custody service would have left it.
/// The employee `holder` holds cash at `branch`: an OPEN shift of their own (`shifts.teller_id`, their
/// mirrored `users.id`). Re-expected for CA-10 #626 by its ticket author (claude/c4d72c, ruling Q3,
/// cash-shift-transfer.md §6): the custodian is gone, and holding an OPEN shift is what `disable` refuses.
async fn custodian(desk: &Desk, branch: &str, holder: &str) {
    let id = BranchId::new(branch);
    let branch = branch.to_owned();
    let holder = holder.to_owned();
    branch_tx(desk)
        .run(&id, move |tx| {
            Box::pin(async move {
                let user: String =
                    sqlx::query_scalar("SELECT id::text FROM users WHERE legacy_employee_id = $1")
                        .bind(&holder)
                        .fetch_one(&mut **tx)
                        .await
                        .map_err(pos_error::AppError::Database)?;
                let shift = format!("SH-{}", uuid::Uuid::new_v4());
                sqlx::query(
                    "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, teller_id) \
                     VALUES ($1, $2, 'OPEN', now(), $3, '', $1, $3::uuid)",
                )
                .bind(&shift)
                .bind(&branch)
                .bind(&user)
                .execute(&mut **tx)
                .await
                .map_err(pos_error::AppError::Database)?;
                Ok::<(), pos_error::AppError>(())
            })
        })
        .await
        .expect("open the holder's shift");
}

// ── reading the mirror ───────────────────────────────────────────────────────

/// (status, auth_subject, name, version) of every user mirrored from employee `id`.
async fn users_of(desk: &Desk, id: &str) -> Vec<(String, Option<String>, String, i32)> {
    sqlx::query_as(
        "SELECT status, auth_subject, name, version FROM users WHERE legacy_employee_id = $1",
    )
    .bind(id)
    .fetch_all(&desk.pool)
    .await
    .expect("read users")
}

/// (role_code, branch_id) of the user mirrored from employee `id`.
async fn roles_of(desk: &Desk, id: &str) -> Vec<(String, Option<String>)> {
    sqlx::query_as(
        "SELECT ur.role_code::text, ur.branch_id::text FROM user_roles ur \
         JOIN users u ON u.id = ur.user_id WHERE u.legacy_employee_id = $1 ORDER BY 1, 2",
    )
    .bind(id)
    .fetch_all(&desk.pool)
    .await
    .expect("read user_roles")
}

async fn status_of(desk: &Desk, id: &str) -> String {
    let rows = users_of(desk, id).await;
    assert_eq!(
        rows.len(),
        1,
        "exactly one user mirrors employee {id}: {rows:?}"
    );
    rows[0].0.clone()
}

async fn is_active(desk: &Desk, id: &str) -> bool {
    sqlx::query_scalar("SELECT active FROM employees WHERE id = $1")
        .bind(id)
        .fetch_one(&desk.pool)
        .await
        .expect("the employee row")
}

// ── the tests ────────────────────────────────────────────────────────────────

/// A create through `POST /api/employees` leaves one `PENDING` user (`legacy_employee_id` = the new id) with its
/// one role, mapped by §3.3, at the branch named.
#[actix_web::test]
async fn a_create_through_the_route_leaves_one_pending_user_with_its_role() {
    let desk = Desk::start().await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    desk.branch(B2).await;

    let ann = create(&app, &desk, &owner, "ann", "FRONT", MAIN).await;
    let ben = create(&app, &desk, &owner, "ben", "BACKEND", B2).await;
    let sys = create(&app, &desk, &owner, "sys", "SYSTEM", B2).await;

    let mirrored: Vec<(String, Option<String>, String)> = users_of(&desk, &ann)
        .await
        .into_iter()
        .map(|(status, subject, name, _)| (status, subject, name))
        .collect();
    assert_eq!(
        mirrored,
        vec![("PENDING".to_owned(), None, "คุณ ann".to_owned())],
        "one PENDING, unclaimed user with the employee's name"
    );
    assert_eq!(
        roles_of(&desk, &ann).await,
        vec![("TELLER".to_owned(), Some(MAIN.to_owned()))]
    );
    assert_eq!(
        roles_of(&desk, &ben).await,
        vec![("BRANCH_MANAGER".to_owned(), Some(B2.to_owned()))]
    );
    assert_eq!(
        roles_of(&desk, &sys).await,
        vec![("ADMIN".to_owned(), Some(B2.to_owned()))]
    );
    for id in [&ben, &sys] {
        assert_eq!(status_of(&desk, id).await, "PENDING");
    }
    drop(app);
    desk.drop_schema().await;
}

/// A disable through `POST /api/employees` runs `DisableEmployee` and leaves the user `DISABLED`; a second
/// employee's user does not change.
#[actix_web::test]
async fn a_disable_through_the_route_disables_the_user() {
    let desk = Desk::start().await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    let ann = create(&app, &desk, &owner, "ann", "FRONT", MAIN).await;
    let bob = create(&app, &desk, &owner, "bob", "FRONT", MAIN).await;
    let bob_before = users_of(&desk, &bob).await;

    let answer = disable(&app, &owner, &ann).await;
    assert_eq!(answer.status, 200, "disable ann: {}", answer.text);
    assert!(!is_active(&desk, &ann).await);
    assert_eq!(status_of(&desk, &ann).await, "DISABLED");
    assert_eq!(
        users_of(&desk, &bob).await,
        bob_before,
        "bob's user is untouched"
    );
    drop(app);
    desk.drop_schema().await;
}

/// The workflow itself, as the route calls it, with the real custody port: `Closed` → `DISABLED`.
#[actix_web::test]
async fn the_disable_employee_workflow_disables_the_user() {
    let desk = Desk::start().await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    let ann = create(&app, &desk, &owner, "ann", "FRONT", MAIN).await;
    let custody: Arc<dyn EmployeeCustody> = Arc::new(PgEmployeeCustody::new(desk.pool.clone()));

    let outcome = DisableEmployee::new(branch_tx(&desk))
        .run(
            &accounts(&desk),
            Some(custody),
            &BranchId::new(MAIN),
            &ann,
            "owner",
            SystemTime::now().into(),
        )
        .await
        .expect("the workflow runs");
    assert_eq!(outcome, DisableOutcome::Closed);
    assert_eq!(status_of(&desk, &ann).await, "DISABLED");
    drop(app);
    desk.drop_schema().await;
}

/// A disable refused because the employee holds cash changes no `users` row — neither through the workflow
/// (`HoldsCash`, its transaction rolled back) nor through the route.
#[actix_web::test]
async fn a_disable_refused_because_the_employee_holds_cash_changes_no_user() {
    let desk = Desk::start().await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    let holder = create(&app, &desk, &owner, "holder", "FRONT", MAIN).await;
    custodian(&desk, MAIN, &holder).await;
    let before = users_of(&desk, &holder).await;
    let roles_before = roles_of(&desk, &holder).await;
    assert_eq!(before.len(), 1, "the holder is mirrored: {before:?}");

    let custody: Arc<dyn EmployeeCustody> = Arc::new(PgEmployeeCustody::new(desk.pool.clone()));
    let outcome = DisableEmployee::new(branch_tx(&desk))
        .run(
            &accounts(&desk),
            Some(custody),
            &BranchId::new(MAIN),
            &holder,
            "owner",
            SystemTime::now().into(),
        )
        .await
        .expect("the workflow runs");
    assert_eq!(outcome, DisableOutcome::HoldsCash);
    assert_eq!(
        users_of(&desk, &holder).await,
        before,
        "the workflow's refusal"
    );

    let answer = disable(&app, &owner, &holder).await;
    assert_eq!(
        (answer.status, answer.error()),
        (400, HOLDS.to_owned()),
        "the route refuses: {}",
        answer.text
    );
    assert_eq!(
        users_of(&desk, &holder).await,
        before,
        "the route's refusal"
    );
    assert_eq!(roles_of(&desk, &holder).await, roles_before);
    assert!(is_active(&desk, &holder).await, "the employee stays active");
    drop(app);
    desk.drop_schema().await;
}
