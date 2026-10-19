//! #2948 up-front tests: `xc_testdb::people` writes `users` + `user_roles` rows with
//! `valid_from` / `valid_to` always explicit.
//!
//! Written by the ticket author before the code. 🔴 The implementer must not edit, delete or
//! weaken them.
//!
//! They need a real PostgreSQL, the same as `template_test.rs`: `TEST_DATABASE_URL` must name an
//! existing database whose name ends in `_test`. Without it they FAIL; none of them skips.

use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use sqlx::Row;
use xc_testdb::fresh_db;
use xc_testdb::people::{RoleRow, seed_person, seed_roles};

/// The organisation the migrations seed (the one every api test uses as `ORG`).
const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn migrations() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
}

async fn branch(conn: &mut sqlx::PgConnection, id: &str) {
    sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
        .bind(id)
        .execute(&mut *conn)
        .await
        .expect("a branch of the seeded organisation");
}

/// `(role_code, branch_id, valid_from, valid_to)` of every role of `user_id`, by role code.
async fn roles_of(
    conn: &mut sqlx::PgConnection,
    user_id: &str,
) -> Vec<(String, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>)> {
    sqlx::query(
        "SELECT role_code, branch_id, valid_from, valid_to FROM user_roles \
         WHERE user_id = $1::uuid ORDER BY role_code",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await
    .expect("read the user's roles")
    .into_iter()
    .map(|r| {
        (
            r.get("role_code"),
            r.get("branch_id"),
            r.get("valid_from"),
            r.get("valid_to"),
        )
    })
    .collect()
}

#[tokio::test]
async fn seed_person_writes_one_active_user_of_the_organisation_and_returns_its_id() {
    let db = fresh_db(&migrations(), "people_user").await.unwrap();
    let mut conn = db.pool.acquire().await.unwrap();

    let id = seed_person(&mut conn, ORG, "Somchai", &[]).await.unwrap();

    assert!(!id.is_empty(), "seed_person returns the new user's id");
    let row = sqlx::query(
        "SELECT organization_id::text AS org, name, status FROM users WHERE id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&mut *conn)
    .await
    .expect("the returned id is a users row");
    assert_eq!(row.get::<String, _>("org"), ORG);
    assert_eq!(row.get::<String, _>("name"), "Somchai");
    assert_eq!(row.get::<String, _>("status"), "ACTIVE");
    drop(conn);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn every_role_row_names_valid_from_and_valid_to_and_its_branch_as_given() {
    let db = fresh_db(&migrations(), "people_roles").await.unwrap();
    let mut conn = db.pool.acquire().await.unwrap();
    branch(&mut conn, "B2948").await;

    let id = seed_person(
        &mut conn,
        ORG,
        "Malee",
        &[
            RoleRow {
                role_code: "TELLER",
                branch_id: Some("B2948"),
                valid_from: at(2026, 1, 1),
                valid_to: None,
            },
            RoleRow {
                role_code: "BRANCH_MANAGER",
                branch_id: None,
                valid_from: at(2026, 3, 1),
                valid_to: Some(at(2026, 12, 31)),
            },
        ],
    )
    .await
    .unwrap();

    assert!(!id.is_empty(), "seed_person returns the new user's id");
    assert_eq!(
        roles_of(&mut conn, &id).await,
        vec![
            (
                "BRANCH_MANAGER".to_owned(),
                None,
                at(2026, 3, 1),
                Some(at(2026, 12, 31))
            ),
            (
                "TELLER".to_owned(),
                Some("B2948".to_owned()),
                at(2026, 1, 1),
                None
            ),
        ],
        "one row per role, each with the window and branch it was given"
    );
    drop(conn);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn valid_from_is_never_left_to_the_wall_clock() {
    let db = fresh_db(&migrations(), "people_clock").await.unwrap();
    let mut conn = db.pool.acquire().await.unwrap();

    let long_ago = at(2001, 2, 3);
    let id = seed_person(
        &mut conn,
        ORG,
        "Anan",
        &[RoleRow {
            role_code: "TELLER",
            branch_id: None,
            valid_from: long_ago,
            valid_to: None,
        }],
    )
    .await
    .unwrap();

    assert!(!id.is_empty(), "seed_person returns the new user's id");
    let rows = roles_of(&mut conn, &id).await;
    assert_eq!(rows.len(), 1, "one role row: {rows:?}");
    assert_eq!(
        rows[0].2, long_ago,
        "valid_from is the given instant, not the column's DEFAULT now()"
    );
    drop(conn);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn seed_roles_adds_role_rows_to_an_existing_user_and_no_user() {
    let db = fresh_db(&migrations(), "people_add").await.unwrap();
    let mut conn = db.pool.acquire().await.unwrap();

    let id = seed_person(&mut conn, ORG, "Kanya", &[]).await.unwrap();
    assert!(!id.is_empty(), "seed_person returns the new user's id");
    assert!(
        roles_of(&mut conn, &id).await.is_empty(),
        "no roles given, no role rows"
    );
    let users_before: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&mut *conn)
        .await
        .unwrap();

    seed_roles(
        &mut conn,
        &id,
        ORG,
        &[RoleRow {
            role_code: "FINANCE",
            branch_id: None,
            valid_from: at(2026, 10, 12),
            valid_to: Some(at(2026, 10, 13)),
        }],
    )
    .await
    .unwrap();

    assert_eq!(
        roles_of(&mut conn, &id).await,
        vec![(
            "FINANCE".to_owned(),
            None,
            at(2026, 10, 12),
            Some(at(2026, 10, 13))
        )]
    );
    let users_after: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(users_after, users_before, "seed_roles writes no users row");
    drop(conn);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn an_unknown_role_code_is_an_error_not_a_silent_skip() {
    let db = fresh_db(&migrations(), "people_bad").await.unwrap();
    let mut conn = db.pool.acquire().await.unwrap();

    let id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, 'Nok', 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(ORG)
    .fetch_one(&mut *conn)
    .await
    .unwrap();

    let res = seed_roles(
        &mut conn,
        &id,
        ORG,
        &[RoleRow {
            role_code: "NO_SUCH_ROLE",
            branch_id: None,
            valid_from: at(2026, 1, 1),
            valid_to: None,
        }],
    )
    .await;

    assert!(
        res.is_err(),
        "a role code that is not in `roles` is an error"
    );
    drop(conn);
    db.drop().await.unwrap();
}
