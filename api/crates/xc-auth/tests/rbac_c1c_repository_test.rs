//! RBAC C1c tests (#3145): `PgUserRepository::find_profile`, `set_profile`, `list_audit`, and
//! `list_logins`.

mod account_common;

use account_common::{TestDb, at, unique};
use pos_error::AppError;
use serde_json::json;
use xc_auth::domain::user::{LoginEntry, UserAuditEntry, UserProfile, UserRepository};
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
async fn find_profile_on_unknown_id_returns_none() {
    let db = TestDb::start().await;
    let mut tx = db.pool.begin().await.expect("begin");
    let got = PgUserRepository
        .find_profile(&mut tx, "00000000-0000-0000-0000-000000000000")
        .await
        .expect("find_profile");
    assert_eq!(got, None);
    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}

#[tokio::test]
async fn find_profile_on_new_user_returns_default() {
    let db = TestDb::start().await;
    let org = org_id(&db, "XC").await;
    let uid = insert_user(&db, &org, &unique("u")).await;
    let mut tx = db.pool.begin().await.expect("begin");
    let got = PgUserRepository
        .find_profile(&mut tx, &uid)
        .await
        .expect("find_profile");
    assert_eq!(got, Some(UserProfile::default()));
    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}

#[tokio::test]
async fn set_profile_then_find_profile_roundtrips() {
    let db = TestDb::start().await;
    let org = org_id(&db, "XC").await;
    let uid = insert_user(&db, &org, &unique("u")).await;
    let profile = UserProfile {
        employee_code: Some("EMP-001".to_owned()),
        department: Some("Operations".to_owned()),
        position: Some("Senior Teller".to_owned()),
        tell_code: Some("T01".to_owned()),
    };
    let mut tx = db.pool.begin().await.expect("begin");
    PgUserRepository
        .set_profile(&mut tx, &uid, &profile)
        .await
        .expect("set_profile");
    let got = PgUserRepository
        .find_profile(&mut tx, &uid)
        .await
        .expect("find_profile");
    assert_eq!(got, Some(profile));
    tx.commit().await.expect("commit");
    db.drop_schema().await;
}

#[tokio::test]
async fn set_profile_blank_strings_stored_as_null() {
    let db = TestDb::start().await;
    let org = org_id(&db, "XC").await;
    let uid = insert_user(&db, &org, &unique("u")).await;
    let profile = UserProfile {
        employee_code: Some("".to_owned()),
        department: Some("   ".to_owned()),
        position: Some("\t\n ".to_owned()),
        tell_code: Some("  T02  ".to_owned()),
    };
    let mut tx = db.pool.begin().await.expect("begin");
    PgUserRepository
        .set_profile(&mut tx, &uid, &profile)
        .await
        .expect("set_profile");
    let got = PgUserRepository
        .find_profile(&mut tx, &uid)
        .await
        .expect("find_profile");
    assert_eq!(
        got,
        Some(UserProfile {
            employee_code: None,
            department: None,
            position: None,
            tell_code: Some("T02".to_owned()),
        })
    );
    let raw: (Option<String>, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT employee_code, department, position, tell_code FROM users WHERE id = $1::uuid",
    )
    .bind(&uid)
    .fetch_one(&mut *tx)
    .await
    .expect("raw row");
    assert_eq!(raw, (None, None, None, Some("T02".to_owned())));
    tx.commit().await.expect("commit");
    db.drop_schema().await;
}

#[tokio::test]
async fn set_profile_duplicate_code_case_insensitive_same_org_conflicts() {
    let db = TestDb::start().await;
    let org = org_id(&db, "XC").await;
    let user_a = insert_user(&db, &org, &unique("a")).await;
    let user_b = insert_user(&db, &org, &unique("b")).await;

    let mut tx = db.pool.begin().await.expect("begin");
    PgUserRepository
        .set_profile(
            &mut tx,
            &user_a,
            &UserProfile {
                employee_code: Some("EMP1".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("set user A");
    let err = PgUserRepository
        .set_profile(
            &mut tx,
            &user_b,
            &UserProfile {
                employee_code: Some("emp1".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect_err("duplicate code in same org must fail");
    assert!(
        matches!(&err, AppError::Conflict(msg) if msg == "employeeCodeTaken"),
        "expected Conflict(\"employeeCodeTaken\"), got {err:?}"
    );
    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}

#[tokio::test]
async fn set_profile_same_code_in_two_organisations_succeeds() {
    let db = TestDb::start().await;
    let org_a = org_id(&db, "XC").await;
    let org_b = org_id(&db, &unique("ORG")).await;
    let user_a = insert_user(&db, &org_a, &unique("a")).await;
    let user_b = insert_user(&db, &org_b, &unique("b")).await;

    let profile = UserProfile {
        employee_code: Some("EMP1".to_owned()),
        ..Default::default()
    };
    let mut tx = db.pool.begin().await.expect("begin");
    PgUserRepository
        .set_profile(&mut tx, &user_a, &profile)
        .await
        .expect("set user A in org A");
    PgUserRepository
        .set_profile(&mut tx, &user_b, &profile)
        .await
        .expect("set user B in org B");
    tx.commit().await.expect("commit");
    db.drop_schema().await;
}

#[tokio::test]
async fn set_profile_with_own_existing_code_succeeds() {
    let db = TestDb::start().await;
    let org = org_id(&db, "XC").await;
    let uid = insert_user(&db, &org, &unique("u")).await;

    let mut tx = db.pool.begin().await.expect("begin");
    PgUserRepository
        .set_profile(
            &mut tx,
            &uid,
            &UserProfile {
                employee_code: Some("EMP1".to_owned()),
                department: Some("D1".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("first set_profile");
    PgUserRepository
        .set_profile(
            &mut tx,
            &uid,
            &UserProfile {
                employee_code: Some("emp1".to_owned()),
                department: Some("D2".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("second set_profile with own code");
    tx.commit().await.expect("commit");
    db.drop_schema().await;
}

#[tokio::test]
async fn list_audit_three_rows_newest_first_joins_names_and_maps_fields() {
    let db = TestDb::start().await;
    let org = org_id(&db, &unique("ORG")).await;
    let actor = insert_user(&db, &org, "Alice Admin").await;
    let subject = insert_user(&db, &org, "Bob Teller").await;

    let id1: String = sqlx::query_scalar(
        "INSERT INTO user_audit (actor_id, subject_id, action, at, organization_id, before, after, ip) \
         VALUES ($1, $2, 'userCreate', $3, $4::uuid, NULL, $5::text::jsonb, '10.0.0.1'::inet) RETURNING id::text",
    )
    .bind(&actor)
    .bind(&subject)
    .bind(at(10))
    .bind(&org)
    .bind(json!({"status": "ACTIVE"}).to_string())
    .fetch_one(&db.pool)
    .await
    .expect("insert audit 1");

    let id2: String = sqlx::query_scalar(
        "INSERT INTO user_audit (actor_id, subject_id, action, at, organization_id, before, after, ip) \
         VALUES ('system', $1, 'userProfileSave', $2, $3::uuid, $4::text::jsonb, $5::text::jsonb, '192.168.1.5'::inet) RETURNING id::text",
    )
    .bind(&subject)
    .bind(at(20))
    .bind(&org)
    .bind(json!({"employee_code": null}).to_string())
    .bind(json!({"employee_code": "E1"}).to_string())
    .fetch_one(&db.pool)
    .await
    .expect("insert audit 2");

    let id3: String = sqlx::query_scalar(
        "INSERT INTO user_audit (actor_id, subject_id, action, at, organization_id) \
         VALUES ($1, 'non-user-subj', 'userDisable', $2, $3::uuid) RETURNING id::text",
    )
    .bind(&actor)
    .bind(at(30))
    .bind(&org)
    .fetch_one(&db.pool)
    .await
    .expect("insert audit 3");

    let mut tx = db.pool.begin().await.expect("begin");
    let rows = PgUserRepository
        .list_audit(&mut tx, &org, None, 10)
        .await
        .expect("list_audit");
    assert_eq!(
        rows,
        vec![
            UserAuditEntry {
                id: id3,
                at: at(30),
                actor_id: actor.clone(),
                actor_name: "Alice Admin".to_owned(),
                subject_id: "non-user-subj".to_owned(),
                subject_name: "non-user-subj".to_owned(),
                action: "userDisable".to_owned(),
                before: None,
                after: None,
                ip: None,
            },
            UserAuditEntry {
                id: id2,
                at: at(20),
                actor_id: "system".to_owned(),
                actor_name: "system".to_owned(),
                subject_id: subject.clone(),
                subject_name: "Bob Teller".to_owned(),
                action: "userProfileSave".to_owned(),
                before: Some(json!({"employee_code": null})),
                after: Some(json!({"employee_code": "E1"})),
                ip: Some("192.168.1.5".to_owned()),
            },
            UserAuditEntry {
                id: id1,
                at: at(10),
                actor_id: actor,
                actor_name: "Alice Admin".to_owned(),
                subject_id: subject,
                subject_name: "Bob Teller".to_owned(),
                action: "userCreate".to_owned(),
                before: None,
                after: Some(json!({"status": "ACTIVE"})),
                ip: Some("10.0.0.1".to_owned()),
            },
        ]
    );
    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}

#[tokio::test]
async fn list_audit_with_before_returns_only_older_rows() {
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
    .expect("row 1");

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
    .expect("row 2");

    let _id3: String = sqlx::query_scalar(
        "INSERT INTO user_audit (actor_id, subject_id, action, at, organization_id) \
         VALUES ($1, $2, 'a3', $3, $4::uuid) RETURNING id::text",
    )
    .bind(&actor)
    .bind(&subject)
    .bind(at(30))
    .bind(&org)
    .fetch_one(&db.pool)
    .await
    .expect("row 3");

    let mut tx = db.pool.begin().await.expect("begin");
    let rows = PgUserRepository
        .list_audit(&mut tx, &org, Some(&id2), 10)
        .await
        .expect("list_audit before 2nd row");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id1);

    let unknown = PgUserRepository
        .list_audit(
            &mut tx,
            &org,
            Some("00000000-0000-0000-0000-000000000000"),
            10,
        )
        .await
        .expect("list_audit unknown before");
    assert!(unknown.is_empty());
    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}

#[tokio::test]
async fn list_audit_excludes_other_organisation() {
    let db = TestDb::start().await;
    let org_a = org_id(&db, &unique("ORGA")).await;
    let org_b = org_id(&db, &unique("ORGB")).await;

    sqlx::query(
        "INSERT INTO user_audit (actor_id, subject_id, action, at, organization_id) \
         VALUES ('sys', 'sub', 'userCreate', $1, $2::uuid)",
    )
    .bind(at(10))
    .bind(&org_b)
    .execute(&db.pool)
    .await
    .expect("insert org B audit");

    let mut tx = db.pool.begin().await.expect("begin");
    let rows = PgUserRepository
        .list_audit(&mut tx, &org_a, None, 10)
        .await
        .expect("list_audit org A");
    assert!(rows.is_empty());
    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}

#[tokio::test]
async fn list_logins_with_null_user_id_returns_none_user_fields() {
    let db = TestDb::start().await;
    let org = org_id(&db, &unique("ORG")).await;

    let id: String = sqlx::query_scalar(
        "INSERT INTO login_history (organization_id, user_id, identifier_hash, ip, result, at) \
         VALUES ($1::uuid, NULL, 'hash-1', '203.0.113.9'::inet, 'UNKNOWN_USER', $2) RETURNING id::text",
    )
    .bind(&org)
    .bind(at(15))
    .fetch_one(&db.pool)
    .await
    .expect("insert login");

    let mut tx = db.pool.begin().await.expect("begin");
    let rows = PgUserRepository
        .list_logins(&mut tx, &org, None, 10)
        .await
        .expect("list_logins");
    assert_eq!(
        rows,
        vec![LoginEntry {
            id,
            at: at(15),
            user_id: None,
            user_name: None,
            ip: Some("203.0.113.9".to_owned()),
            result: "UNKNOWN_USER".to_owned(),
        }]
    );
    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}

#[tokio::test]
async fn list_logins_paging_and_organisation_filter() {
    let db = TestDb::start().await;
    let org_a = org_id(&db, &unique("ORGA")).await;
    let org_b = org_id(&db, &unique("ORGB")).await;
    let user = insert_user(&db, &org_a, "Charlie").await;

    let id1: String = sqlx::query_scalar(
        "INSERT INTO login_history (organization_id, user_id, identifier_hash, ip, result, at) \
         VALUES ($1::uuid, $2::uuid, 'h1', '10.0.0.1'::inet, 'OK', $3) RETURNING id::text",
    )
    .bind(&org_a)
    .bind(&user)
    .bind(at(10))
    .fetch_one(&db.pool)
    .await
    .expect("login 1");

    let id2: String = sqlx::query_scalar(
        "INSERT INTO login_history (organization_id, user_id, identifier_hash, ip, result, at) \
         VALUES ($1::uuid, $2::uuid, 'h1', '10.0.0.2'::inet, 'BAD_PASSWORD', $3) RETURNING id::text",
    )
    .bind(&org_a)
    .bind(&user)
    .bind(at(20))
    .fetch_one(&db.pool)
    .await
    .expect("login 2");

    let id3: String = sqlx::query_scalar(
        "INSERT INTO login_history (organization_id, user_id, identifier_hash, ip, result, at) \
         VALUES ($1::uuid, $2::uuid, 'h1', NULL, 'OK', $3) RETURNING id::text",
    )
    .bind(&org_a)
    .bind(&user)
    .bind(at(30))
    .fetch_one(&db.pool)
    .await
    .expect("login 3");

    sqlx::query(
        "INSERT INTO login_history (organization_id, user_id, identifier_hash, result, at) \
         VALUES ($1::uuid, NULL, 'h2', 'UNKNOWN_USER', $2)",
    )
    .bind(&org_b)
    .bind(at(40))
    .execute(&db.pool)
    .await
    .expect("other org login");

    let mut tx = db.pool.begin().await.expect("begin");
    let all = PgUserRepository
        .list_logins(&mut tx, &org_a, None, 10)
        .await
        .expect("list all org_a");
    assert_eq!(
        all,
        vec![
            LoginEntry {
                id: id3,
                at: at(30),
                user_id: Some(user.clone()),
                user_name: Some("Charlie".to_owned()),
                ip: None,
                result: "OK".to_owned(),
            },
            LoginEntry {
                id: id2.clone(),
                at: at(20),
                user_id: Some(user.clone()),
                user_name: Some("Charlie".to_owned()),
                ip: Some("10.0.0.2".to_owned()),
                result: "BAD_PASSWORD".to_owned(),
            },
            LoginEntry {
                id: id1.clone(),
                at: at(10),
                user_id: Some(user.clone()),
                user_name: Some("Charlie".to_owned()),
                ip: Some("10.0.0.1".to_owned()),
                result: "OK".to_owned(),
            },
        ]
    );

    let older = PgUserRepository
        .list_logins(&mut tx, &org_a, Some(&id2), 10)
        .await
        .expect("list before 2nd row");
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].id, id1);

    let limited = PgUserRepository
        .list_logins(&mut tx, &org_a, None, 2)
        .await
        .expect("list limit 2");
    assert_eq!(limited.len(), 2);

    let unknown = PgUserRepository
        .list_logins(
            &mut tx,
            &org_a,
            Some("00000000-0000-0000-0000-000000000000"),
            10,
        )
        .await
        .expect("list unknown before");
    assert!(unknown.is_empty());

    tx.rollback().await.expect("rollback");
    db.drop_schema().await;
}
