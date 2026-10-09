//! Tests for the first admin invite (§8 of #752 / OR-3).
//!
//! Covers the four states in §8:
//! 1. No owner user -> creates one (`INVITED`, org-wide EXECUTIVE + ADMIN, `legacy_employee_id NULL`),
//!    generates invite valid 7 days.
//! 2. One owner user, `INVITED` -> replaces its invite with a new code (old code stops working).
//! 3. That owner user is `ACTIVE` -> refused, nothing changed.
//! 4. More than one owner user -> refused, nothing changed.

mod account_common;

use account_common::TestDb;
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use xc_auth::application::session::SessionService;

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[tokio::test]
async fn row1_no_owner_user_creates_invited_owner_with_roles_and_invite() {
    let db = TestDb::start().await;
    let service = SessionService::new(db.pool.clone(), None, None, None);
    let now = Utc::now();

    let code = service
        .create_first_admin_invite("Initial Owner", now)
        .await
        .expect("generates invite code");

    assert!(!code.is_empty());

    let owners: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id::text, name, status, legacy_employee_id FROM users WHERE legacy_employee_id IS NULL",
    )
    .fetch_all(&db.pool)
    .await
    .expect("fetch users");

    assert_eq!(owners.len(), 1);
    let (user_id, name, status, legacy_emp) = &owners[0];
    assert_eq!(name, "Initial Owner");
    assert_eq!(status, "INVITED");
    assert!(legacy_emp.is_none());

    let roles: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT role_code::text, branch_id::text FROM user_roles WHERE user_id::text = $1 ORDER BY role_code",
    )
    .bind(user_id)
    .fetch_all(&db.pool)
    .await
    .expect("fetch roles");

    assert_eq!(
        roles,
        vec![("ADMIN".to_string(), None), ("EXECUTIVE".to_string(), None),]
    );

    let invites: Vec<(String, String, Option<chrono::DateTime<Utc>>)> = sqlx::query_as(
        "SELECT user_id::text, code_hash, used_at FROM user_invites WHERE user_id::text = $1",
    )
    .bind(user_id)
    .fetch_all(&db.pool)
    .await
    .expect("fetch invite");

    assert_eq!(invites.len(), 1);
    assert_eq!(invites[0].1, sha256_hex(&code));
    assert!(invites[0].2.is_none());

    db.drop_schema().await;
}

#[tokio::test]
async fn row2_one_invited_owner_replaces_invite() {
    let db = TestDb::start().await;
    let service = SessionService::new(db.pool.clone(), None, None, None);
    let now = Utc::now();

    let code1 = service
        .create_first_admin_invite("Initial Owner", now)
        .await
        .expect("generates first code");

    let now2 = now + Duration::hours(1);
    let code2 = service
        .create_first_admin_invite("Initial Owner", now2)
        .await
        .expect("replaces invite code");

    assert_ne!(code1, code2);

    let users_count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&db.pool)
        .await
        .expect("count users");
    assert_eq!(users_count, 1, "no second user row created");

    let invite: (String, chrono::DateTime<Utc>, Option<chrono::DateTime<Utc>>) =
        sqlx::query_as("SELECT code_hash, expires_at, used_at FROM user_invites")
            .fetch_one(&db.pool)
            .await
            .expect("fetch invite");

    assert_eq!(invite.0, sha256_hex(&code2));
    assert!(invite.1 > now2);
    assert!(invite.2.is_none());

    db.drop_schema().await;
}

#[tokio::test]
async fn row3_active_owner_refuses_invite_creation() {
    let db = TestDb::start().await;
    let service = SessionService::new(db.pool.clone(), None, None, None);
    let now = Utc::now();

    let code1 = service
        .create_first_admin_invite("Active Owner", now)
        .await
        .expect("generates first code");

    // Mark user active
    sqlx::query("UPDATE users SET status = 'ACTIVE', auth_subject = 'sub-active-owner'")
        .execute(&db.pool)
        .await
        .expect("update status");

    let err = service
        .create_first_admin_invite("Active Owner", now + Duration::hours(1))
        .await
        .expect_err("refused because user is active");

    assert!(err.contains("ACTIVE"));

    // Verify existing invite code is untouched
    let invite_hash: String = sqlx::query_scalar("SELECT code_hash FROM user_invites")
        .fetch_one(&db.pool)
        .await
        .expect("fetch invite");
    assert_eq!(invite_hash, sha256_hex(&code1));

    db.drop_schema().await;
}

#[tokio::test]
async fn row4_multiple_owners_refuses_invite_creation() {
    let db = TestDb::start().await;
    let service = SessionService::new(db.pool.clone(), None, None, None);
    let now = Utc::now();

    // Create first owner
    let _ = service
        .create_first_admin_invite("Owner 1", now)
        .await
        .expect("first owner");

    // Insert second owner manually
    let org_id: String =
        sqlx::query_scalar("SELECT id::text FROM organizations WHERE code = 'XC' LIMIT 1")
            .fetch_one(&db.pool)
            .await
            .expect("fetch org");

    let user_id2: String = sqlx::query_scalar(
        "INSERT INTO users (id, organization_id, auth_subject, name, status, version, created_at, created_by, legacy_employee_id) \
         VALUES (gen_random_uuid(), $1::uuid, NULL, 'Owner 2', 'INVITED', 1, $2, 'test', NULL) \
         RETURNING id::text",
    )
    .bind(&org_id)
    .bind(now)
    .fetch_one(&db.pool)
    .await
    .expect("insert second user");

    sqlx::query(
        "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
         VALUES ($1::uuid, 'EXECUTIVE', $2::uuid, NULL, $3), ($1::uuid, 'ADMIN', $2::uuid, NULL, $3)",
    )
    .bind(&user_id2)
    .bind(&org_id)
    .bind(now)
    .execute(&db.pool)
    .await
    .expect("insert second user roles");

    let err = service
        .create_first_admin_invite("Owner", now + Duration::hours(1))
        .await
        .expect_err("refused because multiple owners exist");

    assert!(err.contains("More than one owner") || err.contains("owner"));

    db.drop_schema().await;
}
