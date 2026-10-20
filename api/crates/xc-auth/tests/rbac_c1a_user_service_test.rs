//! RBAC C1a up-front tests (design `docs/design/blueprint/user-management-rbac.md`, PR 3083, §3.2,
//! §3.4, §3.6 row 1; owner decision Q3 A): `UserService::suspend`, `resume`, `enable` and
//! `disable` call `GrantGuard`, `UserStatus::may_move`, the cash guard and `AccountAuditor`, and
//! move `users.version` through `set_status`.
//! Written before the implementation by the ticket author (claude/83f682). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! The cash guard is a method parameter (`holds: &dyn CustodyHolds`, §3.2 "Injection"); these
//! tests pass a fake that answers a fixed value. The status CHECK is widened in each test's own
//! schema the way migration M (§4) will. Names the new methods and types, so on `origin/main`
//! this file fails to compile; that is its red. Needs `TEST_DATABASE_URL`.

mod account_common;

use std::sync::Arc;

use account_common::{TestDb, unique};
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_auth::application::user::UserService;
use xc_auth::application::user::user_service::UserChangeError;
use xc_auth::domain::access::{Actor, GrantRefusal, Role, ScopedRole};
use xc_auth::domain::user::CustodyHolds;
use xc_auth::infrastructure::user::PgUserRepository;
use xc_testdb::people::{RoleRow, seed_roles};

/// A cash guard that answers `holds` for everyone.
struct Holds(bool);

impl CustodyHolds for Holds {
    fn holds_cash_or_pending_handover<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _user_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        let holds = self.0;
        Box::pin(async move { Ok(holds) })
    }
}

const HOLDS: Holds = Holds(true);
const FREE: Holds = Holds(false);

fn service() -> UserService<PgUserRepository> {
    UserService::new(Arc::new(PgUserRepository::new()))
}

fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

fn org_admin() -> Actor {
    Actor::new(
        "admin-1",
        "XC",
        vec![ScopedRole::new(Role::Admin, None, epoch(), None)],
        None,
    )
}

fn owner() -> Actor {
    Actor::new(
        "owner-1",
        "XC",
        vec![
            ScopedRole::new(Role::Executive, None, epoch(), None),
            ScopedRole::new(Role::Admin, None, epoch(), None),
        ],
        None,
    )
}

async fn start() -> TestDb {
    let db = TestDb::start().await;
    db.seed_branch("XC-B01").await;
    sqlx::query("ALTER TABLE users DROP CONSTRAINT IF EXISTS users_status_check")
        .execute(&db.pool)
        .await
        .expect("drop the status check");
    sqlx::query(
        "ALTER TABLE users ADD CONSTRAINT users_status_check \
         CHECK (status IN ('PENDING', 'ACTIVE', 'SUSPENDED', 'DISABLED'))",
    )
    .execute(&db.pool)
    .await
    .expect("the status check as migration M leaves it");
    db
}

/// A user of organisation `XC` with `status`, bound to Identity when `bound`, holding `roles`
/// (`None` branch = organisation-wide); answers its id.
async fn a_user(db: &TestDb, status: &str, bound: bool, roles: &[(&str, Option<&str>)]) -> String {
    let subject = bound.then(|| unique("sub"));
    let (id, org_id): (String, String) = sqlx::query_as(
        "INSERT INTO users (organization_id, auth_subject, name, status, created_by) \
         SELECT id, $1, $2, $3, 'test' FROM organizations WHERE code = 'XC' \
         RETURNING id::text, organization_id::text",
    )
    .bind(subject)
    .bind(unique("name"))
    .bind(status)
    .fetch_one(&db.pool)
    .await
    .expect("insert the user");
    let role_rows: Vec<RoleRow<'_>> = roles
        .iter()
        .map(|(role, branch)| RoleRow {
            role_code: role,
            branch_id: *branch,
            valid_from: epoch(),
            valid_to: None,
        })
        .collect();
    let mut conn = db.pool.acquire().await.expect("acquire connection");
    seed_roles(&mut conn, &id, &org_id, &role_rows)
        .await
        .expect("seed roles");
    id
}

async fn row(db: &TestDb, id: &str) -> (String, i32) {
    sqlx::query_as("SELECT status, version FROM users WHERE id = $1::uuid")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .expect("the user row")
}

async fn audit(db: &TestDb, id: &str) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT actor_id, action FROM user_audit WHERE subject_id = $1 ORDER BY at, action",
    )
    .bind(id)
    .fetch_all(&db.pool)
    .await
    .expect("the audit rows")
}

const TELLER: &[(&str, Option<&str>)] = &[("TELLER", Some("XC-B01"))];

#[tokio::test]
async fn suspend_moves_an_active_user_to_suspended_and_audits_it() {
    let db = start().await;
    let id = a_user(&db, "ACTIVE", true, TELLER).await;
    let mut tx = db.pool.begin().await.expect("begin");
    service()
        .suspend(&mut tx, &org_admin(), None, &id, &FREE)
        .await
        .expect("suspend");
    tx.commit().await.expect("commit");
    assert_eq!(row(&db, &id).await, ("SUSPENDED".to_owned(), 2));
    assert_eq!(
        audit(&db, &id).await,
        vec![("admin-1".to_owned(), "userSuspend".to_owned())]
    );
    db.drop_schema().await;
}

#[tokio::test]
async fn suspend_and_disable_are_refused_while_the_user_holds_cash() {
    let db = start().await;
    let id = a_user(&db, "ACTIVE", true, TELLER).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let s = service()
        .suspend(&mut tx, &org_admin(), None, &id, &HOLDS)
        .await;
    assert!(
        matches!(s, Err(UserChangeError::HoldsCash)),
        "suspend: {s:?}"
    );
    let d = service()
        .disable(&mut tx, &org_admin(), None, &id, &HOLDS)
        .await;
    assert!(
        matches!(d, Err(UserChangeError::HoldsCash)),
        "disable: {d:?}"
    );
    tx.commit().await.expect("commit");
    assert_eq!(row(&db, &id).await, ("ACTIVE".to_owned(), 1), "unchanged");
    assert_eq!(audit(&db, &id).await, vec![], "nothing audited");
    db.drop_schema().await;
}

#[tokio::test]
async fn resume_moves_a_suspended_user_back_to_active() {
    let db = start().await;
    let id = a_user(&db, "SUSPENDED", true, TELLER).await;
    let mut tx = db.pool.begin().await.expect("begin");
    service()
        .resume(&mut tx, &org_admin(), None, &id)
        .await
        .expect("resume");
    tx.commit().await.expect("commit");
    assert_eq!(row(&db, &id).await, ("ACTIVE".to_owned(), 2));
    assert_eq!(
        audit(&db, &id).await,
        vec![("admin-1".to_owned(), "userResume".to_owned())]
    );
    db.drop_schema().await;
}

#[tokio::test]
async fn a_move_outside_the_table_is_refused() {
    let db = start().await;
    let pending = a_user(&db, "PENDING", false, TELLER).await;
    let active = a_user(&db, "ACTIVE", true, TELLER).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let s = service()
        .suspend(&mut tx, &org_admin(), None, &pending, &FREE)
        .await;
    assert!(
        matches!(s, Err(UserChangeError::BadMove { .. })),
        "suspend PENDING: {s:?}"
    );
    let r = service().resume(&mut tx, &org_admin(), None, &active).await;
    assert!(
        matches!(r, Err(UserChangeError::BadMove { .. })),
        "resume ACTIVE: {r:?}"
    );
    let e = service().enable(&mut tx, &owner(), None, &active).await;
    assert!(
        matches!(e, Err(UserChangeError::BadMove { .. })),
        "enable ACTIVE: {e:?}"
    );
    tx.commit().await.expect("commit");
    assert_eq!(row(&db, &pending).await, ("PENDING".to_owned(), 1));
    assert_eq!(row(&db, &active).await, ("ACTIVE".to_owned(), 1));
    db.drop_schema().await;
}

#[tokio::test]
async fn only_an_executive_enables_a_disabled_user() {
    let db = start().await;
    let id = a_user(&db, "DISABLED", true, TELLER).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let by_admin = service().enable(&mut tx, &org_admin(), None, &id).await;
    assert!(
        matches!(
            by_admin,
            Err(UserChangeError::Refused(GrantRefusal::EnableNeedsExecutive))
        ),
        "{by_admin:?}"
    );
    service()
        .enable(&mut tx, &owner(), None, &id)
        .await
        .expect("the owner enables");
    tx.commit().await.expect("commit");
    assert_eq!(row(&db, &id).await, ("ACTIVE".to_owned(), 2));
    assert_eq!(
        audit(&db, &id).await,
        vec![("owner-1".to_owned(), "userEnable".to_owned())]
    );
    db.drop_schema().await;
}

#[tokio::test]
async fn a_disabled_user_never_bound_is_not_enabled() {
    let db = start().await;
    let id = a_user(&db, "DISABLED", false, TELLER).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let e = service().enable(&mut tx, &owner(), None, &id).await;
    assert!(matches!(e, Err(UserChangeError::NotBound)), "{e:?}");
    tx.commit().await.expect("commit");
    assert_eq!(row(&db, &id).await, ("DISABLED".to_owned(), 1));
    db.drop_schema().await;
}

#[tokio::test]
async fn disable_with_no_cash_moves_the_version_and_audits_it() {
    let db = start().await;
    let id = a_user(&db, "ACTIVE", true, TELLER).await;
    let mut tx = db.pool.begin().await.expect("begin");
    service()
        .disable(&mut tx, &org_admin(), None, &id, &FREE)
        .await
        .expect("disable");
    tx.commit().await.expect("commit");
    assert_eq!(row(&db, &id).await, ("DISABLED".to_owned(), 2));
    assert_eq!(
        audit(&db, &id).await,
        vec![("admin-1".to_owned(), "userDisable".to_owned())]
    );
    db.drop_schema().await;
}

#[tokio::test]
async fn the_last_active_owner_is_not_suspended() {
    let db = start().await;
    let only = a_user(&db, "ACTIVE", true, &[("EXECUTIVE", None), ("ADMIN", None)]).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let s = service()
        .suspend(&mut tx, &owner(), None, &only, &FREE)
        .await;
    assert!(
        matches!(s, Err(UserChangeError::Refused(GrantRefusal::LastOwner))),
        "{s:?}"
    );
    tx.commit().await.expect("commit");
    assert_eq!(row(&db, &only).await, ("ACTIVE".to_owned(), 1));
    db.drop_schema().await;
}

#[tokio::test]
async fn a_refused_guard_comes_before_the_cash_guard() {
    let db = start().await;
    let other_admin = a_user(&db, "ACTIVE", true, &[("ADMIN", Some("XC-B01"))]).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let s = service()
        .suspend(&mut tx, &org_admin(), None, &other_admin, &HOLDS)
        .await;
    assert!(
        matches!(
            s,
            Err(UserChangeError::Refused(GrantRefusal::ProtectedTarget))
        ),
        "{s:?}"
    );
    tx.commit().await.expect("commit");
    db.drop_schema().await;
}

#[tokio::test]
async fn an_unknown_user_is_not_found() {
    let db = start().await;
    let mut tx = db.pool.begin().await.expect("begin");
    let s = service()
        .suspend(
            &mut tx,
            &org_admin(),
            None,
            "00000000-0000-4000-8000-0000000000c1",
            &FREE,
        )
        .await;
    assert!(matches!(s, Err(UserChangeError::NotFound)), "{s:?}");
    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}
