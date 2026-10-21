//! OR-7.m w2 W2-T4 (#4016): transaction rules for `UserService::disable` moved from
//! `disable_employee_in_test.rs`.

mod account_common;

use std::sync::Arc;

use account_common::{TestDb, unique};
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_auth::application::user::UserService;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_auth::domain::user::CustodyHolds;
use xc_auth::infrastructure::user::PgUserRepository;
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
async fn a_rolled_back_transaction_stores_nothing() {
    let db = start().await;
    let id = a_user(&db, "ACTIVE", true, TELLER).await;

    let mut tx = db.pool.begin().await.expect("begin");
    service()
        .disable(&mut tx, &org_admin(), None, &id, &FREE)
        .await
        .expect("disable inside the transaction");
    tx.rollback().await.expect("rollback");

    assert_eq!(row(&db, &id).await, ("ACTIVE".to_owned(), 1));
    assert_eq!(audit(&db, &id).await, vec![]);
    db.drop_schema().await;
}

#[tokio::test]
async fn the_closing_is_part_of_the_transaction_until_it_commits() {
    let db = start().await;
    let id = a_user(&db, "ACTIVE", true, TELLER).await;

    let mut tx = db.pool.begin().await.expect("begin");
    service()
        .disable(&mut tx, &org_admin(), None, &id, &FREE)
        .await
        .expect("disable inside the transaction");

    assert_eq!(row(&db, &id).await, ("ACTIVE".to_owned(), 1));
    assert_eq!(audit(&db, &id).await, vec![]);

    tx.commit().await.expect("commit");

    assert_eq!(row(&db, &id).await, ("DISABLED".to_owned(), 2));
    assert_eq!(
        audit(&db, &id).await,
        vec![("admin-1".to_owned(), "userDisable".to_owned())]
    );
    db.drop_schema().await;
}
