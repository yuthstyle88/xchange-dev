//! Ticket #616 (OR-2) — the employees mirror at the repository: every `employees` write of the window between migration 2
//! and OR-7 is mirrored into `users` + `user_roles` in the same transaction. Written before the implementation by
//! the ticket author (claude/e5ab6c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/org-roles-branches.md` (#550, merged f0ec3ca) §9 item 2, "Mirroring":
//! - `create_employee` inserts the `users` row (`PENDING`, `legacy_employee_id = <new id>`) and its one
//!   `user_roles` row, mapped by §3.3, before the commit;
//! - `disable_employee_in` sets `users.status = 'DISABLED'` where `legacy_employee_id = id`, in the caller's tx;
//!   `employees("disable")` → `disable_employee` wraps it;
//! - `reset_employee_password` changes nothing in `users`.
//!
//! And §5.1: `users.version` moves on any role or status change.
//!
//! Follow-up decisions (PR 636, `docs/org-roles-or2-followups`):
//! 1. `create_employee` checks, in its transaction and before any insert, that `branch_id` names a `branches`
//!    row, and refuses with `AppError::Domain(NO_SUCH_BRANCH)`, the route's sentence
//!    (`xc-rate/src/domain/validation.rs:8`, used by `BranchService::require`). xc-auth does not depend on
//!    xc-rate, so the sentence text is asserted here.
//! 2. `create_owner` is mirrored: in its transaction it inserts the owner's `users` row (`PENDING`,
//!    `legacy_employee_id` NULL) with EXECUTIVE and ADMIN, both organisation-wide (`branch_id` NULL).
//!
//! The route's `DisableEmployee` workflow (root crate) is held in `api/tests/employees_mirror_route_test.rs`.
//!
//! These drive the real `AccountService` on the real `PgAccountRepository` (`account_common`). They need a real
//! PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test`. Without it they FAIL; none skips.
//!
//! Signatures these tests assume: none new. They call today's `AccountService::employees`,
//! `AccountService::disable_employee_in` and `AccountService::create_owner`, and read the tables of §8 (`users`, `user_roles`) with ids as `::text`.

mod account_common;

use account_common::{
    EMPLOYEE_DATA, EMPLOYEE_NOT_FOUND, Service, TestDb, at, create_employee, new_employee, refusal,
};
use serde_json::json;

/// The route's sentence for a branch that does not exist (`xc-rate/src/domain/validation.rs:8`).
const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";

/// §3.3 (B1).
const MAPPED: [(&str, &str); 9] = [
    ("FRONT", "TELLER"),
    ("BACKEND", "BRANCH_MANAGER"),
    ("MANAGER", "BRANCH_MANAGER"),
    ("RATES", "RATE_MANAGER"),
    ("FINANCE", "FINANCE"),
    ("SYSTEM", "ADMIN"),
    ("MONITOR", "VERIFICATION_REVIEWER"),
    ("DOCUMENTS", "VERIFICATION_REVIEWER"),
    ("DOCUMENT_REVIEW", "VERIFICATION_REVIEWER"),
];

/// (status, auth_subject, name, version, organization_id) of every user mirrored from employee `id`.
type UserRow = (String, Option<String>, String, i32, String);

async fn users_of(db: &TestDb, id: &str) -> Vec<UserRow> {
    sqlx::query_as(
        "SELECT status, auth_subject, name, version, organization_id::text FROM users \
         WHERE legacy_employee_id = $1",
    )
    .bind(id)
    .fetch_all(&db.pool)
    .await
    .expect("read users")
}

async fn the_user_of(db: &TestDb, id: &str) -> UserRow {
    let mut rows = users_of(db, id).await;
    assert_eq!(
        rows.len(),
        1,
        "exactly one user mirrors employee {id}: {rows:?}"
    );
    rows.remove(0)
}

/// (role_code, branch_id, organization_id, valid_to is NULL) of the user mirrored from employee `id`.
async fn roles_of(db: &TestDb, id: &str) -> Vec<(String, Option<String>, String, bool)> {
    sqlx::query_as(
        "SELECT ur.role_code::text, ur.branch_id::text, ur.organization_id::text, ur.valid_to IS NULL \
         FROM user_roles ur JOIN users u ON u.id = ur.user_id \
         WHERE u.legacy_employee_id = $1 ORDER BY 1, 2",
    )
    .bind(id)
    .fetch_all(&db.pool)
    .await
    .expect("read user_roles")
}

async fn organization(db: &TestDb) -> String {
    sqlx::query_scalar("SELECT id::text FROM organizations WHERE code = 'XC'")
        .fetch_one(&db.pool)
        .await
        .expect("OR-1's one organisation")
}

async fn employee(service: &Service, username: &str, role: &str) -> String {
    employee_at(service, username, role, "MAIN").await
}

async fn employee_at(service: &Service, username: &str, role: &str, branch: &str) -> String {
    create_employee(
        service,
        &new_employee(username, role, branch, "the-first-password"),
        at(0),
    )
    .await
    .id
}

// ── create ───────────────────────────────────────────────────────────────────

/// A create through `employees("create")` leaves one `PENDING` user whose `legacy_employee_id` is the new id,
/// with its one role, mapped by §3.3, at the employee's branch.
#[tokio::test]
async fn a_create_leaves_one_pending_user_with_its_one_mapped_role() {
    let db = TestDb::start().await;
    let service = db.service();
    let org = organization(&db).await;
    let users_before = db.count("users").await;

    for (v3, blueprint) in MAPPED {
        let username = format!("p-{}", v3.to_lowercase().replace('_', "-"));
        let id = employee(&service, &username, v3).await;
        let (status, subject, name, _, user_org) = the_user_of(&db, &id).await;
        assert_eq!(
            (status.as_str(), subject, name, user_org),
            ("PENDING", None, format!("คุณ {username}"), org.clone()),
            "{v3}: PENDING, unclaimed, the employee's name, the one organisation"
        );
        assert_eq!(
            roles_of(&db, &id).await,
            vec![(
                blueprint.to_owned(),
                Some("MAIN".to_owned()),
                org.clone(),
                true
            )],
            "{v3} → {blueprint} at MAIN, with no end"
        );
    }
    assert_eq!(
        db.count("users").await,
        users_before + MAPPED.len() as i64,
        "one user per create, nothing else"
    );
    db.drop_schema().await;
}

/// The mirror is in the create's transaction: a refused create (the username is taken) mirrors nothing.
#[tokio::test]
async fn a_refused_create_mirrors_nothing() {
    let db = TestDb::start().await;
    let service = db.service();
    employee(&service, "ann", "FRONT").await;
    let (users, roles) = (db.count("users").await, db.count("user_roles").await);

    let error = service
        .employees(
            "create",
            &new_employee("ann", "RATES", "MAIN", "the-first-password"),
            "owner",
            at(10),
        )
        .await
        .expect_err("the username is taken");
    assert_eq!(refusal(&error), EMPLOYEE_DATA);
    assert_eq!(
        db.count("users").await,
        users,
        "no user for a refused create"
    );
    assert_eq!(
        db.count("user_roles").await,
        roles,
        "no role for a refused create"
    );
    db.drop_schema().await;
}

// ── disable ──────────────────────────────────────────────────────────────────

/// `employees("disable")` → `disable_employee` → `disable_employee_in`: the user becomes `DISABLED`, and its
/// version moves (§5.1). Its role row stays.
#[tokio::test]
async fn a_disable_through_employees_disables_the_user() {
    let db = TestDb::start().await;
    let service = db.service();
    let id = employee(&service, "bea", "BACKEND").await;
    let before = the_user_of(&db, &id).await;
    let other = employee(&service, "bob", "BACKEND").await;
    let others_before = the_user_of(&db, &other).await;

    service
        .employees("disable", &json!({ "id": id }), "owner", at(60))
        .await
        .expect("disable");

    let after = the_user_of(&db, &id).await;
    assert_eq!(after.0, "DISABLED");
    assert!(
        after.3 > before.3,
        "the version moves: {} → {}",
        before.3,
        after.3
    );
    assert_eq!(roles_of(&db, &id).await.len(), 1, "the role row stays");
    assert_eq!(
        the_user_of(&db, &other).await,
        others_before,
        "nobody else's user changes"
    );
    db.drop_schema().await;
}

/// `AccountService::disable_employee_in` (what `DisableEmployee` calls) mirrors inside the caller's transaction:
/// committed → `DISABLED`.
#[tokio::test]
async fn a_committed_disable_in_the_callers_transaction_disables_the_user() {
    let db = TestDb::start().await;
    let service = db.service();
    let id = employee(&service, "cyd", "FRONT").await;

    let mut tx = db.pool.begin().await.expect("begin");
    service
        .disable_employee_in(&mut tx, &id, at(60), "owner")
        .await
        .expect("disable inside the transaction");
    let inside: String =
        sqlx::query_scalar("SELECT status FROM users WHERE legacy_employee_id = $1")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .expect("read inside the transaction");
    assert_eq!(
        inside, "DISABLED",
        "written through the caller's transaction"
    );
    assert_eq!(
        the_user_of(&db, &id).await.0,
        "PENDING",
        "not seen outside it before the commit"
    );
    tx.commit().await.expect("commit");
    assert_eq!(the_user_of(&db, &id).await.0, "DISABLED");
    db.drop_schema().await;
}

/// … and rolled back → the user is exactly as it was.
#[tokio::test]
async fn a_rolled_back_disable_changes_no_user() {
    let db = TestDb::start().await;
    let service = db.service();
    let id = employee(&service, "dee", "FRONT").await;
    let before = the_user_of(&db, &id).await;

    let mut tx = db.pool.begin().await.expect("begin");
    service
        .disable_employee_in(&mut tx, &id, at(60), "owner")
        .await
        .expect("disable inside the transaction");
    tx.rollback().await.expect("rollback");

    assert_eq!(the_user_of(&db, &id).await, before);
    db.drop_schema().await;
}

/// A disable of an id that names nobody is refused as today and touches no user.
#[tokio::test]
async fn a_refused_disable_changes_no_user() {
    let db = TestDb::start().await;
    let service = db.service();
    let id = employee(&service, "eve", "FRONT").await;
    let before = the_user_of(&db, &id).await;

    let error = service
        .employees("disable", &json!({ "id": "nobody" }), "owner", at(60))
        .await
        .expect_err("nobody");
    assert_eq!(refusal(&error), EMPLOYEE_NOT_FOUND);
    assert_eq!(the_user_of(&db, &id).await, before);
    assert_eq!(
        db.count("users WHERE status = 'DISABLED'").await,
        0,
        "no user is disabled"
    );
    db.drop_schema().await;
}

// ── reset ────────────────────────────────────────────────────────────────────

/// §9 item 2: "`reset_employee_password` changes nothing in `users`: passwords are not part of the new model."
#[tokio::test]
async fn a_password_reset_changes_no_user() {
    let db = TestDb::start().await;
    let service = db.service();
    let id = employee(&service, "fay", "RATES").await;
    let before = (the_user_of(&db, &id).await, roles_of(&db, &id).await);

    service
        .employees(
            "reset",
            &json!({ "id": id, "password": "another-password-1" }),
            "owner",
            at(60),
        )
        .await
        .expect("reset");
    assert_eq!(
        (the_user_of(&db, &id).await, roles_of(&db, &id).await),
        before
    );
    db.drop_schema().await;
}

// ── follow-up decision 1: an unknown branch ──────────────────────────────────

/// A create naming a branch with no `branches` row is refused with `NO_SUCH_BRANCH` and writes nothing: no
/// employee, no user, no role, no audit line.
#[tokio::test]
async fn a_create_at_an_unknown_branch_is_refused_and_writes_nothing() {
    let db = TestDb::start().await;
    let service = db.service();
    let counts = |db: &TestDb| {
        let db = db.pool.clone();
        async move {
            let mut out = Vec::new();
            for table in ["employees", "users", "user_roles", "employee_audit"] {
                let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table}"
                )))
                .fetch_one(&db)
                .await
                .unwrap_or_else(|e| panic!("count {table}: {e}"));
                out.push((table, n));
            }
            out
        }
    };
    let before = counts(&db).await;

    let error = service
        .employees(
            "create",
            &new_employee("ghost", "FRONT", "NOWHERE", "the-first-password"),
            "owner",
            at(0),
        )
        .await
        .expect_err("a branch with no row");
    assert_eq!(refusal(&error), NO_SUCH_BRANCH);
    assert_eq!(counts(&db).await, before, "nothing is written");

    // The same create at a branch that has its row goes through.
    db.seed_branch("NOWHERE").await;
    let id = employee_at(&service, "ghost", "FRONT", "NOWHERE").await;
    assert_eq!(the_user_of(&db, &id).await.0, "PENDING");
    db.drop_schema().await;
}

// ── follow-up decision 2: the owner ──────────────────────────────────────────

/// An owner created after migration 2 (`create-owner` runs after the migrations on a fresh install) has one
/// `users` row, `PENDING`, with no `legacy_employee_id`, holding EXECUTIVE and ADMIN, both `branch_id` NULL. A
/// refused second owner adds no user.
#[tokio::test]
async fn an_owner_created_after_migration_2_is_executive_and_admin_organisation_wide() {
    let db = TestDb::start().await;
    let service = db.service();
    let org = organization(&db).await;
    assert_eq!(db.count("users").await, 0, "a fresh install has no user");

    service
        .create_owner("boss", "owner-password-01", at(0))
        .await
        .expect("create the owner");

    let owners: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id::text, status, name, auth_subject FROM users WHERE legacy_employee_id IS NULL",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read users");
    assert_eq!(owners.len(), 1, "one user for the owner: {owners:?}");
    let (user, status, name, subject) = &owners[0];
    assert_eq!(
        (status.as_str(), name.as_str(), subject.clone()),
        ("PENDING", "boss", None)
    );
    let roles: Vec<(String, Option<String>, String, bool)> = sqlx::query_as(
        "SELECT role_code::text, branch_id::text, organization_id::text, valid_to IS NULL \
         FROM user_roles WHERE user_id::text = $1 ORDER BY 1",
    )
    .bind(user)
    .fetch_all(&db.pool)
    .await
    .expect("read the owner's roles");
    assert_eq!(
        roles,
        vec![
            ("ADMIN".to_owned(), None, org.clone(), true),
            ("EXECUTIVE".to_owned(), None, org.clone(), true),
        ],
        "EXECUTIVE + ADMIN, both with branch_id NULL"
    );

    service
        .create_owner("boss2", "owner-password-02", at(1))
        .await
        .expect_err("there is one owner");
    assert_eq!(db.count("users").await, 1, "a refused owner adds no user");
    assert_eq!(db.count("user_roles").await, 2);
    db.drop_schema().await;
}
