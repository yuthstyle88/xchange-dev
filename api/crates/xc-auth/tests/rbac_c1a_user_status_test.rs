//! RBAC C1a up-front tests (design `docs/design/blueprint/user-management-rbac.md`, PR 3083, §3.2,
//! §4 item 3; owner decision Q3 A): `SUSPENDED` is a status, and a status the code does not know
//! is an error, never read as `PENDING`.
//! Written before the implementation by the ticket author (claude/83f682). Do not edit, delete or
//! weaken; add tests in a file of your own.
//! These compile on `origin/main` and fail there by result. Needs `TEST_DATABASE_URL`.

mod account_common;

use account_common::{TestDb, unique};
use pos_error::AppError;
use xc_auth::domain::user::{UserRepository, UserStatus};
use xc_auth::infrastructure::user::PgUserRepository;

#[test]
fn suspended_is_a_status_with_its_database_text() {
    assert_eq!(
        UserStatus::parse("SUSPENDED").map(|s| s.as_str()),
        Some("SUSPENDED")
    );
}

#[test]
fn every_status_reads_back_as_the_text_it_was_stored_with() {
    for text in ["PENDING", "ACTIVE", "SUSPENDED", "DISABLED"] {
        assert_eq!(
            UserStatus::parse(text).map(|s| s.as_str()),
            Some(text),
            "{text}"
        );
        assert_eq!(
            UserStatus::parse(text).map(|s| s.to_string()),
            Some(text.to_owned()),
            "Display of {text}"
        );
    }
}

#[test]
fn an_unknown_or_differently_cased_text_is_no_status() {
    for text in [
        "",
        "suspended",
        "Suspended",
        "INVITED",
        "ON_LEAVE",
        " ACTIVE",
    ] {
        assert_eq!(UserStatus::parse(text), None, "{text:?}");
    }
}

/// A user row with `status` (the status CHECK dropped in this test's own schema), and its
/// `auth_subject`; answers the user id.
async fn a_row(db: &TestDb, status: &str, subject: &str) -> String {
    sqlx::query("ALTER TABLE users DROP CONSTRAINT IF EXISTS users_status_check")
        .execute(&db.pool)
        .await
        .expect("drop the status check");
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, auth_subject, name, status, created_by) \
         SELECT id, $1, 'Row', $2, 'test' FROM organizations WHERE code = 'XC' RETURNING id::text",
    )
    .bind(subject)
    .bind(status)
    .fetch_one(&db.pool)
    .await
    .expect("insert the user row")
}

#[tokio::test]
async fn find_by_id_refuses_a_status_it_does_not_know() {
    let db = TestDb::start().await;
    let id = a_row(&db, "ON_LEAVE", &unique("sub")).await;
    let repo = PgUserRepository::new();
    let mut tx = db.pool.begin().await.expect("begin");
    let got = repo.find_by_id(&mut tx, &id).await;
    match got {
        Err(AppError::Internal(_)) => {}
        Ok(Some(user)) => panic!("read as {:?}, not refused", user.status),
        other => panic!("not an Internal error: {other:?}"),
    }
    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}

#[tokio::test]
async fn find_by_auth_subject_refuses_a_status_it_does_not_know() {
    let db = TestDb::start().await;
    let subject = unique("sub");
    a_row(&db, "ON_LEAVE", &subject).await;
    let repo = PgUserRepository::new();
    let mut tx = db.pool.begin().await.expect("begin");
    match repo.find_by_auth_subject(&mut tx, &subject).await {
        Err(AppError::Internal(_)) => {}
        Ok(Some(user)) => panic!("read as {:?}, not refused", user.status),
        other => panic!("not an Internal error: {other:?}"),
    }
    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}

#[tokio::test]
async fn a_suspended_row_reads_as_suspended() {
    let db = TestDb::start().await;
    let subject = unique("sub");
    let id = a_row(&db, "SUSPENDED", &subject).await;
    let repo = PgUserRepository::new();
    let mut tx = db.pool.begin().await.expect("begin");
    let by_id = repo
        .find_by_id(&mut tx, &id)
        .await
        .expect("read")
        .expect("the row");
    assert_eq!(by_id.status.as_str(), "SUSPENDED");
    let by_subject = repo
        .find_by_auth_subject(&mut tx, &subject)
        .await
        .expect("read")
        .expect("the row");
    assert_eq!(by_subject.status.as_str(), "SUSPENDED");
    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}

#[tokio::test]
async fn set_status_writes_suspended_and_moves_the_version() {
    let db = TestDb::start().await;
    let id = a_row(&db, "ACTIVE", &unique("sub")).await;
    let repo = PgUserRepository::new();
    let mut tx = db.pool.begin().await.expect("begin");
    let suspended = UserStatus::parse("SUSPENDED").expect("SUSPENDED is a status");
    repo.set_status(&mut tx, &id, suspended)
        .await
        .expect("set SUSPENDED");
    let (status, version): (String, i32) =
        sqlx::query_as("SELECT status, version FROM users WHERE id = $1::uuid")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .expect("read back");
    assert_eq!((status.as_str(), version), ("SUSPENDED", 2));
    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}
