//! RBAC C1c tests (ticket #3146): `UserService::save_profile` and the `before`/`after` audit
//! JSON on `grant`, `revoke`, `disable`, `suspend`, `resume`, `enable`, and `save_profile`.

mod account_common;

use std::sync::Arc;

use account_common::{TestDb, unique};
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use xc_auth::application::user::UserService;
use xc_auth::application::user::user_service::UserChangeError;
use xc_auth::domain::access::{Actor, GrantRefusal, Role, ScopedRole};
use xc_auth::domain::user::{CustodyHolds, UserProfile};
use xc_auth::infrastructure::user::PgUserRepository;
use xc_kernel::ids::BranchId;
use xc_testdb::people::{RoleRow, seed_roles};

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

fn branch_admin(branch: &str) -> Actor {
    Actor::new(
        "branch-admin-1",
        "XC",
        vec![ScopedRole::new(
            Role::Admin,
            Some(BranchId::new(branch)),
            epoch(),
            None,
        )],
        None,
    )
}

async fn start() -> TestDb {
    let db = TestDb::start().await;
    db.seed_branch("XC-B01").await;
    db.seed_branch("XC-B02").await;
    db
}

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

async fn version_of(db: &TestDb, id: &str) -> i32 {
    sqlx::query_scalar("SELECT version FROM users WHERE id = $1::uuid")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .expect("the user version")
}

async fn audit_rows(db: &TestDb, id: &str) -> Vec<(String, Option<Value>, Option<Value>)> {
    sqlx::query_as(
        "SELECT action, before, after FROM user_audit WHERE subject_id = $1 ORDER BY at, id",
    )
    .bind(id)
    .fetch_all(&db.pool)
    .await
    .expect("the audit rows")
}

const TELLER_B01: &[(&str, Option<&str>)] = &[("TELLER", Some("XC-B01"))];

#[tokio::test]
async fn admin_saves_an_active_employees_profile() {
    let db = start().await;
    let id = a_user(&db, "ACTIVE", true, TELLER_B01).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let input = UserProfile {
        employee_code: Some("EMP-01".to_owned()),
        department: Some("Front".to_owned()),
        position: Some("Teller".to_owned()),
        tell_code: Some("T01".to_owned()),
    };
    let saved = service()
        .save_profile(&mut tx, &org_admin(), None, &id, &input)
        .await
        .expect("save_profile");
    tx.commit().await.expect("commit");

    assert_eq!(saved, input);
    assert_eq!(version_of(&db, &id).await, 2);
    assert_eq!(
        audit_rows(&db, &id).await,
        vec![(
            "userProfileSave".to_owned(),
            Some(json!({
                "employeeCode": null,
                "department": null,
                "position": null,
                "tellCode": null,
            })),
            Some(json!({
                "employeeCode": "EMP-01",
                "department": "Front",
                "position": "Teller",
                "tellCode": "T01",
            })),
        )]
    );
    db.drop_schema().await;
}

#[tokio::test]
async fn blank_department_is_normalised_to_none() {
    let db = start().await;
    let id = a_user(&db, "ACTIVE", true, TELLER_B01).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let input = UserProfile {
        employee_code: Some("  EMP-02 ".to_owned()),
        department: Some("  ".to_owned()),
        position: Some("".to_owned()),
        tell_code: None,
    };
    let saved = service()
        .save_profile(&mut tx, &org_admin(), None, &id, &input)
        .await
        .expect("save_profile");
    tx.commit().await.expect("commit");

    assert_eq!(
        saved,
        UserProfile {
            employee_code: Some("EMP-02".to_owned()),
            department: None,
            position: None,
            tell_code: None,
        }
    );
    assert_eq!(
        audit_rows(&db, &id).await,
        vec![(
            "userProfileSave".to_owned(),
            Some(json!({
                "employeeCode": null,
                "department": null,
                "position": null,
                "tellCode": null,
            })),
            Some(json!({
                "employeeCode": "EMP-02",
                "department": null,
                "position": null,
                "tellCode": null,
            })),
        )]
    );
    db.drop_schema().await;
}

#[tokio::test]
async fn disabled_target_is_refused_as_not_active() {
    let db = start().await;
    let id = a_user(&db, "DISABLED", true, TELLER_B01).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let input = UserProfile {
        employee_code: Some("EMP-03".to_owned()),
        ..Default::default()
    };
    let res = service()
        .save_profile(&mut tx, &org_admin(), None, &id, &input)
        .await;
    assert!(matches!(res, Err(UserChangeError::NotActive)), "{res:?}");
    tx.commit().await.expect("commit");

    assert_eq!(version_of(&db, &id).await, 1);
    assert_eq!(audit_rows(&db, &id).await, vec![]);
    db.drop_schema().await;
}

#[tokio::test]
async fn suspended_and_pending_targets_succeed() {
    let db = start().await;
    let suspended = a_user(&db, "SUSPENDED", true, TELLER_B01).await;
    let pending = a_user(&db, "PENDING", false, TELLER_B01).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let p1 = UserProfile {
        department: Some("Ops".to_owned()),
        ..Default::default()
    };
    let p2 = UserProfile {
        position: Some("Clerk".to_owned()),
        ..Default::default()
    };
    let saved1 = service()
        .save_profile(&mut tx, &org_admin(), None, &suspended, &p1)
        .await
        .expect("suspended ok");
    let saved2 = service()
        .save_profile(&mut tx, &org_admin(), None, &pending, &p2)
        .await
        .expect("pending ok");
    tx.commit().await.expect("commit");

    assert_eq!(saved1, p1);
    assert_eq!(saved2, p2);
    assert_eq!(version_of(&db, &suspended).await, 2);
    assert_eq!(version_of(&db, &pending).await, 2);
    db.drop_schema().await;
}

#[tokio::test]
async fn actor_without_scope_over_target_is_refused() {
    let db = start().await;
    let id = a_user(&db, "ACTIVE", true, TELLER_B01).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let input = UserProfile {
        department: Some("Front".to_owned()),
        ..Default::default()
    };
    let res = service()
        .save_profile(&mut tx, &branch_admin("XC-B02"), None, &id, &input)
        .await;
    assert!(
        matches!(res, Err(UserChangeError::Refused(GrantRefusal::NoScope))),
        "{res:?}"
    );
    tx.commit().await.expect("commit");

    assert_eq!(version_of(&db, &id).await, 1);
    assert_eq!(audit_rows(&db, &id).await, vec![]);
    db.drop_schema().await;
}

#[tokio::test]
async fn admin_editing_own_profile_follows_grant_guard_self_rule() {
    let db = start().await;
    let self_id = a_user(&db, "ACTIVE", true, &[("ADMIN", Some("XC-B01"))]).await;
    let self_actor = Actor::new(
        &self_id,
        "XC",
        vec![ScopedRole::new(
            Role::Admin,
            Some(BranchId::new("XC-B01")),
            epoch(),
            None,
        )],
        None,
    );
    let mut tx = db.pool.begin().await.expect("begin");
    let input = UserProfile {
        department: Some("IT".to_owned()),
        ..Default::default()
    };
    let saved = service()
        .save_profile(&mut tx, &self_actor, None, &self_id, &input)
        .await
        .expect("GrantGuard allows EditProfile on self");
    tx.commit().await.expect("commit");

    assert_eq!(saved, input);
    assert_eq!(version_of(&db, &self_id).await, 2);
    db.drop_schema().await;
}

#[tokio::test]
async fn unknown_user_id_returns_not_found() {
    let db = start().await;
    let mut tx = db.pool.begin().await.expect("begin");
    let res = service()
        .save_profile(
            &mut tx,
            &org_admin(),
            None,
            "00000000-0000-4000-8000-0000000000c9",
            &UserProfile::default(),
        )
        .await;
    assert!(matches!(res, Err(UserChangeError::NotFound)), "{res:?}");
    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}

#[tokio::test]
async fn duplicate_employee_code_returns_conflict_and_writes_no_audit_row() {
    let db = start().await;
    let u1 = a_user(&db, "ACTIVE", true, TELLER_B01).await;
    let u2 = a_user(&db, "ACTIVE", true, TELLER_B01).await;
    sqlx::query("UPDATE users SET employee_code = 'DUP-01' WHERE id = $1::uuid")
        .bind(&u1)
        .execute(&db.pool)
        .await
        .expect("seed existing code");

    let mut tx = db.pool.begin().await.expect("begin");
    let input = UserProfile {
        employee_code: Some("dup-01".to_owned()),
        ..Default::default()
    };
    let res = service()
        .save_profile(&mut tx, &org_admin(), None, &u2, &input)
        .await;
    assert!(
        matches!(res, Err(UserChangeError::App(AppError::Conflict(ref c))) if c == "employeeCodeTaken"),
        "{res:?}"
    );
    tx.rollback().await.expect("rollback");

    assert_eq!(version_of(&db, &u2).await, 1);
    assert_eq!(audit_rows(&db, &u2).await, vec![]);
    db.drop_schema().await;
}

#[tokio::test]
async fn suspend_of_an_active_user_records_status_before_and_after() {
    let db = start().await;
    let id = a_user(&db, "ACTIVE", true, TELLER_B01).await;
    let mut tx = db.pool.begin().await.expect("begin");
    service()
        .suspend(&mut tx, &org_admin(), None, &id, &FREE)
        .await
        .expect("suspend");
    tx.commit().await.expect("commit");

    assert_eq!(
        audit_rows(&db, &id).await,
        vec![(
            "userSuspend".to_owned(),
            Some(json!({ "status": "ACTIVE" })),
            Some(json!({ "status": "SUSPENDED" })),
        )]
    );
    db.drop_schema().await;
}

#[tokio::test]
async fn grant_records_before_null_and_after_with_the_role() {
    let db = start().await;
    let id = a_user(&db, "ACTIVE", true, TELLER_B01).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let from = epoch();
    service()
        .grant(
            &mut tx,
            &org_admin(),
            None,
            &id,
            Role::Marketing,
            None,
            from,
            None,
        )
        .await
        .expect("grant");
    tx.commit().await.expect("commit");

    assert_eq!(
        audit_rows(&db, &id).await,
        vec![(
            "userGrant".to_owned(),
            None,
            Some(json!({
                "role": "MARKETING",
                "branch": null,
                "validFrom": from,
                "validTo": null,
            })),
        )]
    );
    db.drop_schema().await;
}

#[tokio::test]
async fn revoke_records_before_with_the_role_and_after_null() {
    let db = start().await;
    let id = a_user(&db, "ACTIVE", true, TELLER_B01).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let branch = BranchId::new("XC-B01");
    service()
        .revoke(
            &mut tx,
            &org_admin(),
            None,
            &id,
            Role::Teller,
            Some(&branch),
            Utc::now(),
        )
        .await
        .expect("revoke");
    tx.commit().await.expect("commit");

    assert_eq!(
        audit_rows(&db, &id).await,
        vec![(
            "userRevoke".to_owned(),
            Some(json!({
                "role": "TELLER",
                "branch": "XC-B01",
            })),
            None,
        )]
    );
    db.drop_schema().await;
}

#[tokio::test]
async fn rolling_back_the_transaction_leaves_no_audit_row() {
    let db = start().await;
    let id = a_user(&db, "ACTIVE", true, TELLER_B01).await;
    let mut tx = db.pool.begin().await.expect("begin");
    service()
        .suspend(&mut tx, &org_admin(), None, &id, &FREE)
        .await
        .expect("suspend");
    tx.rollback().await.expect("rollback");

    assert_eq!(version_of(&db, &id).await, 1);
    assert_eq!(audit_rows(&db, &id).await, vec![]);
    db.drop_schema().await;
}
