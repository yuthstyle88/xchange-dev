//! `AccountService::disable_employee_in` (ticket #157): the account is closed
//! inside the caller's transaction, with its `DISABLE` audit line, so both
//! exist exactly when that transaction commits.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose
//! name ends in `_test`). Without it they FAIL; none skips.

mod account_common;

use account_common::{
    EMPLOYEE_NOT_FOUND, Service, TestDb, at, create_employee, new_employee, refusal,
};

async fn active_and_version(db: &TestDb, id: &str) -> (bool, i32) {
    sqlx::query_as("SELECT active, version FROM employees WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .expect("the employee row")
}

async fn disable_lines(db: &TestDb, id: &str) -> Vec<(String, String, String)> {
    db.audit()
        .await
        .into_iter()
        .filter(|(_, _, subject, action, _)| subject == id && action == "DISABLE")
        .map(|(_, actor, subject, action, _)| (actor, subject, action))
        .collect()
}

async fn employee(service: &Service, username: &str) -> String {
    create_employee(
        service,
        &new_employee(username, "FRONT", "MAIN", "the-first-password"),
        at(0),
    )
    .await
    .id
}

/// Closed once, with one audit line naming the actor, when the transaction commits.
#[tokio::test]
async fn a_committed_transaction_closes_the_account_once_with_its_audit_line() {
    let db = TestDb::start().await;
    let service = db.service();
    let id = employee(&service, "ann").await;
    let (_, version) = active_and_version(&db, &id).await;

    let mut tx = db.pool.begin().await.expect("begin");
    service
        .disable_employee_in(&mut tx, &id, at(60), "owner-1")
        .await
        .expect("disable inside the transaction");
    tx.commit().await.expect("commit");

    assert_eq!(active_and_version(&db, &id).await, (false, version + 1));
    assert_eq!(
        disable_lines(&db, &id).await,
        vec![("owner-1".to_owned(), id.clone(), "DISABLE".to_owned())]
    );
    db.drop_schema().await;
}

/// Nothing is stored — neither the closing nor the audit line — when the
/// transaction is rolled back.
#[tokio::test]
async fn a_rolled_back_transaction_stores_nothing() {
    let db = TestDb::start().await;
    let service = db.service();
    let id = employee(&service, "bea").await;
    let before = active_and_version(&db, &id).await;

    let mut tx = db.pool.begin().await.expect("begin");
    service
        .disable_employee_in(&mut tx, &id, at(60), "owner-1")
        .await
        .expect("disable inside the transaction");
    tx.rollback().await.expect("rollback");

    assert_eq!(active_and_version(&db, &id).await, before);
    assert!(before.0, "the employee is still active");
    assert!(disable_lines(&db, &id).await.is_empty());
    db.drop_schema().await;
}

/// The closing is seen inside the transaction before it commits, and by
/// nobody else until then.
#[tokio::test]
async fn the_closing_is_part_of_the_transaction_until_it_commits() {
    let db = TestDb::start().await;
    let service = db.service();
    let id = employee(&service, "cyd").await;

    let mut tx = db.pool.begin().await.expect("begin");
    service
        .disable_employee_in(&mut tx, &id, at(60), "owner-1")
        .await
        .expect("disable inside the transaction");
    let inside: bool = sqlx::query_scalar("SELECT active FROM employees WHERE id = $1")
        .bind(&id)
        .fetch_one(&mut *tx)
        .await
        .expect("read inside the transaction");
    assert!(!inside, "closed inside the transaction");
    assert!(
        active_and_version(&db, &id).await.0,
        "still active outside it before the commit"
    );
    tx.commit().await.expect("commit");
    assert!(!active_and_version(&db, &id).await.0);
    db.drop_schema().await;
}

/// An id that names nobody is answered as `employees("disable")` answers it,
/// and nothing is written.
#[tokio::test]
async fn an_unknown_id_is_not_found_and_writes_nothing() {
    let db = TestDb::start().await;
    let service = db.service();
    let lines = db.count("employee_audit").await;

    let mut tx = db.pool.begin().await.expect("begin");
    let error = service
        .disable_employee_in(&mut tx, "nobody", at(60), "owner-1")
        .await
        .expect_err("an unknown id is refused");
    tx.commit().await.expect("commit");

    assert_eq!(refusal(&error), EMPLOYEE_NOT_FOUND);
    let through_employees = service
        .employees(
            "disable",
            &serde_json::json!({ "id": "nobody" }),
            "owner-1",
            at(60),
        )
        .await
        .expect_err("the old path refuses it too");
    assert_eq!(refusal(&through_employees), refusal(&error));
    assert_eq!(db.count("employee_audit").await, lines);
    db.drop_schema().await;
}
