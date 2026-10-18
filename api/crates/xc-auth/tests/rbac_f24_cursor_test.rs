//! RBAC f24 (#3213): `PgUserRepository::list_audit` and `list_logins` with malformed cursors.
//!
//! Behaviour table rows:
//! 1. list_audit with before = Some("not-a-uuid") -> Ok(vec![])
//! 2. list_logins with before = Some("not-a-uuid") -> Ok(vec![])
//! 3. before = a valid uuid of no row -> Ok(vec![])
//! 4. before = None and a valid existing id -> unchanged

mod account_common;

use account_common::{TestDb, at, unique};
use xc_auth::domain::user::UserRepository;
use xc_auth::infrastructure::user::PgUserRepository;

async fn org_id(db: &TestDb, code: &str) -> String {
    if code == "XC" {
        return sqlx::query_scalar("SELECT id::text FROM organizations WHERE code = 'XC'")
            .fetch_one(&db.pool)
            .await
            .expect("select XC organization");
    }
    sqlx::query_scalar(
        "INSERT INTO organizations (id, code, timezone, created_at) \
         VALUES (gen_random_uuid(), $1, 'Asia/Bangkok', now()) RETURNING id::text",
    )
    .bind(code)
    .fetch_one(&db.pool)
    .await
    .expect("insert organization")
}

async fn insert_user(db: &TestDb, org_id: &str, name: &str) -> String {
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(org_id)
    .bind(name)
    .fetch_one(&db.pool)
    .await
    .expect("insert user")
}

#[tokio::test]
async fn list_audit_with_malformed_before_returns_empty() {
    let db = TestDb::start().await;
    let org = org_id(&db, &unique("ORG")).await;
    let actor = insert_user(&db, &org, "Alice").await;
    let subject = insert_user(&db, &org, "Bob").await;

    sqlx::query(
        "INSERT INTO user_audit (actor_id, subject_id, action, at, organization_id) \
         VALUES ($1, $2, 'a1', $3, $4::uuid)",
    )
    .bind(&actor)
    .bind(&subject)
    .bind(at(10))
    .bind(&org)
    .execute(&db.pool)
    .await
    .expect("insert audit row");

    let mut tx = db.pool.begin().await.expect("begin");
    let rows = PgUserRepository
        .list_audit(&mut tx, &org, Some("not-a-uuid"), 10)
        .await
        .expect("list_audit with malformed cursor");
    assert!(rows.is_empty(), "expected empty rows, got: {:?}", rows);
    // 36 characters of the right alphabet, but not a uuid (reviewer row).
    let rows = PgUserRepository
        .list_audit(&mut tx, &org, Some(&"-".repeat(36)), 10)
        .await
        .expect("list_audit with a 36-hyphen cursor");
    assert!(rows.is_empty(), "expected empty rows, got: {:?}", rows);

    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}

#[tokio::test]
async fn list_logins_with_malformed_before_returns_empty() {
    let db = TestDb::start().await;
    let org = org_id(&db, &unique("ORG")).await;
    let user = insert_user(&db, &org, "Alice").await;

    sqlx::query(
        "INSERT INTO login_history (user_id, identifier_hash, result, at, organization_id) \
         VALUES ($1::uuid, 'h1', 'OK', $2, $3::uuid)",
    )
    .bind(&user)
    .bind(at(10))
    .bind(&org)
    .execute(&db.pool)
    .await
    .expect("insert login row");

    let mut tx = db.pool.begin().await.expect("begin");
    let rows = PgUserRepository
        .list_logins(&mut tx, &org, Some("not-a-uuid"), 10)
        .await
        .expect("list_logins with malformed cursor");
    assert!(rows.is_empty(), "expected empty rows, got: {:?}", rows);

    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}

#[tokio::test]
async fn before_valid_uuid_of_no_row_returns_empty() {
    let db = TestDb::start().await;
    let org = org_id(&db, &unique("ORG")).await;
    let actor = insert_user(&db, &org, "Alice").await;
    let subject = insert_user(&db, &org, "Bob").await;

    sqlx::query(
        "INSERT INTO user_audit (actor_id, subject_id, action, at, organization_id) \
         VALUES ($1, $2, 'a1', $3, $4::uuid)",
    )
    .bind(&actor)
    .bind(&subject)
    .bind(at(10))
    .bind(&org)
    .execute(&db.pool)
    .await
    .expect("insert audit row");

    sqlx::query(
        "INSERT INTO login_history (user_id, identifier_hash, result, at, organization_id) \
         VALUES ($1::uuid, 'h1', 'OK', $2, $3::uuid)",
    )
    .bind(&actor)
    .bind(at(10))
    .bind(&org)
    .execute(&db.pool)
    .await
    .expect("insert login row");

    let mut tx = db.pool.begin().await.expect("begin");
    let no_row_uuid = "00000000-0000-0000-0000-000000000099";

    let audit_rows = PgUserRepository
        .list_audit(&mut tx, &org, Some(no_row_uuid), 10)
        .await
        .expect("list_audit valid uuid of no row");
    assert!(audit_rows.is_empty());

    let login_rows = PgUserRepository
        .list_logins(&mut tx, &org, Some(no_row_uuid), 10)
        .await
        .expect("list_logins valid uuid of no row");
    assert!(login_rows.is_empty());

    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}

#[tokio::test]
async fn before_none_and_valid_existing_id_returns_expected_rows() {
    let db = TestDb::start().await;
    let org = org_id(&db, &unique("ORG")).await;
    let actor = insert_user(&db, &org, "Alice").await;
    let subject = insert_user(&db, &org, "Bob").await;

    let id1: String = sqlx::query_scalar(
        "INSERT INTO user_audit (actor_id, subject_id, action, at, organization_id) \
         VALUES ($1, $2, 'a1', $3, $4::uuid) RETURNING id::text",
    )
    .bind(&actor)
    .bind(&subject)
    .bind(at(10))
    .bind(&org)
    .fetch_one(&db.pool)
    .await
    .expect("insert audit 1");

    let id2: String = sqlx::query_scalar(
        "INSERT INTO user_audit (actor_id, subject_id, action, at, organization_id) \
         VALUES ($1, $2, 'a2', $3, $4::uuid) RETURNING id::text",
    )
    .bind(&actor)
    .bind(&subject)
    .bind(at(20))
    .bind(&org)
    .fetch_one(&db.pool)
    .await
    .expect("insert audit 2");

    let mut tx = db.pool.begin().await.expect("begin");

    // before = None -> returns all rows newest first
    let all = PgUserRepository
        .list_audit(&mut tx, &org, None, 10)
        .await
        .expect("list_audit before None");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, id2);
    assert_eq!(all[1].id, id1);

    // before = id2 -> returns only id1
    let older = PgUserRepository
        .list_audit(&mut tx, &org, Some(&id2), 10)
        .await
        .expect("list_audit before id2");
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].id, id1);

    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}
