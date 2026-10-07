//! Ticket #289 — verifying that an owner's session ends when the owner's version moves.

mod account_common;

use account_common::*;
use serde_json::json;

const OWNER_PASSWORD: &str = "owner-password-01";

#[tokio::test(flavor = "multi_thread")]
async fn an_owner_session_ends_when_the_owners_version_moves() {
    let db = TestDb::start().await;
    let service = db.service();

    let owner = service
        .create_owner("boss", OWNER_PASSWORD, at(0))
        .await
        .expect("created");

    let login = service
        .login(
            &json!({ "username": "boss", "password": OWNER_PASSWORD }),
            "10.0.0.1",
            at(1),
        )
        .await
        .expect("login");

    let seen = service
        .status(&login.token, "", at(2))
        .await
        .expect("status check")
        .expect("signed in");
    assert_eq!(owner_of(&seen), &owner);

    let changed = sqlx::query("UPDATE owners SET version = version + 1 WHERE id = $1")
        .bind(&owner.id)
        .execute(&db.pool)
        .await
        .expect("update version");
    assert_eq!(changed.rows_affected(), 1);

    let after = service
        .status(&login.token, "", at(3))
        .await
        .expect("status check");
    assert!(
        after.is_none(),
        "the session ends when the owner's version moves"
    );

    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sign_in_after_the_version_moves_carries_the_new_version() {
    let db = TestDb::start().await;
    let service = db.service();

    let owner = service
        .create_owner("boss", OWNER_PASSWORD, at(0))
        .await
        .expect("created");

    let changed = sqlx::query("UPDATE owners SET version = version + 1 WHERE id = $1")
        .bind(&owner.id)
        .execute(&db.pool)
        .await
        .expect("update version");
    assert_eq!(changed.rows_affected(), 1);

    let login = service
        .login(
            &json!({ "username": "boss", "password": OWNER_PASSWORD }),
            "10.0.0.1",
            at(1),
        )
        .await
        .expect("login");

    let seen = service
        .status(&login.token, "", at(2))
        .await
        .expect("status check")
        .expect("signed in");
    let signed_in_owner = owner_of(&seen);
    assert_eq!(signed_in_owner.version, 2);
    assert_eq!(signed_in_owner.id, owner.id);
    assert_eq!(signed_in_owner.username, owner.username);

    db.drop_schema().await;
}
