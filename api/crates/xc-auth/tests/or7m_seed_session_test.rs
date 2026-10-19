//! OR-7.m (#1027): `xc_testdb::sessions::seed_session` writes a session the real
//! `SessionService::session_check` accepts, so tests can sign in without password login.
//!
//! Needs a real PostgreSQL (`TEST_DATABASE_URL`, name ending `_test`); without it the tests FAIL.

use std::path::{Path, PathBuf};

use chrono::{Duration, TimeZone, Utc};
use xc_auth::application::session::SessionService;
use xc_testdb::fresh_db;
use xc_testdb::people::{RoleRow, seed_person};
use xc_testdb::sessions::{cookie, seed_session};

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn migrations() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

#[tokio::test]
async fn a_seeded_session_is_accepted_until_it_expires() {
    let db = fresh_db(&migrations(), "or7m_session")
        .await
        .expect("a test db");
    let mut conn = db.pool.acquire().await.expect("a connection");
    let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let anna = seed_person(
        &mut conn,
        ORG,
        "Anna",
        &[RoleRow {
            role_code: "EXECUTIVE",
            branch_id: None,
            valid_from: t0,
            valid_to: None,
        }],
    )
    .await
    .expect("Anna");
    let now = Utc::now();
    let token = seed_session(&mut conn, &anna, None, now + Duration::hours(1))
        .await
        .expect("a session");
    drop(conn);

    assert_eq!(cookie(&token), format!("__Host-xc_session={token}"));
    let sessions = SessionService::new(db.pool.clone(), None, None, None);
    let user = sessions
        .session_check(&token, now)
        .await
        .expect("no database error")
        .expect("the seeded session is accepted");
    assert_eq!(user.id, anna);
    assert_eq!(user.name, "Anna");
    assert!(
        sessions
            .session_check(&token, now + Duration::hours(2))
            .await
            .expect("no database error")
            .is_none(),
        "an expired session is refused"
    );
    db.drop().await.expect("drop the test db");
}

#[tokio::test]
async fn an_unknown_user_gets_no_session() {
    let db = fresh_db(&migrations(), "or7m_session")
        .await
        .expect("a test db");
    let mut conn = db.pool.acquire().await.expect("a connection");
    let got = seed_session(
        &mut conn,
        "00000000-0000-4000-8000-00000000dead",
        None,
        Utc::now() + Duration::hours(1),
    )
    .await;
    assert!(
        got.is_err(),
        "no users row ⇒ an error, never a dangling session"
    );
    drop(conn);
    db.drop().await.expect("drop the test db");
}
