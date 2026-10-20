//! Tests for UserService (OR-2): grant with ScopeRule check, revoke, and disable.
//! #1157: users are seeded as `userCreate` leaves them before Identity binds them (`seed_user`).
//! RBAC C1 (PR 3083 §3.1, §3.2): the calls name the acting user (the owner, EXECUTIVE + ADMIN
//! organisation-wide, so `GrantGuard` admits each change), the client IP (`None`) and, for
//! `disable`, the cash guard. The assertions are unchanged.

mod account_common;

use std::sync::Arc;

use account_common::TestDb;
use chrono::{DateTime, Duration, SubsecRound, Utc};
use xc_auth::application::user::UserService;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_auth::domain::user::{CustodyHolds, User, UserRepository, UserStatus};
use xc_auth::infrastructure::user::PgUserRepository;
use xc_kernel::ids::BranchId;

/// The owner of B3: EXECUTIVE + ADMIN organisation-wide (RBAC C1).
fn by_owner() -> Actor {
    let epoch = DateTime::<Utc>::UNIX_EPOCH;
    Actor::new(
        "owner-test",
        "XC",
        vec![
            ScopedRole::new(Role::Executive, None, epoch, None),
            ScopedRole::new(Role::Admin, None, epoch, None),
        ],
        None,
    )
}

/// A cash guard that finds no cash (today's behaviour of `userDisable`).
struct NoCash;

impl CustodyHolds for NoCash {
    fn holds_cash_or_pending_handover<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _user_id: &'a str,
    ) -> futures::future::BoxFuture<'a, pos_error::AppResult<bool>> {
        Box::pin(async { Ok(false) })
    }
}

async fn organization(db: &TestDb) -> String {
    sqlx::query_scalar("SELECT id::text FROM organizations WHERE code = 'XC'")
        .fetch_one(&db.pool)
        .await
        .expect("OR-1 organization")
}

/// A `PENDING` user with its roles, as `UserService::create_staff` leaves one before Identity binds it.
async fn seed_user(
    service: &UserService<PgUserRepository>,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_id: &str,
    name: &str,
    roles: Vec<ScopedRole>,
    now: DateTime<Utc>,
) -> User {
    let id: String = sqlx::query_scalar("SELECT gen_random_uuid()::text")
        .fetch_one(&mut **tx)
        .await
        .expect("uuid");
    let user = User {
        id,
        organization_id: org_id.to_string(),
        auth_subject: None,
        name: name.to_string(),
        status: UserStatus::Pending,
        version: 1,
        created_at: now,
        created_by: "admin".to_string(),
        legacy_employee_id: None,
        roles: roles.clone(),
    };
    service
        .repo()
        .create_user(tx, &user)
        .await
        .expect("create user");
    for r in &roles {
        service
            .repo()
            .add_role(tx, &user.id, org_id, r)
            .await
            .expect("add role");
    }
    user
}

#[tokio::test]
async fn test_grant_refused_by_scope_rule() {
    let db = TestDb::start().await;
    let repo = Arc::new(PgUserRepository::new());
    let user_service = UserService::new(repo);
    let org_id = organization(&db).await;
    let now = Utc::now();

    db.seed_branch("B2").await;

    let mut tx = db.pool.begin().await.expect("begin tx");

    // A user with TELLER at MAIN
    let user = seed_user(
        &user_service,
        &mut tx,
        &org_id,
        "Bob",
        vec![ScopedRole::new(
            Role::Teller,
            Some(BranchId::new("MAIN")),
            now,
            None,
        )],
        now,
    )
    .await;

    // Attempting to grant TELLER at B2 should be refused by ScopeRule (Teller takes only one branch)
    let err = user_service
        .grant(
            &mut tx,
            &by_owner(),
            None,
            &user.id,
            Role::Teller,
            Some(BranchId::new("B2")),
            now,
            None,
        )
        .await
        .expect_err("TELLER at second branch must be refused by ScopeRule");

    assert!(
        err.to_string().contains("Scope rule rejected"),
        "error must mention Scope rule rejection: {err}"
    );

    // Attempting to grant EXECUTIVE with a branch should also be refused (Executive must be organisation-wide)
    let err_exec = user_service
        .grant(
            &mut tx,
            &by_owner(),
            None,
            &user.id,
            Role::Executive,
            Some(BranchId::new("B2")),
            now,
            None,
        )
        .await
        .expect_err("EXECUTIVE with branch must be refused by ScopeRule");

    assert!(
        err_exec.to_string().contains("Scope rule rejected"),
        "error must mention Scope rule rejection: {err_exec}"
    );

    tx.commit().await.expect("commit tx");
    db.drop_schema().await;
}

#[tokio::test]
async fn test_revoke_ends_role() {
    let db = TestDb::start().await;
    let repo = Arc::new(PgUserRepository::new());
    let user_service = UserService::new(repo.clone());
    let org_id = organization(&db).await;
    let now = Utc::now().trunc_subsecs(6);

    let mut tx = db.pool.begin().await.expect("begin tx");

    let user = seed_user(
        &user_service,
        &mut tx,
        &org_id,
        "Charlie",
        vec![ScopedRole::new(
            Role::RateManager,
            Some(BranchId::new("MAIN")),
            now,
            None,
        )],
        now,
    )
    .await;

    // Revoke RateManager at MAIN
    let revoke_time = now + Duration::hours(1);
    let main_branch = BranchId::new("MAIN");
    user_service
        .revoke(
            &mut tx,
            &by_owner(),
            None,
            &user.id,
            Role::RateManager,
            Some(&main_branch),
            revoke_time,
        )
        .await
        .expect("revoke role");

    let updated_user = user_service
        .get_user(&mut tx, &user.id)
        .await
        .expect("get user")
        .expect("user exists");

    // Version bumped on revoke
    assert_eq!(updated_user.version, 2);

    // Verify valid_to is set
    assert_eq!(updated_user.roles.len(), 1);
    assert_eq!(updated_user.roles[0].valid_to, Some(revoke_time));

    tx.commit().await.expect("commit tx");
    db.drop_schema().await;
}

#[tokio::test]
async fn test_disable_sets_status_disabled_and_bumps_version() {
    let db = TestDb::start().await;
    let repo = Arc::new(PgUserRepository::new());
    let user_service = UserService::new(repo);
    let org_id = organization(&db).await;
    let now = Utc::now();

    let mut tx = db.pool.begin().await.expect("begin tx");

    let seeded = seed_user(
        &user_service,
        &mut tx,
        &org_id,
        "David",
        vec![ScopedRole::new(Role::Admin, None, now, None)],
        now,
    )
    .await;
    // Bound and ACTIVE, as create_staff's bind leaves it (one version bump).
    user_service
        .repo()
        .set_auth_subject(&mut tx, &seeded.id, "sub-david")
        .await
        .expect("bind");
    user_service
        .repo()
        .set_status(&mut tx, &seeded.id, UserStatus::Active)
        .await
        .expect("activate");
    let active_user = user_service
        .get_user(&mut tx, &seeded.id)
        .await
        .expect("get user")
        .expect("user exists");
    assert_eq!(active_user.status, UserStatus::Active);
    assert_eq!(active_user.version, 2);

    // Disable user
    user_service
        .disable(&mut tx, &by_owner(), None, &active_user.id, &NoCash)
        .await
        .expect("disable");

    let disabled_user = user_service
        .get_user(&mut tx, &active_user.id)
        .await
        .expect("get user")
        .expect("user exists");

    assert_eq!(disabled_user.status, UserStatus::Disabled);
    assert_eq!(disabled_user.version, 3);

    // Further grant on disabled user is refused
    let grant_err = user_service
        .grant(
            &mut tx,
            &by_owner(),
            None,
            &disabled_user.id,
            Role::Marketing,
            None,
            now,
            None,
        )
        .await;
    assert!(grant_err.is_err());

    tx.commit().await.expect("commit tx");
    db.drop_schema().await;
}
