//! Tests for UserService (OR-2): invite, claim, grant with ScopeRule check, revoke, and disable.

mod account_common;

use std::sync::Arc;

use account_common::TestDb;
use chrono::{Duration, SubsecRound, Utc};
use xc_auth::application::user::UserService;
use xc_auth::domain::access::{Role, ScopedRole};
use xc_auth::domain::user::UserStatus;
use xc_auth::infrastructure::user::PgUserRepository;
use xc_kernel::ids::BranchId;

async fn organization(db: &TestDb) -> String {
    sqlx::query_scalar("SELECT id::text FROM organizations WHERE code = 'XC'")
        .fetch_one(&db.pool)
        .await
        .expect("OR-1 organization")
}

#[tokio::test]
async fn test_invite_and_claim_flow() {
    let db = TestDb::start().await;
    let repo = Arc::new(PgUserRepository::new());
    let user_service = UserService::new(repo);
    let org_id = organization(&db).await;
    let now = Utc::now();
    let expires_at = now + Duration::days(7);

    let mut tx = db.pool.begin().await.expect("begin tx");

    // 1. Invite a user
    let (user, invite_code) = user_service
        .invite(
            &mut tx,
            &org_id,
            "Alice",
            vec![ScopedRole::new(
                Role::Teller,
                Some(BranchId::new("MAIN")),
                now,
                None,
            )],
            "admin",
            now,
            expires_at,
        )
        .await
        .expect("invite user");

    assert_eq!(user.status, UserStatus::Invited);
    assert_eq!(user.auth_subject, None);
    assert_eq!(user.version, 1);
    assert_eq!(user.name, "Alice");
    assert!(!invite_code.is_empty());

    // 2. Claim with bad code should fail
    let bad_claim = user_service
        .claim(&mut tx, "invalid-code", "sub-alice", now)
        .await;
    assert!(bad_claim.is_err());

    // 3. Claim with valid code succeeds
    let claimed = user_service
        .claim(&mut tx, &invite_code, "sub-alice", now)
        .await
        .expect("claim invite");

    assert_eq!(claimed.id, user.id);
    assert_eq!(claimed.status, UserStatus::Active);
    assert_eq!(claimed.auth_subject.as_deref(), Some("sub-alice"));
    assert_eq!(claimed.version, 2);

    // 4. Second claim with the same code fails (already used)
    let second_claim = user_service
        .claim(&mut tx, &invite_code, "sub-another", now)
        .await;
    assert!(second_claim.is_err());

    tx.commit().await.expect("commit tx");
    db.drop_schema().await;
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

    // Invite user with TELLER at MAIN
    let (user, _) = user_service
        .invite(
            &mut tx,
            &org_id,
            "Bob",
            vec![ScopedRole::new(
                Role::Teller,
                Some(BranchId::new("MAIN")),
                now,
                None,
            )],
            "admin",
            now,
            now + Duration::days(7),
        )
        .await
        .expect("invite");

    // Attempting to grant TELLER at B2 should be refused by ScopeRule (Teller takes only one branch)
    let err = user_service
        .grant(
            &mut tx,
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

    let (user, _) = user_service
        .invite(
            &mut tx,
            &org_id,
            "Charlie",
            vec![ScopedRole::new(
                Role::RateManager,
                Some(BranchId::new("MAIN")),
                now,
                None,
            )],
            "admin",
            now,
            now + Duration::days(7),
        )
        .await
        .expect("invite");

    // Revoke RateManager at MAIN
    let revoke_time = now + Duration::hours(1);
    let main_branch = BranchId::new("MAIN");
    user_service
        .revoke(
            &mut tx,
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

    let (_user, invite_code) = user_service
        .invite(
            &mut tx,
            &org_id,
            "David",
            vec![ScopedRole::new(Role::Admin, None, now, None)],
            "admin",
            now,
            now + Duration::days(7),
        )
        .await
        .expect("invite");

    let active_user = user_service
        .claim(&mut tx, &invite_code, "sub-david", now)
        .await
        .expect("claim");
    assert_eq!(active_user.status, UserStatus::Active);
    assert_eq!(active_user.version, 2);

    // Disable user
    user_service
        .disable(&mut tx, &active_user.id)
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
        .grant(&mut tx, &disabled_user.id, Role::Marketing, None, now, None)
        .await;
    assert!(grant_err.is_err());

    tx.commit().await.expect("commit tx");
    db.drop_schema().await;
}

#[tokio::test]
async fn test_concurrent_claims_of_same_code() {
    let db = TestDb::start().await;
    let repo = Arc::new(PgUserRepository::new());
    let user_service = UserService::new(repo.clone());
    let org_id = organization(&db).await;
    let now = Utc::now();
    let expires_at = now + Duration::days(7);

    // Setup: invite a user
    let mut setup_tx = db.pool.begin().await.expect("begin setup tx");
    let (user, invite_code) = user_service
        .invite(
            &mut setup_tx,
            &org_id,
            "Eve",
            vec![ScopedRole::new(Role::Admin, None, now, None)],
            "admin",
            now,
            expires_at,
        )
        .await
        .expect("invite");
    setup_tx.commit().await.expect("commit setup tx");

    // Two transactions / connections attempt to claim concurrently with different subs
    let mut tx1 = db.pool.begin().await.expect("begin tx1");
    let mut tx2 = db.pool.begin().await.expect("begin tx2");

    // tx1 claims the code (atomically spends it and holds row lock)
    let claimed1 = user_service
        .claim(&mut tx1, &invite_code, "sub-winner-1", now)
        .await
        .expect("tx1 claim succeeds");
    assert_eq!(claimed1.auth_subject.as_deref(), Some("sub-winner-1"));

    // tx2 claims with different sub in background task (will block on row lock until tx1 commits)
    let service2 = user_service.clone();
    let code2 = invite_code.clone();
    let tx2_task = tokio::spawn(async move {
        let res = service2.claim(&mut tx2, &code2, "sub-loser-2", now).await;
        (tx2, res)
    });

    // Small sleep so tx2 reaches spend_invite and blocks on the lock
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // tx1 commits, releasing lock
    tx1.commit().await.expect("tx1 commit");

    // tx2 unblocks, re-evaluates WHERE used_at IS NULL (now false), and is refused
    let (tx2, res2) = tx2_task.await.expect("join tx2");
    assert!(
        res2.is_err(),
        "tx2 must be refused because invite was spent by tx1"
    );
    let _ = tx2.rollback().await;

    // Assert the user's auth_subject in the database is tx1's winner
    let mut check_tx = db.pool.begin().await.expect("begin check tx");
    let final_user = user_service
        .get_user(&mut check_tx, &user.id)
        .await
        .expect("get user")
        .expect("user exists");
    assert_eq!(final_user.auth_subject.as_deref(), Some("sub-winner-1"));
    assert_eq!(final_user.status, UserStatus::Active);
    check_tx.commit().await.expect("commit check tx");

    db.drop_schema().await;
}
